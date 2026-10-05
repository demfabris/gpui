use super::{FontRun, LineLayout, ShapedGlyph, ShapedRun};
use crate::{FontId, GlyphId, Pixels, PlatformTextSystem, point, px};
use collections::FxHashMap;

const CHARACTERS: &[u8] = b"0123456789.,+-%$KMBT";
const MAX_LEN: usize = 64;
const MAX_FONTS: usize = 1024;
const CHECKED_FIRST: u32 = 32;
const CHECKED_EVERY: u64 = 256;
const TOLERANCE: f32 = 1e-3;

const fn slots() -> [u8; 128] {
    let mut slots = [u8::MAX; 128];
    let mut i = 0;
    while i < CHARACTERS.len() {
        slots[CHARACTERS[i] as usize] = i as u8;
        i += 1;
    }
    slots
}
const SLOTS: [u8; 128] = slots();
const COUNT: usize = CHARACTERS.len();

#[derive(Clone, Copy)]
struct Glyph {
    id: GlyphId,
    advance: f32,
}

struct Font {
    glyphs: [Option<Option<Glyph>>; COUNT],
    kerning: Box<[Option<Option<f32>>; COUNT * COUNT]>,
    ascent: Pixels,
    descent: Pixels,
    is_emoji: bool,
    checked: u32,
    lines: u64,
    disagreed: bool,
}

impl Font {
    fn new() -> Self {
        Font {
            glyphs: [None; COUNT],
            kerning: Box::new([None; COUNT * COUNT]),
            ascent: px(0.),
            descent: px(0.),
            is_emoji: false,
            checked: 0,
            lines: 0,
            disagreed: false,
        }
    }
}

#[derive(Default)]
pub(super) struct NumberShaping {
    fonts: FxHashMap<(FontId, u32), Font>,
    font_generation: usize,
}

impl NumberShaping {
    pub(super) fn shape(
        &mut self,
        platform: &dyn PlatformTextSystem,
        font_generation: usize,
        text: &str,
        font_size: Pixels,
        runs: &[FontRun],
    ) -> Option<LineLayout> {
        let [run] = runs else {
            return None;
        };
        let bytes = text.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_LEN
            || run.len != bytes.len()
            || !bytes
                .iter()
                .all(|&byte| byte < 128 && SLOTS[byte as usize] != u8::MAX)
        {
            return None;
        }

        let font_id = run.font_id;
        let key = (font_id, font_size.0.to_bits());
        if font_generation != self.font_generation
            || (self.fonts.len() >= MAX_FONTS && !self.fonts.contains_key(&key))
        {
            self.fonts.clear();
            self.font_generation = font_generation;
        }
        let font = self.fonts.entry(key).or_insert_with(Font::new);
        if font.disagreed {
            return None;
        }

        let layout = put_together(font, platform, bytes, font_id, font_size)?;
        font.lines += 1;
        if font.checked < CHECKED_FIRST || font.lines.is_multiple_of(CHECKED_EVERY) {
            let shaped = platform.layout_line(text, font_size, runs);
            if !agree(&layout, &shaped) {
                log::debug!(
                    "numbers in font {font_id:?} at {font_size:?} are left to the platform: \
                     it shapes {text:?} differently"
                );
                font.disagreed = true;
                return Some(shaped);
            }
            font.checked = font.checked.saturating_add(1);
        }
        Some(layout)
    }
}

fn put_together(
    font: &mut Font,
    platform: &dyn PlatformTextSystem,
    bytes: &[u8],
    font_id: FontId,
    font_size: Pixels,
) -> Option<LineLayout> {
    let mut glyphs = Vec::with_capacity(bytes.len());
    let mut x = 0f32;
    let mut previous: Option<(usize, Glyph)> = None;
    for (index, &byte) in bytes.iter().enumerate() {
        let slot = SLOTS[byte as usize] as usize;
        let glyph = glyph(font, platform, slot, font_id, font_size)?;
        if let Some((previous_slot, previous_glyph)) = previous {
            let kerning = kerning(font, platform, previous_slot, slot, font_id, font_size)?;
            x += previous_glyph.advance + kerning;
        }
        glyphs.push(ShapedGlyph {
            id: glyph.id,
            position: point(px(x), px(0.)),
            index,
            is_emoji: font.is_emoji,
        });
        previous = Some((slot, glyph));
    }
    let (_, last) = previous?;
    Some(LineLayout {
        font_size,
        width: px(x + last.advance),
        ascent: font.ascent,
        descent: font.descent,
        runs: vec![ShapedRun { font_id, glyphs }],
        len: bytes.len(),
    })
}

fn glyph(
    font: &mut Font,
    platform: &dyn PlatformTextSystem,
    slot: usize,
    font_id: FontId,
    font_size: Pixels,
) -> Option<Glyph> {
    if let Some(glyph) = font.glyphs[slot] {
        return glyph;
    }
    let text = &CHARACTERS[slot..slot + 1];
    let layout = platform.layout_line(
        std::str::from_utf8(text).ok()?,
        font_size,
        &[FontRun { len: 1, font_id }],
    );
    let glyph = match layout.runs.as_slice() {
        [run] if run.font_id == font_id => match run.glyphs.as_slice() {
            [shaped] if shaped.position == point(px(0.), px(0.)) && shaped.index == 0 => {
                font.ascent = layout.ascent;
                font.descent = layout.descent;
                font.is_emoji = shaped.is_emoji;
                Some(Glyph {
                    id: shaped.id,
                    advance: layout.width.0,
                })
            }
            _ => None,
        },
        _ => None,
    };
    font.glyphs[slot] = Some(glyph);
    glyph
}

fn kerning(
    font: &mut Font,
    platform: &dyn PlatformTextSystem,
    left: usize,
    right: usize,
    font_id: FontId,
    font_size: Pixels,
) -> Option<f32> {
    let pair = left * COUNT + right;
    if let Some(kerning) = font.kerning[pair] {
        return kerning;
    }
    let left_glyph = glyph(font, platform, left, font_id, font_size)?;
    let right_glyph = glyph(font, platform, right, font_id, font_size)?;
    let text = [CHARACTERS[left], CHARACTERS[right]];
    let layout = platform.layout_line(
        std::str::from_utf8(&text).ok()?,
        font_size,
        &[FontRun { len: 2, font_id }],
    );
    let kerning = match layout.runs.as_slice() {
        [run] if run.font_id == font_id => match run.glyphs.as_slice() {
            [first, second]
                if first.id == left_glyph.id
                    && second.id == right_glyph.id
                    && first.index == 0
                    && second.index == 1
                    && first.position == point(px(0.), px(0.))
                    && second.position.y == px(0.) =>
            {
                let kerning = second.position.x.0 - left_glyph.advance;
                let width = second.position.x.0 + right_glyph.advance;
                ((layout.width.0 - width).abs() <= TOLERANCE).then_some(kerning)
            }
            _ => None,
        },
        _ => None,
    };
    font.kerning[pair] = Some(kerning);
    kerning
}

fn agree(ours: &LineLayout, theirs: &LineLayout) -> bool {
    let close = |a: Pixels, b: Pixels| (a.0 - b.0).abs() <= TOLERANCE;
    ours.len == theirs.len
        && ours.font_size == theirs.font_size
        && close(ours.width, theirs.width)
        && ours.ascent == theirs.ascent
        && ours.descent == theirs.descent
        && ours.runs.len() == theirs.runs.len()
        && ours.runs.iter().zip(&theirs.runs).all(|(ours, theirs)| {
            ours.font_id == theirs.font_id
                && ours.glyphs.len() == theirs.glyphs.len()
                && ours
                    .glyphs
                    .iter()
                    .zip(&theirs.glyphs)
                    .all(|(ours, theirs)| {
                        ours.id == theirs.id
                            && ours.index == theirs.index
                            && ours.is_emoji == theirs.is_emoji
                            && close(ours.position.x, theirs.position.x)
                            && close(ours.position.y, theirs.position.y)
                    })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Bounds, DevicePixels, FontMetrics, NoopTextSystem, RenderGlyphParams, Result, Size,
        TextRenderingMode,
    };
    use std::{
        borrow::Cow,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    const FONT: FontId = FontId(3);

    #[derive(Default)]
    struct Shaper {
        lines: AtomicUsize,
        in_context: AtomicBool,
    }

    impl Shaper {
        fn advance(ch: char) -> f64 {
            match ch {
                '0'..='9' => 7.25,
                '.' | ',' => 3.5,
                _ => 6.125,
            }
        }

        fn kerning(left: char, right: char) -> f64 {
            match (left, right) {
                ('7', '4') => -0.75,
                ('1', '1') => -0.5,
                ('T', '.') => -1.25,
                _ => 0.,
            }
        }

        fn lines(&self) -> usize {
            self.lines.load(Ordering::Relaxed)
        }
    }

    impl PlatformTextSystem for Shaper {
        fn add_fonts(&self, _: Vec<Cow<'static, [u8]>>) -> Result<()> {
            Ok(())
        }

        fn all_font_names(&self) -> Vec<String> {
            Vec::new()
        }

        fn font_id(&self, _: &crate::Font) -> Result<FontId> {
            Ok(FONT)
        }

        fn font_metrics(&self, font_id: FontId) -> FontMetrics {
            NoopTextSystem.font_metrics(font_id)
        }

        fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
            NoopTextSystem.typographic_bounds(font_id, glyph_id)
        }

        fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
            NoopTextSystem.advance(font_id, glyph_id)
        }

        fn glyph_for_char(&self, _: FontId, ch: char) -> Option<GlyphId> {
            Some(GlyphId(ch as u32))
        }

        fn glyph_raster_bounds(&self, _: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
            Ok(Default::default())
        }

        fn rasterize_glyph(
            &self,
            _: &RenderGlyphParams,
            raster_bounds: Bounds<DevicePixels>,
        ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
            Ok((raster_bounds.size, Vec::new()))
        }

        fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
            self.lines.fetch_add(1, Ordering::Relaxed);
            let chars: Vec<char> = text.chars().collect();
            let in_context = self.in_context.load(Ordering::Relaxed);
            let mut glyphs = Vec::new();
            let mut x = 0f64;
            let mut ix = 0;
            while ix < chars.len() {
                let ch = chars[ix];
                if ch == '%' && chars.get(ix + 1) == Some(&'%') {
                    glyphs.push(glyph(1000, x, ix));
                    x += 9.;
                    ix += 2;
                    continue;
                }
                let between_digits = ix > 0
                    && chars[ix - 1].is_ascii_digit()
                    && chars.get(ix + 1).is_some_and(char::is_ascii_digit);
                let id = if in_context && ch == '-' && between_digits {
                    2000
                } else {
                    ch as u32
                };
                glyphs.push(glyph(id, x, ix));
                x += Self::advance(ch);
                if let Some(&next) = chars.get(ix + 1) {
                    x += Self::kerning(ch, next);
                }
                ix += 1;
            }
            LineLayout {
                font_size,
                width: px(x as f32),
                ascent: px(9.),
                descent: px(3.),
                runs: vec![ShapedRun {
                    font_id: runs[0].font_id,
                    glyphs,
                }],
                len: text.len(),
            }
        }

        fn recommended_rendering_mode(&self, _: FontId, _: Pixels) -> TextRenderingMode {
            TextRenderingMode::Grayscale
        }
    }

    fn glyph(id: u32, x: f64, index: usize) -> ShapedGlyph {
        ShapedGlyph {
            id: GlyphId(id),
            position: point(px(x as f32), px(0.)),
            index,
            is_emoji: false,
        }
    }

    fn run(text: &str) -> [FontRun; 1] {
        [FontRun {
            len: text.len(),
            font_id: FONT,
        }]
    }

    fn assert_shaped_alike(ours: &LineLayout, theirs: &LineLayout, text: &str) {
        let close = |a: Pixels, b: Pixels| (a.0 - b.0).abs() <= 1e-3;
        assert!(close(ours.width, theirs.width), "{text}: width");
        assert_eq!((ours.ascent, ours.descent), (theirs.ascent, theirs.descent));
        assert_eq!(ours.len, theirs.len);
        let [ours] = ours.runs.as_slice() else {
            panic!("{text}: one run");
        };
        let [theirs] = theirs.runs.as_slice() else {
            panic!("{text}: one run");
        };
        assert_eq!(ours.font_id, theirs.font_id);
        assert_eq!(ours.glyphs.len(), theirs.glyphs.len(), "{text}: glyphs");
        for (ours, theirs) in ours.glyphs.iter().zip(&theirs.glyphs) {
            assert_eq!((ours.id, ours.index), (theirs.id, theirs.index), "{text}");
            assert!(
                close(ours.position.x, theirs.position.x),
                "{text}: position"
            );
        }
    }

    #[test]
    fn numbers_are_put_together_as_the_platform_shapes_them() {
        let shaper = Shaper::default();
        let mut shaping = NumberShaping::default();
        let size = px(13.);
        let numbers: Vec<String> = (0..CHECKED_FIRST as u64 + 40)
            .map(|n| match n % 4 {
                0 => format!("{:.2}", 7400.0 + n as f64 * 1.11),
                1 => format!("{:+.2}%", n as f64 * 0.7 - 11.0),
                2 => format!("{:.2}T", 1.0 + n as f64),
                _ => format!("${}", 11_747 + n),
            })
            .collect();

        for text in &numbers {
            let ours = shaping.shape(&shaper, 0, text, size, &run(text)).unwrap();
            let theirs = shaper.layout_line(text, size, &run(text));
            assert_shaped_alike(&ours, &theirs, text);
        }

        let before = shaper.lines();
        let ours = shaping.shape(&shaper, 0, "7411.47", size, &run("7411.47"));
        assert_eq!(shaper.lines(), before, "a checked font needs no platform");
        assert_shaped_alike(
            &ours.unwrap(),
            &shaper.layout_line("7411.47", size, &run("7411.47")),
            "7411.47",
        );
    }

    #[test]
    fn a_pair_the_platform_joins_is_left_to_it() {
        let shaper = Shaper::default();
        let mut shaping = NumberShaping::default();
        assert!(
            shaping
                .shape(&shaper, 0, "5%%", px(13.), &run("5%%"))
                .is_none()
        );
        assert!(
            shaping
                .shape(&shaper, 0, "5%", px(13.), &run("5%"))
                .is_some()
        );
    }

    #[test]
    fn a_font_shaping_in_context_is_left_to_the_platform() {
        let shaper = Shaper::default();
        shaper.in_context.store(true, Ordering::Relaxed);
        let mut shaping = NumberShaping::default();
        let size = px(13.);

        let shaped = shaping.shape(&shaper, 0, "1-2", size, &run("1-2")).unwrap();
        assert_eq!(shaped.runs[0].glyphs[1].id, GlyphId(2000));
        assert!(shaping.shape(&shaper, 0, "12", size, &run("12")).is_none());
        assert!(
            shaping
                .shape(&shaper, 0, "12", px(14.), &run("12"))
                .is_some()
        );
    }

    #[test]
    fn lines_other_than_numbers_are_left_to_the_platform() {
        let shaper = Shaper::default();
        let mut shaping = NumberShaping::default();
        let size = px(13.);
        for text in ["", "12:30", "abc", "1 234", "½", &"1".repeat(65)] {
            assert!(
                shaping.shape(&shaper, 0, text, size, &run(text)).is_none(),
                "{text:?}"
            );
        }
        let two_runs = [
            FontRun {
                len: 1,
                font_id: FONT,
            },
            FontRun {
                len: 1,
                font_id: FONT,
            },
        ];
        assert!(shaping.shape(&shaper, 0, "12", size, &two_runs).is_none());
    }

    fn window_text_system() -> (crate::WindowTextSystem, Arc<Shaper>, crate::Font) {
        let shaper = Arc::new(Shaper::default());
        let text_system = Arc::new(crate::TextSystem::new(shaper.clone()));
        (
            crate::WindowTextSystem::new(text_system),
            shaper,
            crate::font("Numbers"),
        )
    }

    fn text_run(text: &str, font: &crate::Font) -> [crate::TextRun; 1] {
        [crate::TextRun {
            len: text.len(),
            font: font.clone(),
            ..crate::TextRun::default()
        }]
    }

    #[test]
    fn numbers_put_together_are_kept_in_recent_shapes() {
        let (system, shaper, font) = window_text_system();
        let line = system.layout_line("12", px(13.), &text_run("12", &font), None);
        assert_eq!(shaper.lines(), 4);
        drop(line);
        for _ in 0..3 {
            system.finish_frame();
        }
        system.layout_line("12", px(13.), &text_run("12", &font), None);
        assert_eq!(shaper.lines(), 4);
        system.layout_line("21", px(13.), &text_run("21", &font), None);
        assert_eq!(shaper.lines(), 6);
    }

    #[test]
    fn adding_fonts_measures_numbers_again() {
        let (system, shaper, font) = window_text_system();
        system.layout_line("12", px(13.), &text_run("12", &font), None);
        assert_eq!(shaper.lines(), 4);
        system.add_fonts(Vec::new()).unwrap();
        system.layout_line("12", px(13.), &text_run("12", &font), None);
        assert_eq!(shaper.lines(), 8);
    }

    #[test]
    fn forced_widths_apply_to_numbers_put_together() {
        let (system, _, font) = window_text_system();
        let natural = system.layout_line("74", px(13.), &text_run("74", &font), None);
        assert_eq!(natural.runs[0].glyphs[1].position.x, px(6.5));
        let forced = system.layout_line("74", px(13.), &text_run("74", &font), Some(px(8.)));
        assert_eq!(forced.runs[0].glyphs[1].position.x, px(8.));
        assert_eq!(natural.runs[0].glyphs[1].position.x, px(6.5));
    }

    #[test]
    fn measurements_are_kept_for_a_bounded_number_of_sizes() {
        let shaper = Shaper::default();
        let mut shaping = NumberShaping::default();
        let size = |index: usize| px(8. + index as f32 / 16.);
        for index in 0..MAX_FONTS {
            shaping
                .shape(&shaper, 0, "1", size(index), &run("1"))
                .unwrap();
        }
        assert_eq!(shaping.fonts.len(), MAX_FONTS);
        shaping.shape(&shaper, 0, "1", size(0), &run("1")).unwrap();
        assert_eq!(shaping.fonts.len(), MAX_FONTS);
        shaping
            .shape(&shaper, 0, "1", size(MAX_FONTS), &run("1"))
            .unwrap();
        assert_eq!(shaping.fonts.len(), 1);
    }
}

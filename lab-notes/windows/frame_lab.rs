use std::{
    fs::OpenOptions,
    io::Write,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, SharedString, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    rgb, size,
};
use gpui_platform::application;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Static,
    Wake,
    Anim,
    Timer16,
}

struct FrameLab {
    mode: Mode,
    notified_at: Option<Instant>,
    renders: u64,
    window_start: Instant,
    log: Option<std::fs::File>,
    tick: u64,
}

impl FrameLab {
    fn log(&mut self, line: String) {
        if let Some(log) = self.log.as_mut() {
            let _ = writeln!(log, "{line}");
        }
    }
}

impl Render for FrameLab {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if let Some(notified_at) = self.notified_at.take() {
            let latency = now.duration_since(notified_at).as_secs_f64() * 1000.0;
            self.log(format!("latency_ms {latency:.3}"));
        }
        self.renders += 1;
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            let fps = self.renders as f64 / now.duration_since(self.window_start).as_secs_f64();
            self.log(format!("fps {fps:.2}"));
            self.renders = 0;
            self.window_start = now;
        }
        if self.mode == Mode::Anim {
            window.request_animation_frame();
        }
        let tick = self.tick;
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .p_2()
            .bg(rgb(0x202020))
            .size_full()
            .text_xs()
            .text_color(rgb(0xffffff))
            .children((0..300).map(|ix| {
                let label: SharedString = format!("{}", ix + tick as usize % 7).into();
                div()
                    .w(px(36.))
                    .h(px(20.))
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(0x606060))
                    .bg(rgb(0x303030 + (ix as u32 * 0x010203) % 0x404040))
                    .child(label)
            }))
    }
}

fn main() {
    let mode = match std::env::var("FRAME_LAB").as_deref() {
        Ok("wake") => Mode::Wake,
        Ok("anim") => Mode::Anim,
        Ok("timer16") => Mode::Timer16,
        _ => Mode::Static,
    };
    let log = std::env::var_os("FRAME_LAB_LOG").and_then(|path| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(move |cx| {
                    let interval = match mode {
                        Mode::Wake => Some(Duration::from_millis(250)),
                        Mode::Timer16 => Some(Duration::from_millis(16)),
                        _ => None,
                    };
                    if let Some(interval) = interval {
                        cx.spawn(async move |this, cx| {
                            loop {
                                cx.background_executor().timer(interval).await;
                                let alive = this
                                    .update(cx, |this: &mut FrameLab, cx| {
                                        this.tick += 1;
                                        this.notified_at = Some(Instant::now());
                                        cx.notify();
                                    })
                                    .is_ok();
                                if !alive {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    FrameLab {
                        mode,
                        notified_at: None,
                        renders: 0,
                        window_start: Instant::now(),
                        log,
                        tick: 0,
                    }
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

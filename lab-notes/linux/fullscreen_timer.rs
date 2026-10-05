#[path = "example_support/fonts.rs"]
mod example_support;

use std::time::Duration;

use gpui::{App, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size};
use gpui_platform::application;

struct Ticker {
    ticks: u64,
}

impl Render for Ticker {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        eprintln!(
            "render tick={} fullscreen={} viewport={:?} at_ms={}",
            self.ticks,
            window.is_fullscreen(),
            window.viewport_size(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x202020))
            .text_color(rgb(0xffffff))
            .text_xl()
            .child(format!("tick {}", self.ticks))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }
        let bounds = Bounds::centered(None, size(px(800.), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Fullscreen(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(100))
                                .await;
                            if this
                                .update(cx, |this: &mut Ticker, cx| {
                                    this.ticks += 1;
                                    cx.notify();
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    Ticker { ticks: 0 }
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

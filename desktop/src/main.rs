// `disks::human` stays unused here: this window formats base-1000 sizes like the fix-storage design.
#[allow(dead_code)]
#[path = "../../src/disks.rs"]
mod disks;

use disks::Disk;
use gpui::{
    App, Application, Bounds, Context, FocusHandle, FontWeight, KeyBinding, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};

const BG: u32 = 0x000000;
const TEXT: u32 = 0xe6e6e6;
const MUTED: u32 = 0x8c8c8c;
const EMPTY_DOT: u32 = 0x262626;
const GREEN: u32 = 0x4ade80;
const AMBER: u32 = 0xfbbf24;
const RED: u32 = 0xf87171;
const DOTS: usize = 48;
const METER_ROWS: usize = 3;
const DOT: f32 = 6.0;
const DOT_GAP: f32 = 3.0;
const METER_W: f32 = DOTS as f32 * (DOT + DOT_GAP) - DOT_GAP;
const METER_H: f32 = METER_ROWS as f32 * (DOT + DOT_GAP) - DOT_GAP;
const FONT_SIZE: f32 = 14.0;
const LINE: f32 = FONT_SIZE * 1.5;
const PAD: f32 = 24.0;
const IN_BLOCK: f32 = 6.0;
const BETWEEN_BLOCKS: f32 = 24.0;
const BLOCK_H: f32 = 2.0 * LINE + METER_H + 2.0 * IN_BLOCK;
/// Past this many disks the list scrolls instead of growing the window.
const MAX_VISIBLE: usize = 6;

actions!(alpheus_desktop, [Quit]);

struct DiskListView {
    disks: Vec<Disk>,
    font: SharedString,
    focus_handle: FocusHandle,
}

impl Render for DiskListView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.disks.len();
        let count_label = if count == 1 {
            "1 disk".to_string()
        } else {
            format!("{count} disks")
        };

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .font_family(self.font.clone())
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE))
            .p(px(PAD))
            .flex()
            .flex_col()
            .gap(px(BETWEEN_BLOCKS))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(div().font_weight(FontWeight::BOLD).child("alpheus"))
                    .child(div().text_color(rgb(MUTED)).child(count_label)),
            )
            .child(
                div()
                    .id("disk-list")
                    .flex()
                    .flex_col()
                    .gap(px(BETWEEN_BLOCKS))
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.disks.iter().map(render_disk)),
            )
    }
}

fn render_disk(disk: &Disk) -> impl IntoElement {
    let pct = disk.pct();
    let color = level_color(pct);

    div()
        .w(px(METER_W))
        .flex()
        .flex_col()
        .gap(px(IN_BLOCK))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .min_w(px(0.0))
                                .truncate()
                                .child(disk.mount.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(rgb(MUTED))
                                .child(disk.fstype.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(color))
                        .child(format!("{pct:.0}%")),
                ),
        )
        .child(meter(dot_count(pct), color))
        .child(
            div()
                .flex()
                .flex_row()
                .child(human_si(disk.used))
                .child(div().text_color(rgb(MUTED)).child(" used of "))
                .child(human_si(disk.total))
                .child(div().text_color(rgb(MUTED)).child(" · "))
                .child(human_si(disk.avail))
                .child(div().text_color(rgb(MUTED)).child(" free")),
        )
}

/// A grid of dots filled column by column, like the fix-storage disk meter.
fn meter(filled: usize, color: u32) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap(px(DOT_GAP))
        .children((0..DOTS).map(move |i| {
            let c = if i < filled { color } else { EMPTY_DOT };
            div().flex().flex_col().gap(px(DOT_GAP)).children(
                (0..METER_ROWS).map(move |_| div().size(px(DOT)).rounded_full().bg(rgb(c))),
            )
        }))
}

/// Fits the window to its content so short disk lists leave no empty space below.
fn window_height(disks: usize) -> f32 {
    let n = disks.clamp(1, MAX_VISIBLE) as f32;
    2.0 * PAD + LINE + BETWEEN_BLOCKS + n * BLOCK_H + (n - 1.0) * BETWEEN_BLOCKS
}

/// Linux's CosmicTextSystem matches exact fontdb family names, not fontconfig aliases like "monospace".
fn pick_mono_font(cx: &App) -> SharedString {
    const PREFERRED: [&str; 7] = [
        "JetBrains Mono",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Noto Sans Mono",
        "Ubuntu Mono",
        "Fira Code",
        "Fira Mono",
    ];
    let available = cx.text_system().all_font_names();
    for name in PREFERRED {
        if available.iter().any(|f| f == name) {
            return name.into();
        }
    }
    // Dot-prefixed names like `.ZedMono` are gpui aliases for bundled fonts that may not be installed.
    if let Some(mono) = available
        .into_iter()
        .find(|f| !f.starts_with('.') && f.to_ascii_lowercase().contains("mono"))
    {
        return mono.into();
    }
    ".SystemUIFont".into()
}

fn human_si(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn dot_count(pct: f64) -> usize {
    ((pct.clamp(0.0, 100.0) / 100.0) * DOTS as f64).round() as usize
}

fn level_color(pct: f64) -> u32 {
    if pct < 75.0 {
        GREEN
    } else if pct <= 90.0 {
        AMBER
    } else {
        RED
    }
}

fn main() {
    let disks = disks::list();
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("q", Quit, None),
            KeyBinding::new("escape", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let font = pick_mono_font(cx);
        let bounds = Bounds::centered(
            None,
            size(px(METER_W + 2.0 * PAD), px(window_height(disks.len()))),
            cx,
        );

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("alpheus".into()),
                    ..Default::default()
                }),
                app_id: Some("alpheus".into()),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| {
                    let focus_handle = cx.focus_handle();
                    focus_handle.focus(window);
                    DiskListView {
                        disks,
                        font,
                        focus_handle,
                    }
                })
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_si_sizes_and_counts_dots() {
        assert_eq!(human_si(0), "0 B");
        assert_eq!(human_si(999), "999 B");
        assert_eq!(human_si(1_000), "1.0 KB");
        assert_eq!(human_si(401_300_000_000), "401.3 GB");
        assert_eq!(human_si(2_500_000_000_000), "2.5 TB");

        assert_eq!(dot_count(0.0), 0);
        assert_eq!(dot_count(50.0), 24);
        assert_eq!(dot_count(75.0), 36);
        assert_eq!(dot_count(100.0), 48);
        assert_eq!(dot_count(120.0), 48);
        assert_eq!(dot_count(-5.0), 0);

        assert_eq!(level_color(74.9), GREEN);
        assert_eq!(level_color(75.0), AMBER);
        assert_eq!(level_color(90.0), AMBER);
        assert_eq!(level_color(90.1), RED);

        // One disk: padding, header line, header gap, and one block of two lines, meter and gaps.
        assert_eq!(
            window_height(1),
            2.0 * 24.0 + 21.0 + 24.0 + (2.0 * 21.0 + 24.0 + 2.0 * 6.0)
        );
        assert_eq!(window_height(0), window_height(1));
        assert_eq!(window_height(40), window_height(MAX_VISIBLE));
        assert!(window_height(3) > window_height(2));
    }
}

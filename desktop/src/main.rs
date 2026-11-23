// `disks::human` stays unused here: this window formats base-1000 sizes like the fix-storage design.
#[allow(dead_code)]
#[path = "../../src/disks.rs"]
mod disks;

use disks::Disk;
use gpui::{
    App, Application, Bounds, Context, FocusHandle, FontWeight, KeyBinding, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, relative,
    rgb, size,
};

const BG: u32 = 0x000000;
const TEXT: u32 = 0xe6e6e6;
const MUTED: u32 = 0x8c8c8c;
const FAINT: u32 = 0x4f4f4f;
const GREEN: u32 = 0x4ade80;
const AMBER: u32 = 0xfbbf24;
const RED: u32 = 0xf87171;
const DOTS: usize = 48;
const FONT_SIZE: f32 = 14.0;

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
            .line_height(relative(1.5))
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
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
                    .gap_5()
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.disks.iter().map(render_disk)),
            )
    }
}

fn render_disk(disk: &Disk) -> impl IntoElement {
    let pct = disk.pct();
    let filled = dot_count(pct);
    let empty = DOTS - filled;
    let bar_color = level_color(pct);

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(disk.mount.clone()),
                )
                .child(div().text_color(rgb(MUTED)).child(disk.fstype.clone())),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .child(human_si(disk.used))
                .child(div().text_color(rgb(MUTED)).child(" used of "))
                .child(human_si(disk.total))
                .child(div().text_color(rgb(MUTED)).child(" · "))
                .child(human_si(disk.avail))
                .child(div().text_color(rgb(MUTED)).child(" free · "))
                .child(format!("{pct:.0}%"))
                .child(div().text_color(rgb(MUTED)).child(" full")),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .when(filled > 0, |el| {
                    el.child(div().text_color(rgb(bar_color)).child("•".repeat(filled)))
                })
                .when(empty > 0, |el| {
                    el.child(div().text_color(rgb(FAINT)).child("•".repeat(empty)))
                }),
        )
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
        let bounds = Bounds::centered(None, size(px(560.0), px(420.0)), cx);

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
    }
}

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
const EMPTY_DOT: u32 = 0x333333;
/// Healthy disks stay neutral so color only appears when a disk needs attention.
const CALM: u32 = 0xa3a3a3;
const AMBER: u32 = 0xfbbf24;
const RED: u32 = 0xf87171;
/// 720p, the HD 16:9 size the window opens at.
const WINDOW_W: f32 = 1280.0;
const WINDOW_H: f32 = 720.0;
const DOT: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
const PRIMARY_ROWS: usize = 10;
const CARD_ROWS: usize = 3;
const CARDS_PER_ROW: usize = 3;
const FONT_SIZE: f32 = 13.0;
/// ponytail: assumes a 0.6 em monospace advance (JetBrains Mono, DejaVu Sans Mono); measure the font if others overflow.
const CHAR_W: f32 = FONT_SIZE * 0.6;
const LINE: f32 = FONT_SIZE * 1.5;
const PAD: f32 = 32.0;
const IN_BLOCK: f32 = 6.0;
const BETWEEN_BLOCKS: f32 = 24.0;

actions!(alpheus_desktop, [Quit]);

struct DiskListView {
    /// The disk holding the home folder comes first and gets the full-width meter; the rest are cards.
    disks: Vec<Disk>,
    font: SharedString,
    focus_handle: FocusHandle,
}

impl Render for DiskListView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Measured from the viewport, since a compositor may hand out a smaller window than asked.
        let width = f32::from(window.viewport_size().width) - 2.0 * PAD;
        let gaps = (CARDS_PER_ROW - 1) as f32 * BETWEEN_BLOCKS;
        let card_w = (width - gaps) / CARDS_PER_ROW as f32;
        let (primary, rest) = match self.disks.split_first() {
            Some((first, rest)) => (Some(first), rest),
            None => (None, &[][..]),
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
                    .justify_between()
                    .child(div().font_weight(FontWeight::BOLD).child("alpheus"))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .child("q")
                            .child(div().text_color(rgb(MUTED)).child(" quit")),
                    ),
            )
            .when(primary.is_none(), |el| {
                el.child(
                    div()
                        .text_color(rgb(MUTED))
                        .child("No mounted disks found."),
                )
            })
            .child(
                div()
                    .id("disk-list")
                    .flex()
                    .flex_col()
                    .gap(px(BETWEEN_BLOCKS))
                    .flex_1()
                    .overflow_y_scroll()
                    .children(primary.map(|d| render_block(d, width, PRIMARY_ROWS)))
                    .when(!rest.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(BETWEEN_BLOCKS))
                                .children(rest.iter().map(|d| render_block(d, card_w, CARD_ROWS))),
                        )
                    }),
            )
    }
}

/// One disk: name and percent, a dot meter `rows` tall, then free space. `width` is in logical pixels.
fn render_block(disk: &Disk, width: f32, rows: usize) -> impl IntoElement {
    // Round once so the label, the dot count and the color always agree (74.6 shows as 75% and is amber).
    let pct = disk.pct().round();
    let color = level_color(pct);
    let room = (width - 16.0 - 8.0) / CHAR_W;
    let cols = ((width + DOT_GAP) / (DOT + DOT_GAP)).max(1.0) as usize;
    let mount = fit_path(
        &disk.mount,
        (room as usize).saturating_sub(disk.fstype.len() + 4),
    );

    div()
        .w(px(width))
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
                        .child(div().font_weight(FontWeight::BOLD).child(mount))
                        .child(div().text_color(rgb(MUTED)).child(disk.fstype.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(pct_color(color)))
                        .child(format!("{pct:.0}%")),
                ),
        )
        .child(meter(dot_count(pct, cols), color, cols, rows))
        .child(
            div()
                .flex()
                .flex_row()
                .child(human_si(disk.avail))
                .child(div().text_color(rgb(MUTED)).child(" free of "))
                .child(human_si(disk.total)),
        )
}

/// Shortens a path in the middle, keeping its end, which names the volume.
fn fit_path(path: &str, max_chars: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    match max_chars {
        _ if chars.len() <= max_chars => return path.to_string(),
        0 => return String::new(),
        1 => return "…".to_string(),
        _ => {}
    }
    let head = (max_chars - 1) / 3;
    let tail = max_chars - 1 - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// A grid of dots filled column by column, like the fix-storage disk meter.
fn meter(filled: usize, color: u32, cols: usize, rows: usize) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .gap(px(DOT_GAP))
        .children((0..cols).map(move |i| {
            let c = if i < filled { color } else { EMPTY_DOT };
            div()
                .flex()
                .flex_col()
                .gap(px(DOT_GAP))
                .children((0..rows).map(move |_| div().size(px(DOT)).rounded_full().bg(rgb(c))))
        }))
}

/// Moves the disk holding `home` to the front, keeping the others in mount order.
fn home_first(disks: &mut [Disk], home: &str) {
    let holds =
        |mount: &str| mount == "/" || home == mount || home.starts_with(&format!("{mount}/"));
    if let Some(i) = disks
        .iter()
        .enumerate()
        .filter(|(_, d)| holds(&d.mount))
        .max_by_key(|(_, d)| d.mount.len())
        .map(|(i, _)| i)
    {
        disks[..=i].rotate_right(1);
    }
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

fn dot_count(pct: f64, dots: usize) -> usize {
    ((pct.clamp(0.0, 100.0) / 100.0) * dots as f64).round() as usize
}

fn pct_color(level: u32) -> u32 {
    if level == CALM { TEXT } else { level }
}

fn level_color(pct: f64) -> u32 {
    if pct < 75.0 {
        CALM
    } else if pct <= 90.0 {
        AMBER
    } else {
        RED
    }
}

fn main() {
    let mut disks = disks::list();
    home_first(&mut disks, &std::env::var("HOME").unwrap_or_default());
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
        let bounds = Bounds::centered(None, size(px(WINDOW_W), px(WINDOW_H)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("alpheus".into()),
                    ..Default::default()
                }),
                app_id: Some("alpheus".into()),
                // Below this the cards would get zero or negative widths.
                window_min_size: Some(size(px(640.0), px(360.0))),
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

        assert_eq!(dot_count(0.0, 150), 0);
        assert_eq!(dot_count(50.0, 150), 75);
        assert_eq!(dot_count(100.0, 150), 150);
        assert_eq!(dot_count(120.0, 20), 20);
        assert_eq!(dot_count(-5.0, 20), 0);

        assert_eq!(level_color(74.9), CALM);
        assert_eq!(level_color(75.0), AMBER);
        assert_eq!(level_color(90.0), AMBER);
        assert_eq!(level_color(90.1), RED);

        let disk = |mount: &str| Disk {
            mount: mount.into(),
            fstype: "ext4".into(),
            total: 0,
            used: 0,
            avail: 0,
        };
        let mut list = vec![
            disk("/boot/efi"),
            disk("/"),
            disk("/home"),
            disk("/homework"),
        ];
        home_first(&mut list, "/home/user");
        let order: Vec<&str> = list.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(order, ["/home", "/boot/efi", "/", "/homework"]);

        assert_eq!(fit_path("/boot/efi", 40), "/boot/efi");
        assert_eq!(fit_path("/boot/efi", 1), "…");
        assert_eq!(fit_path("/boot/efi", 0), "");
        let long = fit_path(
            "/run/media/user/AVeryLongVolumeLabelWithoutAnySpacesAtAll",
            40,
        );
        assert_eq!(long.chars().count(), 40);
        assert!(
            long.starts_with("/run/media/u") && long.ends_with("SpacesAtAll") && long.contains('…')
        );
    }
}

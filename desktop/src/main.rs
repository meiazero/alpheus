// `disks::human` stays unused here: this window formats base-1000 sizes like the fix-storage design.
#[allow(dead_code)]
#[path = "../../src/disks.rs"]
mod disks;
#[path = "../../src/scan.rs"]
mod scan;

use disks::Disk;
use gpui::{
    App, Application, Bounds, ClickEvent, Context, FocusHandle, FontWeight, KeyBinding,
    ScrollStrategy, SharedString, TitlebarOptions, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size, uniform_list,
};
use scan::Node;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BG: u32 = 0x000000;
const TEXT: u32 = 0xe6e6e6;
const MUTED: u32 = 0x8c8c8c;
const FAINT: u32 = 0x5c5c5c;
const EMPTY_DOT: u32 = 0x333333;
/// Healthy disks stay neutral so color only appears when a disk needs attention.
const CALM: u32 = 0xa3a3a3;
const AMBER: u32 = 0xfbbf24;
const RED: u32 = 0xf87171;
const SELECTED_BG: u32 = 0x161616;
const HOVER_BG: u32 = 0x0c0c0c;
/// 720p, the HD 16:9 size the window opens at.
const WINDOW_W: f32 = 1280.0;
const WINDOW_H: f32 = 720.0;
const DOT: f32 = 4.0;
const DOT_GAP: f32 = 3.0;
const PRIMARY_ROWS: usize = 6;
const CARD_ROWS: usize = 3;
const CARDS_PER_ROW: usize = 3;
const ROW_DOTS: usize = 24;
const FONT_SIZE: f32 = 13.0;
/// ponytail: assumes a 0.6 em monospace advance (JetBrains Mono, DejaVu Sans Mono); measure the font if others overflow.
const CHAR_W: f32 = FONT_SIZE * 0.6;
const LINE: f32 = FONT_SIZE * 1.5;
const PAD: f32 = 32.0;
const IN_BLOCK: f32 = 6.0;
const BETWEEN_BLOCKS: f32 = 24.0;
const ROW_PAD: f32 = 4.0;
const SIZE_W: f32 = 9.0 * CHAR_W;
const PCT_W: f32 = 4.0 * CHAR_W;
const COL_GAP: f32 = 12.0;
/// A child holding at least this share of its parent is "where the space is".
const DOMINANT: u64 = 2;
const CULPRIT_DEPTH: usize = 12;
/// About 25 frames a second: smooth enough for the sweep without repainting the whole window at 60.
const SWEEP_FRAME: Duration = Duration::from_millis(40);
/// Columns in the sweep's fading tail, and how many milliseconds the head takes per column.
const SWEEP_TAIL: usize = 16;
const SWEEP_MS_PER_COL: u128 = 12;
const SWEEP_ROWS: usize = 3;

actions!(alpheus_desktop, [Quit, Up, Down, Open, Back]);

enum Explorer {
    Scanning {
        files: Arc<AtomicU64>,
        started: Instant,
    },
    Ready(Tree),
}

struct Tree {
    root: Node,
    /// Child indices from the root to the open folder.
    path: Vec<usize>,
    selected: usize,
}

impl Tree {
    fn current(&self) -> &Node {
        self.path.iter().fold(&self.root, |n, &i| &n.children[i])
    }
}

struct DiskListView {
    /// The disk holding the home folder comes first and gets the full-width meter; the rest are cards.
    disks: Vec<Disk>,
    home: PathBuf,
    explorer: Explorer,
    scroll: UniformListScrollHandle,
    font: SharedString,
    focus_handle: FocusHandle,
}

impl DiskListView {
    fn start_scan(&mut self, cx: &mut Context<Self>) {
        let files = Arc::new(AtomicU64::new(0));
        self.explorer = Explorer::Scanning {
            files: files.clone(),
            started: Instant::now(),
        };
        let home = self.home.clone();
        let scanned = cx
            .background_executor()
            .spawn(async move { scan::scan(&home, &files) });
        cx.spawn(async move |this, cx| {
            let root = scanned.await;
            this.update(cx, |view, cx| {
                view.explorer = Explorer::Ready(Tree {
                    root,
                    path: vec![],
                    selected: 0,
                });
                cx.notify();
            })
        })
        .detach();
        // Repaints the sweep and the counter until the scan lands.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SWEEP_FRAME).await;
                let scanning = this.update(cx, |view, cx| {
                    cx.notify();
                    matches!(view.explorer, Explorer::Scanning { .. })
                });
                if !matches!(scanning, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn tree(&mut self) -> Option<&mut Tree> {
        match &mut self.explorer {
            Explorer::Ready(tree) => Some(tree),
            Explorer::Scanning { .. } => None,
        }
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tree) = self.tree() {
            let last = tree.current().children.len().saturating_sub(1);
            tree.selected = ix.min(last);
            let selected = tree.selected;
            self.scroll.scroll_to_item(selected, ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        if let Some(tree) = self.tree() {
            let ix = tree.selected;
            let opens = tree
                .current()
                .children
                .get(ix)
                .is_some_and(|c| c.is_dir && !c.children.is_empty());
            if opens {
                tree.path.push(ix);
                tree.selected = 0;
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(tree) = self.tree()
            && let Some(ix) = tree.path.pop()
        {
            tree.selected = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn render_explorer(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = match &self.explorer {
            Explorer::Scanning { files, started } => {
                let elapsed = started.elapsed();
                return div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(IN_BLOCK))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("Measuring {}", tilde(&self.home, &self.home))),
                            )
                            .child(
                                div()
                                    .text_color(rgb(MUTED))
                                    .child("finding what takes the space in your home folder"),
                            ),
                    )
                    .child(sweep(columns(width), elapsed))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .child(thousands(files.load(Ordering::Relaxed)))
                            .child(div().text_color(rgb(MUTED)).child(" items checked · "))
                            .child(clock(elapsed)),
                    )
                    .child(
                        div()
                            .text_color(rgb(FAINT))
                            .child("Folders appear here when the scan finishes."),
                    )
                    .into_any_element();
            }
            Explorer::Ready(tree) => tree,
        };
        let node = tree.current();
        let mut location = self.home.clone();
        let mut walk = &tree.root;
        for &i in &tree.path {
            walk = &walk.children[i];
            location.push(&walk.name);
        }
        let count = node.children.len();

        div()
            .flex_1()
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
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child(tilde(&location, &self.home)),
                            )
                            .child(
                                div()
                                    .text_color(rgb(MUTED))
                                    .child(format!("{} · {count} items", human_si(node.size))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .text_color(rgb(MUTED))
                            .child(key("↑↓"))
                            .child(" move  ")
                            .child(key("enter"))
                            .child(" open  ")
                            .child(key("←"))
                            .child(" back"),
                    ),
            )
            .when(count == 0, |el| {
                el.child(
                    div()
                        .text_color(rgb(MUTED))
                        .child("Nothing here you can remove."),
                )
            })
            .child(
                uniform_list(
                    "entries",
                    count,
                    cx.processor(move |view, range: std::ops::Range<usize>, _window, cx| {
                        let Explorer::Ready(tree) = &view.explorer else {
                            return vec![];
                        };
                        let node = tree.current();
                        range
                            .map(|ix| {
                                let child = &node.children[ix];
                                render_entry(child, node.size, ix == tree.selected, width)
                                    .id(ix)
                                    .on_click(cx.listener(
                                        move |view, event: &ClickEvent, _, cx| {
                                            view.select(ix, cx);
                                            if event.click_count() >= 2 {
                                                view.open(cx);
                                            }
                                        },
                                    ))
                            })
                            .collect()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .flex_1(),
            )
            .into_any_element()
    }
}

impl Render for DiskListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(|view, _: &Up, _, cx| {
                if let Some(ix) = view.tree().map(|t| t.selected) {
                    view.select(ix.saturating_sub(1), cx);
                }
            }))
            .on_action(cx.listener(|view, _: &Down, _, cx| {
                if let Some(ix) = view.tree().map(|t| t.selected) {
                    view.select(ix + 1, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &Open, _, cx| view.open(cx)))
            .on_action(cx.listener(|view, _: &Back, _, cx| view.back(cx)))
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
                            .text_color(rgb(MUTED))
                            .child(key("q"))
                            .child(" quit"),
                    ),
            )
            .when(primary.is_none(), |el| {
                el.child(
                    div()
                        .text_color(rgb(MUTED))
                        .child("No mounted disks found."),
                )
            })
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
            })
            .child(self.render_explorer(width, cx))
    }
}

/// A key name in the hint lines: bright, so the eye finds the key before the word.
fn key(name: &str) -> impl IntoElement {
    div().text_color(rgb(TEXT)).child(name.to_string())
}

/// One disk: name and percent, a dot meter `rows` tall, then free space. `width` is in logical pixels.
fn render_block(disk: &Disk, width: f32, rows: usize) -> impl IntoElement {
    // Round once so the label, the dot count and the color always agree (74.6 shows as 75% and is amber).
    let pct = disk.pct().round();
    let color = level_color(pct);
    let room = (width - 16.0 - 8.0) / CHAR_W;
    let cols = columns(width);
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

/// One folder or file in the explorer: name, size, share of the open folder, then where its space is.
/// Every entry is two lines tall, because the list needs equal heights.
fn render_entry(child: &Node, parent: u64, selected: bool, width: f32) -> gpui::Div {
    let share = if parent == 0 {
        0.0
    } else {
        (child.size as f64 * 100.0 / parent as f64).round()
    };
    let name = if child.is_dir {
        format!("{}/", child.name)
    } else {
        child.name.clone()
    };
    let room =
        (width - 2.0 * CHAR_W - SIZE_W - meter_len(ROW_DOTS) - PCT_W - 3.0 * COL_GAP) / CHAR_W;
    let chain = culprit(child);
    let detail = chain.last().map(|last| {
        let names: Vec<&str> = chain.iter().map(|n| n.name.as_str()).collect();
        let tail = if last.is_dir { "/" } else { "" };
        let joined = format!("{}{tail}", names.join("/"));
        let pct = (last.size as f64 * 100.0 / child.size.max(1) as f64).round();
        (joined, format!("{} · {pct:.0}%", human_si(last.size)))
    });

    div()
        .w_full()
        .py(px(ROW_PAD))
        .px(px(CHAR_W))
        .flex()
        .flex_col()
        .whitespace_nowrap()
        .when(selected, |el| el.bg(rgb(SELECTED_BG)))
        .when(!selected, |el| el.hover(|s| s.bg(rgb(HOVER_BG))))
        .child(
            div()
                .h(px(LINE))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(COL_GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .w(px(2.0 * CHAR_W))
                                .flex_none()
                                .text_color(rgb(MUTED))
                                .child(if selected { "›" } else { "" }),
                        )
                        .child(fit_path(&name, room as usize)),
                )
                .child(
                    div()
                        .w(px(SIZE_W))
                        .flex_none()
                        .text_right()
                        .child(human_si(child.size)),
                )
                .child(meter(dot_count(share, ROW_DOTS), CALM, ROW_DOTS, 1))
                .child(
                    div()
                        .w(px(PCT_W))
                        .flex_none()
                        .text_right()
                        .text_color(rgb(MUTED))
                        .child(format!("{share:.0}%")),
                ),
        )
        .child(
            div()
                .h(px(LINE))
                .pl(px(2.0 * CHAR_W))
                .flex()
                .flex_row()
                .text_color(rgb(FAINT))
                .when_some(detail, |el, (path, amount)| {
                    let budget = (room as usize).saturating_sub(amount.chars().count() + 14);
                    el.child("└ mostly in ")
                        .child(div().text_color(rgb(MUTED)).child(fit_path(&path, budget)))
                        .child(format!("  {amount}"))
                }),
        )
}

/// Follows the biggest child down while it holds at least half of its parent, like fix-storage's "mostly in".
fn culprit(node: &Node) -> Vec<&Node> {
    let mut chain = vec![];
    let mut current = node;
    // Children come sorted by size, so the first one is the biggest.
    while let Some(best) = current.children.first() {
        if best.size * DOMINANT < current.size || chain.len() == CULPRIT_DEPTH {
            break;
        }
        chain.push(best);
        current = best;
    }
    chain
}

/// Shows paths under the home folder as `~/...`.
fn tilde(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
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

/// How many dot columns fit in `width` logical pixels.
fn columns(width: f32) -> usize {
    ((width + DOT_GAP) / (DOT + DOT_GAP)).max(1.0) as usize
}

/// The loading meter: a bright head moves left to right with a fading tail, then wraps around.
fn sweep(cols: usize, elapsed: Duration) -> impl IntoElement {
    let lap = (cols + SWEEP_TAIL) as u128;
    let head = (elapsed.as_millis() / SWEEP_MS_PER_COL % lap) as usize;
    div()
        .flex_none()
        .flex()
        .flex_row()
        .gap(px(DOT_GAP))
        .children((0..cols).map(move |i| {
            let c = sweep_shade(head, i);
            div().flex().flex_col().gap(px(DOT_GAP)).children(
                (0..SWEEP_ROWS).map(move |_| div().size(px(DOT)).rounded_full().bg(rgb(c))),
            )
        }))
}

/// Grey for column `i` when the sweep's head is at `head`: bright at the head, fading to the empty dot.
fn sweep_shade(head: usize, i: usize) -> u32 {
    let (lo, hi) = (EMPTY_DOT & 0xff, TEXT & 0xff);
    let v = match head.checked_sub(i) {
        Some(d) if d < SWEEP_TAIL => hi - (hi - lo) * d as u32 / SWEEP_TAIL as u32,
        _ => lo,
    };
    v << 16 | v << 8 | v
}

/// 3393299 -> "3,393,299".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 84 s -> "1 min 24 s".
fn clock(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{} min {} s", secs / 60, secs % 60)
    }
}

const fn meter_len(dots: usize) -> f32 {
    dots as f32 * (DOT + DOT_GAP) - DOT_GAP
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
    let home = std::env::var("HOME").unwrap_or_default();
    home_first(&mut disks, &home);
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("q", Quit, None),
            KeyBinding::new("up", Up, None),
            KeyBinding::new("k", Up, None),
            KeyBinding::new("down", Down, None),
            KeyBinding::new("j", Down, None),
            KeyBinding::new("enter", Open, None),
            KeyBinding::new("right", Open, None),
            KeyBinding::new("l", Open, None),
            KeyBinding::new("left", Back, None),
            KeyBinding::new("h", Back, None),
            KeyBinding::new("backspace", Back, None),
            KeyBinding::new("escape", Back, None),
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
                    let mut view = DiskListView {
                        disks,
                        home: PathBuf::from(home),
                        explorer: Explorer::Scanning {
                            files: Arc::default(),
                            started: Instant::now(),
                        },
                        scroll: UniformListScrollHandle::new(),
                        font,
                        focus_handle,
                    };
                    view.start_scan(cx);
                    view
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

        let home = std::path::Path::new("/home/user");
        assert_eq!(tilde(home, home), "~");
        assert_eq!(tilde(&home.join("Projects"), home), "~/Projects");
        assert_eq!(tilde(std::path::Path::new("/opt"), home), "/opt");

        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(3_393_299), "3,393,299");
        assert_eq!(clock(Duration::from_secs(9)), "9 s");
        assert_eq!(clock(Duration::from_secs(84)), "1 min 24 s");

        assert_eq!(sweep_shade(10, 10), TEXT);
        assert_eq!(
            sweep_shade(10, 11),
            EMPTY_DOT,
            "ahead of the head stays empty"
        );
        assert_eq!(sweep_shade(40, 10), EMPTY_DOT, "past the tail stays empty");
        assert!(sweep_shade(10, 5) & 0xff < TEXT & 0xff);
    }

    #[test]
    fn culprit_follows_the_dominant_child() {
        let leaf = |name: &str, size: u64, is_dir: bool| Node {
            name: name.into(),
            size,
            is_dir,
            children: vec![],
            hidden: 0,
        };
        let dir = |name: &str, children: Vec<Node>| Node {
            name: name.into(),
            size: children.iter().map(|c| c.size).sum(),
            is_dir: true,
            children,
            hidden: 0,
        };
        // target/ holds 90 of 100 bytes, and debug/ holds 80 of target's 90; inside debug/ nothing dominates.
        let project = dir(
            "alpheus",
            vec![
                dir(
                    "target",
                    vec![
                        dir(
                            "debug",
                            vec![
                                leaf("a", 30, false),
                                leaf("b", 30, false),
                                leaf("c", 20, false),
                            ],
                        ),
                        leaf("x", 10, false),
                    ],
                ),
                leaf("src", 10, true),
            ],
        );
        let names: Vec<&str> = culprit(&project).iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["target", "debug"]);
        assert!(culprit(&leaf("file", 5, false)).is_empty());
    }
}

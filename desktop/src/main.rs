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
use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::rc::Rc;
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

/// The home folder as measured so far. Each top-level entry is scanned on its own and lands when done,
/// so the root re-sorts while the user browses. Top-level entries are therefore known by an id (names
/// can collide after lossy UTF-8 conversion); deeper levels never change and are known by index.
struct Explorer {
    root: Node,
    /// The id of each of `root.children`, in the same order.
    root_ids: Vec<usize>,
    /// Top-level entries still being measured, in name order.
    pending: Vec<(usize, String)>,
    total: usize,
    files: Arc<AtomicU64>,
    started: Instant,
    /// How long the whole scan took, once every entry has landed.
    took: Option<Duration>,
    /// From the root to the open folder: a top-level id, then child indices.
    path: Vec<usize>,
    selected: usize,
    /// Set once the user moves the selection; until then it stays on the biggest entry.
    picked: bool,
}

enum Entry<'a> {
    Done(usize, &'a Node),
    Pending(usize, &'a str),
}

impl Entry<'_> {
    /// The id at the root, the child index below it.
    fn key(&self) -> usize {
        match self {
            Entry::Done(key, _) | Entry::Pending(key, _) => *key,
        }
    }
}

impl Explorer {
    /// `own_size` is the home folder's own blocks, which no child accounts for.
    fn new(pending: Vec<(usize, String)>, own_size: u64) -> Self {
        Explorer {
            root: Node {
                name: String::new(),
                size: own_size,
                is_dir: true,
                children: vec![],
                hidden: 0,
            },
            root_ids: vec![],
            total: pending.len(),
            took: pending.is_empty().then_some(Duration::ZERO),
            pending,
            files: Arc::default(),
            started: Instant::now(),
            path: vec![],
            selected: 0,
            picked: false,
        }
    }

    /// The folders from the root's child down to the open one.
    fn trail(&self) -> Vec<&Node> {
        let mut trail = vec![];
        let Some((&id, rest)) = self.path.split_first() else {
            return trail;
        };
        let Some(pos) = self.root_ids.iter().position(|&i| i == id) else {
            return trail;
        };
        let mut node = &self.root.children[pos];
        trail.push(node);
        for &ix in rest {
            match node.children.get(ix) {
                Some(child) => node = child,
                None => break,
            }
            trail.push(node);
        }
        trail
    }

    fn current(&self) -> &Node {
        self.trail().last().copied().unwrap_or(&self.root)
    }

    /// Measured entries biggest first; at the root, the ones still being measured follow.
    fn entries(&self) -> Vec<Entry<'_>> {
        if !self.path.is_empty() {
            let node = self.current();
            return node
                .children
                .iter()
                .enumerate()
                .map(|(i, c)| Entry::Done(i, c))
                .collect();
        }
        let done = self.root_ids.iter().zip(&self.root.children);
        let mut list: Vec<Entry> = done.map(|(&id, c)| Entry::Done(id, c)).collect();
        list.extend(self.pending.iter().map(|(id, n)| Entry::Pending(*id, n)));
        list
    }

    /// Adds a measured top-level entry. It is listed under the same rule as `scan::walk` uses for
    /// children: removable from the home folder itself, or holding something removable inside.
    /// The entry the user picked stays selected as the root re-sorts.
    fn land(&mut self, id: usize, node: Node, removable: bool) {
        let keep = self
            .entries()
            .get(self.selected)
            .filter(|_| self.picked)
            .map(|e| e.key());
        self.pending.retain(|(i, _)| *i != id);
        self.root.size += node.size;
        if node.size > 0 && (removable || !node.children.is_empty()) {
            let at = self.root.children.partition_point(|c| c.size >= node.size);
            self.root.children.insert(at, node);
            self.root_ids.insert(at, id);
        } else {
            self.root.hidden += node.size;
        }
        if self.pending.is_empty() {
            self.took = Some(self.started.elapsed());
        }
        // An entry that landed hidden drops out of the list, which can leave the selection past the end.
        self.selected = self.selected.min(self.entries().len().saturating_sub(1));
        if self.path.is_empty()
            && let Some(key) = keep
            && let Some(ix) = self.entries().iter().position(|e| e.key() == key)
        {
            self.selected = ix;
        }
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
    /// Measures each top-level entry of the home folder on its own, a few at a time, so results arrive
    /// as they finish and the scan uses several cores without filling gpui's background pool.
    /// ponytail: a hard link shared by two top-level folders counts in both; one shared `seen` set would need scan.rs changes.
    fn start_scan(&mut self, cx: &mut Context<Self>) {
        let meta = std::fs::symlink_metadata(&self.home).ok();
        let dev = meta.as_ref().map(|m| m.dev());
        // Same rule as scan::walk: unlinking needs write and search on the parent, and a sticky one
        // only lets owners remove their own entries.
        let writable = std::ffi::CString::new(self.home.as_os_str().as_bytes())
            .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } == 0);
        let sticky = meta.as_ref().is_some_and(|m| m.mode() & 0o1000 != 0);
        let uid = unsafe { libc::getuid() };
        let mut entries: Vec<(String, PathBuf, bool)> = std::fs::read_dir(&self.home)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                // DirEntry::metadata does not follow symlinks; another filesystem mounted inside home is skipped, like `du -x`.
                let m = e.metadata().ok().filter(|m| Some(m.dev()) == dev)?;
                let removable = writable && (!sticky || m.uid() == uid);
                Some((
                    e.file_name().to_string_lossy().into_owned(),
                    e.path(),
                    removable,
                ))
            })
            .collect();
        entries.sort();
        let own_size = meta.map_or(0, |m| m.blocks() * 512);
        let pending = entries
            .iter()
            .enumerate()
            .map(|(id, (name, ..))| (id, name.clone()));
        self.explorer = Explorer::new(pending.collect(), own_size);
        let queue: Rc<RefCell<VecDeque<(usize, PathBuf, bool)>>> = Rc::new(RefCell::new(
            entries
                .into_iter()
                .enumerate()
                .map(|(id, (_, path, removable))| (id, path, removable))
                .collect(),
        ));
        // ponytail: half the cores, 2 to 8; disks rather than CPUs set the real limit.
        let workers = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(2, 8));
        for _ in 0..workers {
            let queue = queue.clone();
            let files = self.explorer.files.clone();
            cx.spawn(async move |this, cx| {
                loop {
                    let next = queue.borrow_mut().pop_front();
                    let Some((id, path, removable)) = next else {
                        break;
                    };
                    let files = files.clone();
                    let scanned = cx
                        .background_executor()
                        .spawn(async move { scan::scan(&path, &files) })
                        .await;
                    let landed = this.update(cx, |view, cx| {
                        let name = view
                            .explorer
                            .pending
                            .iter()
                            .find(|(i, _)| *i == id)
                            .map(|(_, n)| n.clone());
                        let node = Node {
                            name: name.unwrap_or_default(),
                            ..scanned
                        };
                        view.explorer.land(id, node, removable);
                        cx.notify();
                    });
                    if landed.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        // Repaints the progress meter and the counter until the last entry lands.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SWEEP_FRAME).await;
                let scanning = this.update(cx, |view, cx| {
                    cx.notify();
                    view.explorer.took.is_none()
                });
                if !matches!(scanning, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.explorer.picked = true;
        let last = self.explorer.entries().len().saturating_sub(1);
        self.explorer.selected = ix.min(last);
        self.scroll
            .scroll_to_item(self.explorer.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let target = match self.explorer.entries().get(self.explorer.selected) {
            Some(Entry::Done(key, node)) if node.is_dir && !node.children.is_empty() => Some(*key),
            _ => None,
        };
        if let Some(key) = target {
            self.explorer.path.push(key);
            self.explorer.selected = 0;
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.explorer.path.pop() {
            let ix = self
                .explorer
                .entries()
                .iter()
                .position(|e| e.key() == key)
                .unwrap_or(0);
            self.explorer.selected = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn render_explorer(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let ex = &self.explorer;
        let node = ex.current();
        let mut location = self.home.clone();
        for folder in ex.trail() {
            location.push(&folder.name);
        }
        let count = ex.entries().len();
        let done = ex.total - ex.pending.len();
        let items = thousands(ex.files.load(Ordering::Relaxed));
        let summary = match (ex.path.is_empty(), ex.took) {
            (true, None) => format!(
                "measuring {done} of {} · {items} items · {}",
                ex.total,
                clock(ex.started.elapsed())
            ),
            (true, Some(took)) => format!(
                "{} · {count} items · measured in {}",
                human_si(node.size),
                clock(took)
            ),
            (false, _) => format!("{} · {count} items", human_si(node.size)),
        };
        let cols = columns(width);
        let filled = (cols * done).checked_div(ex.total).unwrap_or(cols);

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
                            .child(div().text_color(rgb(MUTED)).child(summary)),
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
            // The meter slot stays after the scan so the list does not jump up when it ends.
            .child(
                div()
                    .h(px(meter_len(SWEEP_ROWS)))
                    .when(ex.took.is_none(), |el| {
                        el.child(progress(cols, filled, ex.started.elapsed()))
                    }),
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
                        let ex = &view.explorer;
                        let entries = ex.entries();
                        let parent = ex.current().size;
                        range
                            .filter_map(|ix| {
                                let selected = ix == ex.selected;
                                let row = match entries.get(ix)? {
                                    Entry::Done(_, child) => {
                                        render_entry(child, parent, selected, width)
                                    }
                                    Entry::Pending(_, name) => {
                                        render_pending(name, selected, width)
                                    }
                                };
                                Some(row.id(ix).on_click(cx.listener(
                                    move |view, event: &ClickEvent, _, cx| {
                                        view.select(ix, cx);
                                        if event.click_count() >= 2 {
                                            view.open(cx);
                                        }
                                    },
                                )))
                            })
                            .collect()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .flex_1(),
            )
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
                let ix = view.explorer.selected.saturating_sub(1);
                view.select(ix, cx);
            }))
            .on_action(cx.listener(|view, _: &Down, _, cx| {
                let ix = view.explorer.selected + 1;
                view.select(ix, cx);
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

/// A top-level entry still being measured: same slots as a measured one, so nothing moves when it lands.
fn render_pending(name: &str, selected: bool, width: f32) -> gpui::Div {
    let room =
        (width - 2.0 * CHAR_W - SIZE_W - meter_len(ROW_DOTS) - PCT_W - 3.0 * COL_GAP) / CHAR_W;
    div()
        .w_full()
        .py(px(ROW_PAD))
        .px(px(CHAR_W))
        .flex()
        .flex_col()
        .whitespace_nowrap()
        .text_color(rgb(MUTED))
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
                        .child(div().w(px(2.0 * CHAR_W)).flex_none().child(if selected {
                            "›"
                        } else {
                            ""
                        }))
                        .child(fit_path(name, room as usize)),
                )
                .child(
                    div()
                        .w(px(SIZE_W))
                        .flex_none()
                        .text_right()
                        .text_color(rgb(FAINT))
                        .child("measuring"),
                )
                .child(meter(0, CALM, ROW_DOTS, 1))
                .child(div().w(px(PCT_W)).flex_none()),
        )
        .child(div().h(px(LINE)))
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

/// The scan meter: columns before `filled` are done; over the rest a bright head sweeps with a fading tail.
fn progress(cols: usize, filled: usize, elapsed: Duration) -> impl IntoElement {
    let lap = (cols.saturating_sub(filled) + SWEEP_TAIL) as u128;
    let head = filled + (elapsed.as_millis() / SWEEP_MS_PER_COL % lap) as usize;
    div()
        .flex_none()
        .flex()
        .flex_row()
        .gap(px(DOT_GAP))
        .children((0..cols).map(move |i| {
            let c = if i < filled {
                CALM
            } else {
                sweep_shade(head, i)
            };
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
                        explorer: Explorer::new(vec![], 0),
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
    fn landing_entries_sort_live_and_keep_the_selection() {
        let leaf = |size: u64| Node {
            name: String::new(),
            size,
            is_dir: false,
            children: vec![],
            hidden: 0,
        };
        // Ids 0 and 1 share a name, as two non-UTF-8 names can after lossy conversion.
        let names = ["a", "a", "c", "d"].iter().enumerate();
        let mut ex = Explorer::new(names.map(|(i, n)| (i, n.to_string())).collect(), 4);
        let keys = |ex: &Explorer| -> Vec<usize> { ex.entries().iter().map(|e| e.key()).collect() };
        assert_eq!(keys(&ex), [0, 1, 2, 3]);
        ex.land(1, leaf(10), true);
        assert_eq!(
            keys(&ex),
            [1, 0, 2, 3],
            "the twin named alike is still pending"
        );
        assert_eq!(ex.selected, 0, "an untouched selection stays on top");
        ex.land(2, leaf(50), true);
        assert_eq!(keys(&ex), [2, 1, 0, 3]);
        assert_eq!(ex.selected, 0, "still on top after a bigger entry lands");
        ex.selected = 1; // the user picks id 1
        ex.picked = true;
        ex.land(3, leaf(20), true);
        assert_eq!(
            ex.entries()[ex.selected].key(),
            1,
            "a picked entry follows its id"
        );
        assert!(ex.took.is_none());
        ex.land(0, leaf(7), false);
        assert_eq!(
            keys(&ex),
            [2, 3, 1],
            "an entry the user cannot remove is hidden"
        );
        assert!(ex.took.is_some());
        assert_eq!(
            ex.root.size,
            4 + 10 + 50 + 20 + 7,
            "own blocks plus every entry"
        );
        assert_eq!(ex.root.hidden, 7);
        assert!(
            Explorer::new(vec![], 0).took.is_some(),
            "an empty home is done at once"
        );
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

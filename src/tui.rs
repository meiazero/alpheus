use crate::clean::{self, Action, Candidate, Risk};
use crate::disks::{self, Disk, human};
use crate::scan::{self, Node};
use crate::{GREEN, ORANGE, RED, TRACK, YELLOW, level};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

pub enum Start {
    Disks,
    Explore(PathBuf),
    Clean(PathBuf),
}

enum Done {
    Tree(Node),
    Clean(Vec<Candidate>),
}

enum Screen {
    Disks,
    Scanning {
        path: PathBuf,
        files: Arc<AtomicU64>,
        rx: mpsc::Receiver<Done>,
        started: Instant,
    },
    Tree(Tree),
    Clean(Clean),
}

struct Tree {
    root: Node,
    /// Child indices from the root down to the directory being shown.
    path: Vec<usize>,
    /// Selected row in each parent, restored when going back up.
    parents_sel: Vec<usize>,
    list: ListState,
}

impl Tree {
    fn current(&self) -> &Node {
        self.path.iter().fold(&self.root, |n, &i| &n.children[i])
    }

    fn location(&self) -> PathBuf {
        let mut n = &self.root;
        let mut out = PathBuf::from(&self.root.name);
        for &i in &self.path {
            n = &n.children[i];
            out.push(&n.name);
        }
        out
    }
}

enum Mode {
    Browse,
    Confirm,
    /// Set by the confirm keys and run on the next loop turn, after a "Working…" frame is on screen.
    Run {
        permanent: bool,
    },
}

struct Clean {
    root: PathBuf,
    items: Vec<Candidate>,
    marked: Vec<bool>,
    table: TableState,
    mode: Mode,
    status: String,
}

impl Clean {
    fn marked(&self) -> impl Iterator<Item = &Candidate> {
        self.items
            .iter()
            .zip(&self.marked)
            .filter(|(_, m)| **m)
            .map(|(c, _)| c)
    }

    fn marked_size(&self) -> u64 {
        self.marked().map(|c| c.size).sum()
    }

    fn execute(&mut self, permanent: bool) {
        let (mut freed, mut ok, mut failed) = (0, 0, 0);
        let mut keep_items = Vec::new();
        let mut keep_marked = Vec::new();
        for (mut c, m) in std::mem::take(&mut self.items)
            .into_iter()
            .zip(std::mem::take(&mut self.marked))
        {
            if m {
                match clean::execute(&c, permanent) {
                    Ok(()) => {
                        freed += c.size;
                        ok += 1;
                        continue;
                    }
                    Err(e) => {
                        failed += 1;
                        c.blocked = Some(format!("failed: {e}"));
                    }
                }
            }
            keep_items.push(c);
            keep_marked.push(false);
        }
        self.items = keep_items;
        self.marked = keep_marked;
        let verb = if permanent { "Freed" } else { "Moved to trash" };
        self.status = format!("{verb} {} from {ok} items", human(freed));
        if !permanent && ok > 0 {
            self.status += " (space returns when the trash is emptied)";
        }
        if failed > 0 {
            self.status += &format!(" · {failed} failed, see red rows");
        }
        self.mode = Mode::Browse;
    }
}

struct App {
    disks: Vec<Disk>,
    table: TableState,
    screen: Screen,
}

pub fn run(start: Start) -> io::Result<()> {
    let mut app = App {
        disks: disks::list(),
        table: TableState::default().with_selected(0),
        screen: Screen::Disks,
    };
    match start {
        Start::Disks => {}
        Start::Explore(path) => app.start_scan(path, false),
        Start::Clean(path) => app.start_scan(path, true),
    }
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

impl App {
    fn start_scan(&mut self, path: PathBuf, cleanup: bool) {
        let files = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel();
        let (p, f) = (path.clone(), files.clone());
        std::thread::spawn(move || {
            let done = if cleanup {
                Done::Clean(clean::find(&p, &f))
            } else {
                Done::Tree(scan::scan(&p, &f))
            };
            let _ = tx.send(done);
        });
        self.screen = Screen::Scanning {
            path,
            files,
            rx,
            started: Instant::now(),
        };
    }

    fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        loop {
            if let Screen::Scanning { rx, path, .. } = &self.screen {
                match rx.try_recv() {
                    Ok(Done::Tree(root)) => {
                        self.screen = Screen::Tree(Tree {
                            root,
                            path: vec![],
                            parents_sel: vec![],
                            list: ListState::default().with_selected(Some(0)),
                        })
                    }
                    Ok(Done::Clean(items)) => {
                        let status = format!(
                            "{} candidates · {} reclaimable",
                            items.len(),
                            human(
                                items
                                    .iter()
                                    .filter(|c| c.blocked.is_none())
                                    .map(|c| c.size)
                                    .sum()
                            )
                        );
                        self.screen = Screen::Clean(Clean {
                            root: path.clone(),
                            marked: vec![false; items.len()],
                            items,
                            table: TableState::default().with_selected(0),
                            mode: Mode::Browse,
                            status,
                        });
                    }
                    Err(_) => {}
                }
            }
            terminal.draw(|f| self.draw(f))?;
            if let Screen::Clean(c) = &mut self.screen
                && let Mode::Run { permanent } = c.mode
            {
                c.execute(permanent);
                self.disks = disks::list();
                continue;
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            // Raw mode turns Ctrl-C into a plain key event, so quit has to be handled here.
            let ctrl_c =
                key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            if key.code == KeyCode::Char('q') || ctrl_c {
                return Ok(());
            }
            self.on_key(key.code);
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        match &mut self.screen {
            Screen::Disks => match code {
                KeyCode::Down | KeyCode::Char('j') => self.table.select_next(),
                KeyCode::Up | KeyCode::Char('k') => self.table.select_previous(),
                KeyCode::Char('r') => self.disks = disks::list(),
                KeyCode::Char('c') => self.start_scan(home(), true),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    if let Some(d) = self.table.selected().and_then(|i| self.disks.get(i)) {
                        self.start_scan(PathBuf::from(&d.mount), false);
                    }
                }
                _ => {}
            },
            // A scan in flight is detached on Esc; its thread finishes in the background and the result is dropped.
            Screen::Scanning { .. } => {
                if code == KeyCode::Esc {
                    self.screen = Screen::Disks;
                }
            }
            Screen::Tree(t) => match code {
                KeyCode::Down | KeyCode::Char('j') => t.list.select_next(),
                KeyCode::Up | KeyCode::Char('k') => t.list.select_previous(),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    let sel = t.list.selected().unwrap_or(0);
                    if t.current()
                        .children
                        .get(sel)
                        .is_some_and(|c| !c.children.is_empty())
                    {
                        t.path.push(sel);
                        t.parents_sel.push(sel);
                        t.list.select(Some(0));
                    }
                }
                KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                    if t.path.pop().is_some() {
                        t.list.select(t.parents_sel.pop());
                    }
                }
                KeyCode::Char('c') => {
                    let here = t.location();
                    self.start_scan(here, true);
                }
                KeyCode::Esc => {
                    self.disks = disks::list();
                    self.screen = Screen::Disks;
                }
                _ => {}
            },
            Screen::Clean(c) => match c.mode {
                Mode::Run { .. } => {}
                Mode::Confirm => match code {
                    KeyCode::Enter | KeyCode::Char('t') => c.mode = Mode::Run { permanent: false },
                    KeyCode::Char('D') => c.mode = Mode::Run { permanent: true },
                    KeyCode::Esc | KeyCode::Char('n') => c.mode = Mode::Browse,
                    _ => {}
                },
                Mode::Browse => match code {
                    KeyCode::Down | KeyCode::Char('j') => c.table.select_next(),
                    KeyCode::Up | KeyCode::Char('k') => c.table.select_previous(),
                    KeyCode::Char(' ') => {
                        if let Some(i) = c
                            .table
                            .selected()
                            .filter(|&i| c.items.get(i).is_some_and(|it| it.blocked.is_none()))
                        {
                            c.marked[i] = !c.marked[i];
                            c.table.select_next();
                        }
                    }
                    // Bulk select never picks Care items or artifacts of projects touched this week.
                    KeyCode::Char('a') => {
                        for (m, it) in c.marked.iter_mut().zip(&c.items) {
                            *m = it.blocked.is_none() && !it.recent && it.risk != Risk::Care;
                        }
                    }
                    KeyCode::Char('n') => c.marked.iter_mut().for_each(|m| *m = false),
                    KeyCode::Enter | KeyCode::Char('d') => {
                        if c.marked.iter().any(|&m| m) {
                            c.mode = Mode::Confirm;
                        }
                    }
                    KeyCode::Char('r') => {
                        let root = c.root.clone();
                        self.start_scan(root, true);
                    }
                    KeyCode::Esc => {
                        self.disks = disks::list();
                        self.screen = Screen::Disks;
                    }
                    _ => {}
                },
            },
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let [main, help] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
        let keys = match &self.screen {
            Screen::Disks => "↑↓ move · enter explore · c cleanup ~ · r refresh · q quit",
            Screen::Scanning { .. } => "esc cancel · q quit",
            Screen::Tree(_) => {
                "↑↓ move · enter open · ← back · c cleanup here · esc disks · q quit"
            }
            Screen::Clean(c) => match c.mode {
                Mode::Browse => {
                    "space mark · a mark old safe · n none · enter review · r rescan · esc disks · q quit"
                }
                _ => "enter move to trash · D delete permanently · esc cancel",
            },
        };
        f.render_widget(Paragraph::new(keys).fg(Color::DarkGray), help);

        match &mut self.screen {
            Screen::Disks => {
                let rows = self.disks.iter().map(|d| {
                    let pct = d.pct();
                    Row::new(vec![
                        Cell::from(d.mount.clone()),
                        Cell::from(d.fstype.clone()).fg(Color::DarkGray),
                        Cell::from(bar(pct, 24)),
                        Cell::from(format!("{pct:>3.0}%")).fg(rgb(level(pct).1)),
                        Cell::from(format!("{} / {}", human(d.used), human(d.total))),
                        Cell::from(format!("{} free", human(d.avail))).fg(Color::DarkGray),
                    ])
                });
                let widths = [
                    Constraint::Fill(1),
                    Constraint::Length(6),
                    Constraint::Length(24),
                    Constraint::Length(4),
                    Constraint::Length(17),
                    Constraint::Length(12),
                ];
                let table = Table::new(rows, widths)
                    .column_spacing(2)
                    .block(Block::bordered().title(" Disks ").fg(Color::Blue))
                    .row_highlight_style(Style::new().reversed());
                f.render_stateful_widget(table, main, &mut self.table);
            }
            Screen::Scanning {
                path,
                files,
                started,
                ..
            } => {
                let text = format!(
                    "Scanning {} … {} entries · {}s",
                    tilde(path),
                    files.load(Ordering::Relaxed),
                    started.elapsed().as_secs()
                );
                let [mid] = Layout::vertical([Constraint::Length(3)])
                    .flex(layout::Flex::Center)
                    .areas(main);
                f.render_widget(
                    Paragraph::new(text)
                        .centered()
                        .block(Block::bordered().fg(Color::Blue)),
                    mid,
                );
            }
            Screen::Tree(t) => {
                let location = tilde(&t.location());
                let node = t.current();
                let total = node.size.max(1);
                let items: Vec<ListItem> = node
                    .children
                    .iter()
                    .map(|c| {
                        let share = c.size as f64 * 100.0 / total as f64;
                        let mut spans = vec![Span::raw(format!("{:>8}  ", human(c.size)))];
                        spans.extend(bar(share, 20).spans);
                        spans.push(Span::raw(format!(" {share:>5.1}%  ")).fg(Color::DarkGray));
                        let name = if c.is_dir {
                            format!("{}/", c.name)
                        } else {
                            c.name.clone()
                        };
                        spans.push(if c.is_dir {
                            Span::raw(name).fg(Color::Blue).bold()
                        } else {
                            Span::raw(name)
                        });
                        ListItem::new(Line::from(spans))
                    })
                    .collect();
                let title = format!(
                    " {location} · {} · {} items ",
                    human(node.size),
                    node.children.len()
                );
                let list = List::new(items)
                    .block(Block::bordered().title(title).fg(Color::Blue))
                    .highlight_style(Style::new().reversed());
                f.render_stateful_widget(list, main, &mut t.list);
            }
            Screen::Clean(c) => draw_clean(f, main, c),
        }
    }
}

fn draw_clean(f: &mut Frame, area: Rect, c: &mut Clean) {
    let [list_area, detail] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(4)]).areas(area);
    let rows = c.items.iter().zip(&c.marked).map(|(it, &m)| {
        let mark = match (&it.blocked, m) {
            (Some(_), _) => Span::raw(" ✗ ").fg(rgb(RED)),
            (None, true) => Span::raw("[x]").fg(rgb(GREEN)).bold(),
            (None, false) => Span::raw("[ ]"),
        };
        let note = match (&it.blocked, &it.action) {
            (Some(why), _) => Span::raw(why.clone()).fg(rgb(RED)),
            (None, _) if it.recent => Span::raw("changed this week").fg(rgb(YELLOW)),
            (None, Action::Run(cmd)) => {
                Span::raw(format!("runs `{}`", cmd.join(" "))).fg(Color::DarkGray)
            }
            (None, Action::Remove) => Span::raw(""),
        };
        Row::new(vec![
            Cell::from(mark),
            Cell::from(format!("{:>8}", human(it.size))),
            Cell::from(risk_span(it.risk)),
            Cell::from(format!("{:>5}", age(it.changed))).fg(Color::DarkGray),
            Cell::from(it.kind),
            Cell::from(tilde(&it.path)).fg(Color::Blue),
            Cell::from(note),
        ])
    });
    let header = Row::new(["", "size", "risk", "age", "kind", "path", ""]).fg(Color::DarkGray);
    let widths = [
        Constraint::Length(3),
        Constraint::Length(8),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(15),
        Constraint::Fill(2),
        Constraint::Fill(1),
    ];
    let title = format!(
        " Cleanup · {} · marked {} ",
        tilde(&c.root),
        human(c.marked_size())
    );
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(Block::bordered().title(title).fg(Color::Blue))
        .row_highlight_style(Style::new().reversed());
    f.render_stateful_widget(table, list_area, &mut c.table);

    let info = match c.table.selected().and_then(|i| c.items.get(i)) {
        Some(it) => {
            let how = match it.action {
                Action::Run(cmd) => format!("runs `{}` (the tool's own cleanup)", cmd.join(" ")),
                Action::Remove => "removes the directory".into(),
            };
            format!("{}\nRegenerate: {} · Action: {how}", c.status, it.regen)
        }
        None => format!("{}\nNothing regenerable found here.", c.status),
    };
    f.render_widget(
        Paragraph::new(info)
            .wrap(Wrap { trim: true })
            .block(Block::bordered().fg(Color::DarkGray)),
        detail,
    );

    if matches!(c.mode, Mode::Browse) {
        return;
    }
    let marked: Vec<&Candidate> = c.marked().collect();
    let mut lines: Vec<Line> = marked
        .iter()
        .take(12)
        .map(|it| Line::from(format!("{:>8}  {}", human(it.size), tilde(&it.path))))
        .collect();
    if marked.len() > 12 {
        lines.push(Line::from(format!("… and {} more", marked.len() - 12)).fg(Color::DarkGray));
    }
    lines.push(Line::from(""));
    if marked.iter().any(|it| it.risk == Risk::Care) {
        lines.push(
            Line::from("Includes CARE items that may not be regenerable.")
                .fg(rgb(RED))
                .bold(),
        );
    }
    if marked.iter().any(|it| matches!(it.action, Action::Run(_))) {
        lines.push(
            Line::from("Tool caches run their own cleanup command either way.").fg(Color::DarkGray),
        );
    }
    lines.push(match c.mode {
        Mode::Run { .. } => Line::from("Working…").bold(),
        _ => Line::from(
            "enter move to trash (default, recoverable) · D delete permanently · esc cancel",
        )
        .bold(),
    });
    let title = format!(
        " Remove {} items · {} ",
        marked.len(),
        human(c.marked_size())
    );
    let h = (lines.len() as u16 + 2).min(area.height);
    let [popup] = Layout::vertical([Constraint::Length(h)])
        .flex(layout::Flex::Center)
        .areas(area);
    let [popup] = Layout::horizontal([Constraint::Percentage(80)])
        .flex(layout::Flex::Center)
        .areas(popup);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title).fg(rgb(ORANGE))),
        popup,
    );
}

fn risk_span(r: Risk) -> Span<'static> {
    match r {
        Risk::Safe => Span::raw("safe").fg(rgb(GREEN)),
        Risk::Slow => Span::raw("slow").fg(rgb(YELLOW)),
        Risk::Care => Span::raw("care").fg(rgb(RED)).bold(),
    }
}

fn age(t: SystemTime) -> String {
    let days = t.elapsed().map_or(0, |d| d.as_secs() / 86_400);
    match days {
        0 => "today".into(),
        d if d < 60 => format!("{d}d"),
        d if d < 730 => format!("{}mo", d / 30),
        d => format!("{}y", d / 365),
    }
}

fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if home() != Path::new("/") => format!("~/{}", rest.display())
            .trim_end_matches('/')
            .to_string(),
        _ => p.display().to_string(),
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

fn bar(pct: f64, width: usize) -> Line<'static> {
    let filled = ((pct / 100.0 * width as f64).round() as usize).min(width);
    Line::from(vec![
        Span::raw("█".repeat(filled)).fg(rgb(level(pct).1)),
        Span::raw("░".repeat(width - filled)).fg(rgb(TRACK)),
    ])
}

mod clean;
mod disks;
mod scan;
mod tui;

use disks::{Disk, human};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

const HELP: &str = "\
Waybar widget, terminal dashboard and TUI explorer for disk usage.

Usage: alpheus [OPTIONS]
       alpheus tui [PATH]
       alpheus clean [PATH]

Output modes:
  default       Waybar JSON ({text, tooltip, class, percentage}) when piped,
                pretty dashboard when stdout is a TTY
  --json        force JSON
  --pretty      force the pretty dashboard
  --watch N     pretty dashboard refreshed every N seconds (Ctrl-C to exit)

Options:
  --mount PATH  filesystem shown in the bar text (default: /)
  -h, --help    print this help
  -V, --version print the version

TUI:
  tui           disk list; Enter scans the selected mount
  tui PATH      scan PATH directly (ncdu-style explorer)
  clean [PATH]  find regenerable build artifacts and caches under PATH
                (default: ~) and remove the ones you mark, after review
";

pub const GREEN: (u8, u8, u8) = (152, 195, 121);
pub const YELLOW: (u8, u8, u8) = (229, 192, 123);
pub const ORANGE: (u8, u8, u8) = (209, 154, 102);
pub const RED: (u8, u8, u8) = (224, 108, 117);
const BLUE: (u8, u8, u8) = (97, 175, 239);
const GRAY: (u8, u8, u8) = (92, 99, 112);
const TEXT: (u8, u8, u8) = (171, 178, 191);
pub const TRACK: (u8, u8, u8) = (62, 68, 81);

pub fn level(pct: f64) -> (&'static str, (u8, u8, u8)) {
    match pct {
        p if p < 50.0 => ("low", GREEN),
        p if p < 75.0 => ("mid", YELLOW),
        p if p < 90.0 => ("high", ORANGE),
        _ => ("critical", RED),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("alpheus: {e}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut mount = "/".to_string();
    let mut json = None;
    let mut watch = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "tui" => {
                let start = it.next().map_or(Ok(tui::Start::Disks), |p| {
                    absolute(p).map(tui::Start::Explore)
                })?;
                return tui::run(start).map_err(|e| e.to_string());
            }
            "clean" => {
                let root = absolute(
                    it.next()
                        .map_or(
                            std::env::var("HOME").unwrap_or_else(|_| "/".into()),
                            String::clone,
                        )
                        .as_str(),
                )?;
                return tui::run(tui::Start::Clean(root)).map_err(|e| e.to_string());
            }
            "--json" => json = Some(true),
            "--pretty" => json = Some(false),
            "--mount" => mount = it.next().ok_or("--mount needs a path")?.clone(),
            "--watch" => {
                let secs: u64 = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|&s| s > 0)
                    .ok_or("--watch needs a positive number of seconds")?;
                watch = Some(secs);
            }
            "-V" | "--version" => {
                println!("alpheus {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}` (see --help)")),
        }
    }

    if let Some(secs) = watch {
        loop {
            print!("\x1b[2J\x1b[H{}", pretty(&disks::list()));
            std::thread::sleep(Duration::from_secs(secs));
        }
    }
    if json.unwrap_or(!std::io::stdout().is_terminal()) {
        let disk = disks::stat(&mount, "").ok_or_else(|| format!("cannot stat {mount}"))?;
        println!("{}", waybar(&disk, &disks::list()));
    } else {
        print!("{}", pretty(&disks::list()));
    }
    Ok(())
}

fn absolute(path: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))
}

fn waybar(bar: &Disk, all: &[Disk]) -> serde_json::Value {
    let pct = bar.pct();
    let tooltip = all
        .iter()
        .map(|d| {
            format!(
                "{:<14} {:>3.0}%  {} / {} · {} free",
                d.mount,
                d.pct(),
                human(d.used),
                human(d.total),
                human(d.avail)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::json!({
        "text": format!("{pct:.0}%"),
        "tooltip": tooltip,
        "class": level(pct).0,
        "percentage": pct.round() as u64,
    })
}

const WIDTH: usize = 58;

fn paint(text: &str, (r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[0m")
}

/// One boxed line; padding is computed from the uncolored text so ANSI codes don't skew the border.
fn row(segments: &[(&str, (u8, u8, u8))]) -> String {
    let len: usize = segments.iter().map(|(t, _)| t.chars().count()).sum();
    let body: String = segments.iter().map(|(t, c)| paint(t, *c)).collect();
    let border = paint("│", BLUE);
    format!(
        "{border} {body}{} {border}\n",
        " ".repeat(WIDTH.saturating_sub(len))
    )
}

fn pretty(all: &[Disk]) -> String {
    let title = "Disk usage";
    let pad = (WIDTH - title.len()) / 2;
    let rule = "─".repeat(WIDTH);
    let mut out = paint(&format!("╭{}╮\n", "─".repeat(WIDTH + 2)), BLUE);
    out += &row(&[(&format!("{}{title}", " ".repeat(pad)), BLUE)]);
    out += &row(&[(&rule, GRAY)]);
    for d in all {
        let pct = d.pct();
        let color = level(pct).1;
        let filled = ((pct / 5.0).round() as usize).min(20);
        out += &row(&[]);
        out += &row(&[
            (&format!("  {}", d.mount), TEXT),
            (&format!("  {}", d.fstype), GRAY),
        ]);
        out += &row(&[
            ("  ", TEXT),
            (&"█".repeat(filled), color),
            (&"░".repeat(20 - filled), TRACK),
            (&format!("  {pct:.0}%"), color),
        ]);
        out += &row(&[(
            &format!(
                "  {} of {} used · {} free",
                human(d.used),
                human(d.total),
                human(d.avail)
            ),
            GRAY,
        )]);
    }
    out += &row(&[]);
    out + &paint(&format!("╰{}╯\n", "─".repeat(WIDTH + 2)), BLUE)
}

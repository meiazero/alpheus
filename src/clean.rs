//! Finds regenerable directories (build artifacts, dependency installs, tool caches) and removes them with re-checks.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, PartialEq)]
pub enum Risk {
    /// Derived data, regenerated for free.
    Safe,
    /// Regenerable, but costs a rebuild or re-download.
    Slow,
    /// May hold data that never comes back (venvs with manual installs, the trash).
    Care,
}

pub enum Action {
    Remove,
    /// The owning tool's own cleanup command; preferred because the tool knows its locks and layout.
    Run(&'static [&'static str]),
}

pub struct Candidate {
    pub kind: &'static str,
    pub path: PathBuf,
    pub size: u64,
    /// Newest directory mtime inside the candidate: directories change when a build or install writes into them.
    pub changed: SystemTime,
    pub risk: Risk,
    pub regen: &'static str,
    pub action: Action,
    pub blocked: Option<String>,
    /// Project artifact touched in the last week, so probably part of active work.
    pub recent: bool,
    /// Inside a git work tree, so removal must first prove git tracks nothing in it.
    in_repo: bool,
    project: bool,
    id: (u64, u64),
}

pub const RECENT: Duration = Duration::from_secs(7 * 24 * 3600);

struct Rule {
    dir: &'static str,
    /// The candidate counts only when the parent holds one of these; empty means the name alone is proof.
    markers: &'static [&'static str],
    kind: &'static str,
    risk: Risk,
    regen: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        dir: "target",
        markers: &["Cargo.toml"],
        kind: "Rust build",
        risk: Risk::Slow,
        regen: "cargo build",
    },
    Rule {
        dir: "node_modules",
        // A lockfile, not just package.json: Electron apps ship package.json + node_modules that no install can rebuild.
        markers: &[
            "package-lock.json",
            "npm-shrinkwrap.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lock",
            "bun.lockb",
        ],
        kind: "Node deps",
        risk: Risk::Slow,
        regen: "npm / pnpm / yarn install",
    },
    Rule {
        dir: ".next",
        markers: &["package.json"],
        kind: "Next.js build",
        risk: Risk::Safe,
        regen: "next build",
    },
    Rule {
        dir: ".turbo",
        markers: &["package.json"],
        kind: "Turbo cache",
        risk: Risk::Safe,
        regen: "next turbo run",
    },
    Rule {
        dir: "__pycache__",
        markers: &[],
        kind: "Python bytecode",
        risk: Risk::Safe,
        regen: "recompiled on import",
    },
    Rule {
        dir: ".pytest_cache",
        markers: &[],
        kind: "pytest cache",
        risk: Risk::Safe,
        regen: "next pytest run",
    },
    Rule {
        dir: ".mypy_cache",
        markers: &[],
        kind: "mypy cache",
        risk: Risk::Safe,
        regen: "next mypy run",
    },
    Rule {
        dir: ".ruff_cache",
        markers: &[],
        kind: "ruff cache",
        risk: Risk::Safe,
        regen: "next ruff run",
    },
    Rule {
        dir: ".venv",
        markers: &["pyproject.toml", "requirements.txt", "uv.lock", "Pipfile"],
        kind: "Python venv",
        risk: Risk::Care,
        regen: "uv sync / pip install -r requirements.txt (manual installs are lost)",
    },
];

/// `root` must be canonical; $HOME and cache paths are canonicalized here so symlinked homes compare equal.
pub fn find(root: &Path, progress: &AtomicU64) -> Vec<Candidate> {
    let mut out = Vec::new();
    let home = env_dir("HOME");
    if let Ok(meta) = fs::symlink_metadata(root) {
        let in_repo = root.ancestors().any(|a| a.join(".git").exists());
        walk(
            root,
            meta.dev(),
            in_repo,
            home.as_deref(),
            &mut out,
            progress,
        );
    }
    for (kind, path, risk, regen, action) in home.map(|h| known_caches(&h)).unwrap_or_default() {
        if path.starts_with(root)
            && let Some(c) = candidate(kind, &path, risk, regen, action, false, false, progress)
        {
            out.push(c);
        }
    }
    out.sort_unstable_by_key(|c| std::cmp::Reverse(c.size));
    out
}

/// Recurses into every subdirectory except `.git`, `node_modules` and the dot-dirs directly in $HOME: those hold app data and
/// installed tools (VS Code extensions, mise runtimes) whose `node_modules` and `__pycache__` are not disposable.
fn walk(
    dir: &Path,
    dev: u64,
    in_repo: bool,
    home: Option<&Path>,
    out: &mut Vec<Candidate>,
    progress: &AtomicU64,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    let has = |name: &str| entries.iter().any(|e| e.file_name() == name);
    if has("pyvenv.cfg") {
        return; // Inside an unmatched virtualenv: its __pycache__ dirs belong to installed packages.
    }
    let in_repo = in_repo || has(".git");
    for e in &entries {
        progress.fetch_add(1, Ordering::Relaxed);
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_dir() || meta.dev() != dev {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if let Some(rule) = RULES
            .iter()
            .find(|r| r.dir == name && (r.markers.is_empty() || r.markers.iter().any(|m| has(m))))
        {
            if let Some(c) = candidate(
                rule.kind,
                &e.path(),
                rule.risk,
                rule.regen,
                Action::Remove,
                true,
                in_repo,
                progress,
            ) {
                out.push(c);
            }
            continue;
        }
        let app_data = name.starts_with('.') && home == Some(dir);
        if !app_data && name != ".git" && name != "node_modules" {
            walk(&e.path(), dev, in_repo, home, out, progress);
        }
    }
}

fn env_dir(var: &str) -> Option<PathBuf> {
    let raw = PathBuf::from(std::env::var_os(var)?);
    Some(fs::canonicalize(&raw).unwrap_or(raw))
}

fn known_caches(home: &Path) -> Vec<(&'static str, PathBuf, Risk, &'static str, Action)> {
    let cache = env_dir("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
    let cargo = env_dir("CARGO_HOME").unwrap_or_else(|| home.join(".cargo"));
    // A cache whose tool is no longer installed is orphaned, so removing the directory directly is the only option.
    let tool = |cmd: &'static [&'static str]| {
        if on_path(cmd[0]) {
            Action::Run(cmd)
        } else {
            Action::Remove
        }
    };
    let redownload = "re-downloaded on next install";
    vec![
        (
            "Thumbnails",
            cache.join("thumbnails"),
            Risk::Safe,
            "file manager regenerates",
            Action::Remove,
        ),
        (
            "npm cache",
            home.join(".npm/_cacache"),
            Risk::Slow,
            redownload,
            tool(&["npm", "cache", "clean", "--force"]),
        ),
        (
            "pip cache",
            cache.join("pip"),
            Risk::Slow,
            redownload,
            tool(&["pip", "cache", "purge"]),
        ),
        (
            "uv cache",
            cache.join("uv"),
            Risk::Slow,
            redownload,
            tool(&["uv", "cache", "clean"]),
        ),
        (
            "pnpm store",
            home.join(".local/share/pnpm/store"),
            Risk::Slow,
            redownload,
            tool(&["pnpm", "store", "prune"]),
        ),
        (
            "Go build cache",
            cache.join("go-build"),
            Risk::Slow,
            "rebuilt by go build",
            tool(&["go", "clean", "-cache"]),
        ),
        (
            "Cargo downloads",
            cargo.join("registry/cache"),
            Risk::Slow,
            "re-downloaded by cargo",
            Action::Remove,
        ),
        (
            "Cargo sources",
            cargo.join("registry/src"),
            Risk::Slow,
            "re-extracted by cargo",
            Action::Remove,
        ),
        (
            "Cargo git deps",
            cargo.join("git/checkouts"),
            Risk::Slow,
            "re-cloned by cargo",
            Action::Remove,
        ),
        (
            "Trash",
            home.join(".local/share/Trash"),
            Risk::Care,
            "NOT regenerable: files you deleted",
            tool(&["gio", "trash", "--empty"]),
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    kind: &'static str,
    path: &Path,
    risk: Risk,
    regen: &'static str,
    action: Action,
    project: bool,
    in_repo: bool,
    progress: &AtomicU64,
) -> Option<Candidate> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let m = measure(path, meta.dev(), progress);
    if m.size == 0 {
        return None;
    }
    let blocked = if meta.uid() != unsafe { libc::getuid() } {
        Some("owned by another user".into())
    } else {
        unsafe_reason(path, &m, project, in_repo).map(String::from)
    };
    let recent = project && m.newest.elapsed().is_ok_and(|age| age < RECENT);
    Some(Candidate {
        kind,
        path: path.to_path_buf(),
        size: m.size,
        changed: m.newest,
        risk,
        regen,
        action,
        blocked,
        recent,
        in_repo,
        project,
        id: (meta.dev(), meta.ino()),
    })
}

struct Measure {
    size: u64,
    newest: SystemTime,
    /// A `.git` file or directory: nested repo, worktree or submodule.
    git: bool,
    /// Another filesystem is mounted inside; `remove_dir_all` would descend into it.
    mount: bool,
}

/// Allocated bytes like `du -x`, counting a hard-linked file only when all its links are inside `path`,
/// so pnpm `node_modules` (hard links into the store) report the space removal actually frees.
fn measure(path: &Path, dev: u64, progress: &AtomicU64) -> Measure {
    let mut links: HashMap<(u64, u64), (u64, u64)> = HashMap::new();
    let mut stack = vec![path.to_path_buf()];
    let mut m = Measure {
        size: 0,
        newest: SystemTime::UNIX_EPOCH,
        git: false,
        mount: false,
    };
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
            progress.fetch_add(1, Ordering::Relaxed);
            let Ok(meta) = e.metadata() else { continue };
            m.git |= e.file_name() == ".git";
            if meta.is_dir() {
                if meta.dev() != dev {
                    m.mount = true;
                    continue;
                }
                m.newest = m.newest.max(meta.modified().unwrap_or(m.newest));
                stack.push(e.path());
            } else if meta.nlink() > 1 {
                let seen = links
                    .entry((meta.dev(), meta.ino()))
                    .or_insert((0, meta.nlink()));
                seen.0 += 1;
                if seen.0 < seen.1 {
                    continue;
                }
            }
            m.size += meta.blocks() * 512;
        }
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        m.size += meta.blocks() * 512;
        m.newest = m.newest.max(meta.modified().unwrap_or(m.newest));
    }
    m
}

/// Checks shared by the scan and the re-check right before removal.
fn unsafe_reason(path: &Path, m: &Measure, project: bool, in_repo: bool) -> Option<&'static str> {
    if m.mount {
        Some("contains a mount point")
    } else if m.git && project {
        // Tool caches (uv, cargo) legitimately hold git checkouts; only project artifacts may not hide a repo.
        Some("contains a git repository")
    } else if in_repo && !git_tracks_nothing(path) {
        Some("git-tracked files, or git could not tell")
    } else {
        None
    }
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// Only called inside a work tree, so any git failure (missing binary, "dubious ownership") counts as "maybe tracked".
fn git_tracks_nothing(dir: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "-z", "--", "."])
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.is_empty())
}

/// Re-checks the path right before acting, so a directory swapped for a symlink or a new checkout since the scan is refused.
pub fn execute(c: &Candidate, permanent: bool) -> Result<(), String> {
    if let Action::Run(cmd) = c.action {
        let out = Command::new(cmd[0])
            .args(&cmd[1..])
            .output()
            .map_err(|e| format!("{}: {e}", cmd[0]))?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().last().unwrap_or("command failed").to_string());
    }
    let meta = fs::symlink_metadata(&c.path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || (meta.dev(), meta.ino()) != c.id {
        return Err("changed since scan; rescan first".into());
    }
    if meta.uid() != unsafe { libc::getuid() } {
        return Err("owned by another user".into());
    }
    let m = measure(&c.path, meta.dev(), &AtomicU64::new(0));
    if let Some(why) = unsafe_reason(&c.path, &m, c.project, c.in_repo) {
        return Err(why.into());
    }
    if permanent {
        fs::remove_dir_all(&c.path).map_err(|e| e.to_string())
    } else {
        let out = Command::new("gio")
            .arg("trash")
            .arg(&c.path)
            .output()
            .map_err(|e| format!("gio: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_marked_artifacts_and_refuses_swapped_paths() {
        let root = std::env::temp_dir().join(format!("alpheus-clean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for d in [
            "rs/target/debug",
            "lone/target",
            "js/node_modules/a/node_modules/b",
            "py/pkg/__pycache__",
            "env/lib/__pycache__",
            ".tools/cli/target",
        ] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::write(root.join("rs/Cargo.toml"), "").unwrap();
        fs::write(root.join("rs/target/debug/bin"), vec![0u8; 8192]).unwrap();
        fs::write(root.join("lone/target/x"), "x").unwrap();
        fs::write(root.join("js/package.json"), "{}").unwrap();
        fs::write(root.join("js/package-lock.json"), "{}").unwrap();
        // Shipped app: package.json without a lockfile must not be offered.
        fs::create_dir_all(root.join("app/resources/node_modules/dep")).unwrap();
        fs::write(root.join("app/resources/package.json"), "{}").unwrap();
        fs::write(root.join("app/resources/node_modules/dep/i.js"), "x").unwrap();
        // A worktree inside target/ has a `.git` file, not a directory.
        fs::create_dir_all(root.join("wt/target/tree")).unwrap();
        fs::write(root.join("wt/Cargo.toml"), "").unwrap();
        fs::write(root.join("wt/target/tree/.git"), "gitdir: /elsewhere").unwrap();
        fs::write(root.join("js/node_modules/a/node_modules/b/i.js"), "x").unwrap();
        fs::write(root.join("py/pkg/__pycache__/m.pyc"), "x").unwrap();
        fs::write(root.join("env/pyvenv.cfg"), "").unwrap();
        fs::write(root.join(".tools/cli/Cargo.toml"), "").unwrap();
        fs::write(root.join(".tools/cli/target/x"), "x").unwrap();
        fs::write(root.join("env/lib/__pycache__/m.pyc"), "x").unwrap();

        let found = find(&root, &AtomicU64::new(0));
        let mut paths: Vec<_> = found
            .iter()
            .map(|c| c.path.strip_prefix(&root).unwrap().to_path_buf())
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            [
                ".tools/cli/target",
                "js/node_modules",
                "py/pkg/__pycache__",
                "rs/target",
                "wt/target"
            ]
            .map(PathBuf::from)
        );
        let blocked = |p: &str| {
            found
                .iter()
                .find(|c| c.path.ends_with(p))
                .unwrap()
                .blocked
                .clone()
        };
        assert_eq!(
            blocked("wt/target").as_deref(),
            Some("contains a git repository")
        );
        assert!(
            found
                .iter()
                .all(|c| c.recent && (c.path.ends_with("wt/target") || c.blocked.is_none()))
        );

        // Swap the scanned dir for a symlink to something precious: execute must refuse.
        let target = found
            .iter()
            .find(|c| c.path.ends_with("rs/target"))
            .unwrap();
        fs::remove_dir_all(&target.path).unwrap();
        std::os::unix::fs::symlink(root.join("lone"), &target.path).unwrap();
        assert!(execute(target, true).is_err());
        assert!(root.join("lone/target/x").exists());

        let pyc = found.iter().find(|c| c.kind == "Python bytecode").unwrap();
        execute(pyc, true).unwrap();
        assert!(!pyc.path.exists() && root.join("py/pkg").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}

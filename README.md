# alpheus

Disk usage for Linux in three forms: a Waybar widget, a terminal dashboard, and a TUI that
explores directories and removes regenerable files. Named after the river Heracles turned through
the Augean stables to clear them in a day.

- Lists block-device filesystems with `df`-style usage; the bar text tracks one mount, `/` by
  default.
- `alpheus tui` browses any directory ncdu-style: allocated size, one filesystem, hard links
  counted once.
- `alpheus clean` finds build artifacts (`target/`, `node_modules/`, `.venv/`, `__pycache__/`,
  ...) and tool caches (npm, pip, uv, pnpm, Go, Cargo, thumbnails), then removes only what you
  mark, after a review screen.
- Removal goes to the trash by default; permanent delete needs `D`. Tool caches run the tool's
  own cleanup command (`uv cache clean`, `npm cache clean --force`).
- Never removes a directory with git-tracked files, a nested git repo, or another user's files,
  and re-checks each path right before acting. Bulk select (`a`) skips projects changed in the
  last 7 days and anything marked `care`.
- Linux only: reads `/proc/mounts` and uses `gio trash`.

License: [MIT](LICENSE).

## Install

On Linux x86_64, this installs the latest release to `~/.local/bin` after checking it against the
release's `sha256.sum`:

```bash
curl -fsSL https://raw.githubusercontent.com/meiazero/alpheus/main/install.sh | sh
```

`ALPHEUS_VERSION=v0.1.0` pins a release and `ALPHEUS_INSTALL_DIR` changes the target directory.
From source: `cargo install --git https://github.com/meiazero/alpheus`.

## Quick start

```bash
alpheus                  # dashboard on a TTY, Waybar JSON when piped
alpheus --watch 5        # dashboard refreshed every 5 s
alpheus tui ~/Projects   # explore a directory
alpheus clean            # regenerable files under ~; space marks, enter reviews
```

`alpheus --help` lists every flag.

In the explorer, `c` scans the current directory for cleanup candidates. `Esc` from cleanup
returns to the same explorer, including its selection and scroll position, without scanning
the disk again. Cancelling the cleanup scan also returns there; `r` explicitly rescans only
the cleanup root. When started with `alpheus clean`, `Esc` still returns to the disk list.

Successful directory removals update the retained tree and ancestor sizes in memory. After
cleanup, the explorer is labelled a snapshot: tool commands may leave cache contents, trash
may remain on the same disk, and hard-link accounting or external changes may be stale.
Disk usage is refreshed from filesystem statistics rather than inferred from removed rows.

## Waybar

```json
"custom/disk": {
  "exec": "alpheus --mount /",
  "return-type": "json",
  "interval": 60,
  "on-click": "<TERMINAL> -e alpheus tui"
}
```

The JSON `class` is `low`, `mid`, `high`, or `critical` (below 50, 75, 90 %, and above), for
styling in `style.css`.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- `rust-toolchain.toml` pins the toolchain; rustup installs it on the first `cargo` call.
- CI runs the commands above, a release build, and `bashka --check` on `install.sh`.
- Releases: merging the release-please PR tags the version, and `release.yml` builds the static
  musl binary with cargo-dist, attests it, and publishes it with `sha256.sum`.

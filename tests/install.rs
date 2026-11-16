//! Pipes `install.sh` into `sh`, as `curl -fsSL .../install.sh | sh` does, against a local server laid
//! out like GitHub's release download URLs.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TARGET: &str = "alpheus-x86_64-unknown-linux-musl";

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("alpheus-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `download/vX.Y.Z/` and `latest/download/` hold the archive of the binary under test and its
/// `sha256.sum`, optionally tampered so the checksum no longer matches.
fn release(label: &str, tamper: bool) -> PathBuf {
    let root = temp_dir(label);
    let staging = root.join(TARGET);
    fs::create_dir_all(&staging).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_alpheus"), staging.join("alpheus")).unwrap();
    let tag = root.join(format!("download/v{VERSION}"));
    fs::create_dir_all(&tag).unwrap();
    let asset = format!("{TARGET}.tar.xz");
    let tar = Command::new("tar")
        .env("XZ_OPT", "-0")
        .arg("-cJf")
        .arg(tag.join(&asset))
        .arg("-C")
        .arg(&root)
        .arg(TARGET)
        .status()
        .unwrap();
    assert!(tar.success());
    let digest = Command::new("sha256sum")
        .arg(tag.join(&asset))
        .output()
        .unwrap();
    let mut hash = String::from_utf8(digest.stdout).unwrap()[..64].to_string();
    if tamper {
        hash = hash.replace(|c: char| c != '0', "0");
    }
    fs::write(tag.join("sha256.sum"), format!("{hash}  {asset}\n")).unwrap();
    fs::create_dir_all(root.join("latest")).unwrap();
    symlink(&tag, root.join("latest/download")).unwrap();
    root
}

/// Minimal static file server on loopback: the only origin install.sh accepts over plain HTTP.
fn serve(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut header = String::new();
            while reader.read_line(&mut header).unwrap() > 2 {
                header.clear();
            }
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            match fs::read(root.join(path.trim_start_matches('/'))) {
                Ok(body) => {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body);
                }
                Err(_) => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        }
    });
    format!("http://{address}")
}

fn install(base: &str, home: &Path, path: &str, version: Option<&str>) -> Output {
    let mut command = Command::new("sh");
    command
        .env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .env("ALPHEUS_RELEASE_BASE", base)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(v) = version {
        command.env("ALPHEUS_VERSION", v);
    }
    let mut child = command.spawn().unwrap();
    let script = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh")).unwrap();
    child.stdin.take().unwrap().write_all(&script).unwrap();
    child.wait_with_output().unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn install_puts_the_latest_release_on_the_user_path_and_upgrades_in_place() {
    let root = release("release-ok", false);
    let base = serve(root.clone());
    let home = temp_dir("home-ok");
    let system_path = std::env::var("PATH").unwrap();
    let bin = home.join(".local/bin/alpheus");

    let first = install(&base, &home, &system_path, None);
    assert!(first.status.success(), "{}", text(&first));
    assert_eq!(
        fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let version = Command::new(&bin).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        format!("alpheus {VERSION}")
    );
    assert!(
        text(&first).contains("PATH"),
        "~/.local/bin is not on PATH, so the user is told: {}",
        text(&first)
    );

    let on_path = format!("{}:{system_path}", bin.parent().unwrap().display());
    let second = install(&base, &home, &on_path, Some(VERSION));
    assert!(second.status.success(), "{}", text(&second));
    assert!(!text(&second).contains("PATH"), "{}", text(&second));

    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn install_refuses_a_tampered_or_missing_release_and_installs_nothing() {
    let root = release("release-bad", true);
    let base = serve(root.clone());
    let home = temp_dir("home-bad");
    let path = std::env::var("PATH").unwrap();
    let bin = home.join(".local/bin/alpheus");

    let tampered = install(&base, &home, &path, None);
    assert!(!tampered.status.success());
    assert!(
        text(&tampered).contains("checksum mismatch"),
        "{}",
        text(&tampered)
    );
    assert!(!bin.exists());

    let missing = install(&base, &home, &path, Some("v0.0.1"));
    assert!(!missing.status.success());
    assert!(text(&missing).contains("v0.0.1"), "{}", text(&missing));
    assert!(!bin.exists());

    // Userinfo makes the URL start like the loopback origin while naming another host: no plain HTTP.
    let disguised = base.replacen("http://", "http://127.0.0.1:1@", 1);
    let output = install(&disguised, &home, &path, None);
    assert!(
        text(&output).contains("cannot download"),
        "{}",
        text(&output)
    );
    assert!(!bin.exists());

    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(root).unwrap();
}

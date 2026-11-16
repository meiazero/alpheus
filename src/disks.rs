use std::ffi::CString;

pub struct Disk {
    pub mount: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

impl Disk {
    /// Same formula as `df`: reserved root blocks count as neither used nor available.
    pub fn pct(&self) -> f64 {
        let usable = self.used + self.avail;
        if usable == 0 {
            0.0
        } else {
            self.used as f64 * 100.0 / usable as f64
        }
    }
}

/// Block-device mounts only; the first mount of each device wins so btrfs subvolumes and bind mounts don't repeat.
pub fn list() -> Vec<Disk> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut seen = Vec::new();
    let mut disks = Vec::new();
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        let (Some(dev), Some(mount), Some(fstype)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if !dev.starts_with('/') || fstype == "squashfs" || seen.contains(&dev) {
            continue;
        }
        seen.push(dev);
        let mount = unescape(mount);
        if let Some(disk) = stat(&mount, fstype) {
            disks.push(disk);
        }
    }
    disks
}

// statvfs field widths differ between targets (c_ulong vs u64), so the casts stay for 32-bit builds.
#[allow(clippy::unnecessary_cast)]
pub fn stat(mount: &str, fstype: &str) -> Option<Disk> {
    let path = CString::new(mount).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as u64;
    Some(Disk {
        mount: mount.to_string(),
        fstype: fstype.to_string(),
        total: s.f_blocks as u64 * frsize,
        used: s.f_blocks.saturating_sub(s.f_bfree) as u64 * frsize,
        avail: s.f_bavail as u64 * frsize,
    })
}

/// `/proc/mounts` writes space, tab, newline and backslash as 3-digit octal escapes (`\040`).
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = b
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match octal {
            Some(d) if b[i] == b'\\' => {
                out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                i += 4;
            }
            _ => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes}B")
    } else {
        format!("{v:.1}{}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_mount_paths() {
        assert_eq!(unescape(r"/media/my\040disk"), "/media/my disk");
        assert_eq!(unescape(r"/a\134b"), r"/a\b");
        assert_eq!(unescape("/plain"), "/plain");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human(512), "512B");
        assert_eq!(human(1536), "1.5K");
        assert_eq!(human(5 * 1024 * 1024 * 1024), "5.0G");
    }
}

use std::collections::HashSet;
use std::ffi::CString;
use std::fs::{self, Metadata};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Node {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<Node>,
    /// Bytes of children left out of `children`: empty entries and ones holding nothing the user can remove.
    pub hidden: u64,
}

/// Walks `path` like `du -x`: allocated blocks, one filesystem, hard links counted once.
/// Only entries the user could act on are kept, which also keeps `/usr`-sized trees out of memory.
/// ponytail: single-threaded; parallelize per top-level child if multi-million-file disks are too slow.
pub fn scan(path: &Path, files: &AtomicU64) -> Node {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Node {
            name: path.display().to_string(),
            size: 0,
            is_dir: false,
            children: vec![],
            hidden: 0,
        };
    };
    let uid = unsafe { libc::getuid() };
    let (mut root, _) = walk(path, &meta, meta.dev(), uid, &mut HashSet::new(), files);
    root.name = path.display().to_string();
    root
}

/// Write and search permission on `dir`, which `unlink`/`rmdir` of its entries need. `access` also honors ACLs and groups.
fn writable(dir: &Path) -> bool {
    CString::new(dir.as_os_str().as_bytes())
        .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } == 0)
}

/// Returns the node and whether anything inside it is removable by `uid`.
fn walk(
    path: &Path,
    meta: &Metadata,
    dev: u64,
    uid: u32,
    seen: &mut HashSet<(u64, u64)>,
    files: &AtomicU64,
) -> (Node, bool) {
    files.fetch_add(1, Ordering::Relaxed);
    let hard_link_repeat =
        meta.nlink() > 1 && !meta.is_dir() && !seen.insert((meta.dev(), meta.ino()));
    let mut node = Node {
        name: path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        size: if hard_link_repeat {
            0
        } else {
            meta.blocks() * 512
        },
        is_dir: meta.is_dir(),
        children: vec![],
        hidden: 0,
    };
    let mut removable_inside = false;
    if node.is_dir && meta.dev() == dev {
        let can_unlink = writable(path);
        // Sticky directories like /tmp only let owners remove their own entries.
        let sticky = meta.mode() & 0o1000 != 0;
        for entry in fs::read_dir(path).into_iter().flatten().flatten() {
            // DirEntry::metadata does not follow symlinks, so links never cause cycles.
            let Ok(m) = entry.metadata() else { continue };
            let removable = can_unlink && (!sticky || m.uid() == uid);
            let (child, inside) = walk(&entry.path(), &m, dev, uid, seen, files);
            node.size += child.size;
            if child.size > 0 && (removable || inside) {
                removable_inside = true;
                node.children.push(child);
            } else {
                node.hidden += child.size;
            }
        }
        node.children
            .sort_unstable_by_key(|c| std::cmp::Reverse(c.size));
    }
    (node, removable_inside)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sums_sizes_and_counts_hard_links_once() {
        let dir = std::env::temp_dir().join(format!("alpheus-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/big"), vec![1u8; 64 * 1024]).unwrap();
        fs::write(dir.join("small"), b"x").unwrap();
        fs::hard_link(dir.join("sub/big"), dir.join("big-link")).unwrap();
        // Symlinks take no blocks; a read-only dir's contents cannot be unlinked by its owner.
        std::os::unix::fs::symlink("sub", dir.join("link")).unwrap();
        fs::create_dir(dir.join("ro")).unwrap();
        fs::write(dir.join("ro/locked"), vec![1u8; 8192]).unwrap();
        fs::set_permissions(dir.join("ro"), fs::Permissions::from_mode(0o555)).unwrap();

        let root = scan(&dir, &AtomicU64::new(0));
        let big = fs::metadata(dir.join("sub/big")).unwrap().blocks() * 512;
        let sub = root.children.iter().find(|c| c.name == "sub").unwrap();
        // The link walked second counts 0 bytes and is hidden like any empty entry.
        let link_size = root
            .children
            .iter()
            .find(|c| c.name == "big-link")
            .map_or(0, |c| c.size);

        let sub_self = fs::metadata(dir.join("sub")).unwrap().blocks() * 512;
        assert!(sub.is_dir);
        assert_eq!(
            sub.size - sub_self + link_size,
            big,
            "the two hard links count once in total"
        );
        assert!(
            root.children.windows(2).all(|w| w[0].size >= w[1].size),
            "children sorted by size"
        );
        let names: Vec<_> = root.children.iter().map(|c| c.name.as_str()).collect();
        assert!(!names.contains(&"link"), "0B entries are hidden: {names:?}");
        let ro = root.children.iter().find(|c| c.name == "ro").unwrap();
        // Root may unlink anywhere, so only an unprivileged run can see the file hidden.
        if unsafe { libc::getuid() } != 0 {
            assert!(
                ro.children.is_empty() && ro.hidden > 0,
                "nothing removable inside ro/"
            );
        }
        assert_eq!(
            root.size,
            fs::metadata(&dir).unwrap().blocks() * 512
                + root.children.iter().map(|c| c.size).sum::<u64>()
                + root.hidden
        );
        fs::set_permissions(dir.join("ro"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Node {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<Node>,
}

/// Walks `path` like `du -x`: allocated blocks, one filesystem, hard links counted once.
/// ponytail: single-threaded and keeps every entry in memory; fine for home dirs, slow on multi-million-file disks — parallelize per top-level child if that bites.
pub fn scan(path: &Path, files: &AtomicU64) -> Node {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Node {
            name: path.display().to_string(),
            size: 0,
            is_dir: false,
            children: vec![],
        };
    };
    let mut root = walk(path, &meta, meta.dev(), &mut HashSet::new(), files);
    root.name = path.display().to_string();
    root
}

fn walk(
    path: &Path,
    meta: &Metadata,
    dev: u64,
    seen: &mut HashSet<(u64, u64)>,
    files: &AtomicU64,
) -> Node {
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
    };
    if node.is_dir && meta.dev() == dev {
        for entry in fs::read_dir(path).into_iter().flatten().flatten() {
            // DirEntry::metadata does not follow symlinks, so links never cause cycles.
            let Ok(m) = entry.metadata() else { continue };
            let child = walk(&entry.path(), &m, dev, seen, files);
            node.size += child.size;
            node.children.push(child);
        }
        node.children
            .sort_unstable_by_key(|c| std::cmp::Reverse(c.size));
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_sizes_and_counts_hard_links_once() {
        let dir = std::env::temp_dir().join(format!("alpheus-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/big"), vec![1u8; 64 * 1024]).unwrap();
        fs::write(dir.join("small"), b"x").unwrap();
        fs::hard_link(dir.join("sub/big"), dir.join("big-link")).unwrap();

        let root = scan(&dir, &AtomicU64::new(0));
        let big = fs::metadata(dir.join("sub/big")).unwrap().blocks() * 512;
        let sub = root.children.iter().find(|c| c.name == "sub").unwrap();
        let link = root.children.iter().find(|c| c.name == "big-link").unwrap();

        let sub_self = fs::metadata(dir.join("sub")).unwrap().blocks() * 512;
        assert!(sub.is_dir);
        assert_eq!(
            sub.size - sub_self + link.size,
            big,
            "the two hard links count once in total"
        );
        assert!(
            root.children.windows(2).all(|w| w[0].size >= w[1].size),
            "children sorted by size"
        );
        assert_eq!(
            root.size,
            fs::metadata(&dir).unwrap().blocks() * 512
                + root.children.iter().map(|c| c.size).sum::<u64>()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

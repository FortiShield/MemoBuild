use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::hasher::ignore::IgnoreRules;

/// Walk a directory and return all non-ignored files.
/// Fix 1 — Deterministic sort: sorted by absolute path before returning.
pub fn walk_dir(root: &Path, ignore: &IgnoreRules) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_path_buf();
            if ignore.is_ignored(&rel) {
                None
            } else {
                Some(entry.path().to_path_buf())
            }
        })
        .collect();

    // Fix 1: explicit sort by path for OS-independent determinism
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn make_temp_tree() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.txt"), "world").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("c.txt"), "nested").unwrap();
        dir
    }

    #[test]
    fn test_walk_finds_all_files() {
        let dir = make_temp_tree();
        assert_eq!(walk_dir(dir.path(), &IgnoreRules::empty()).len(), 3);
    }

    #[test]
    fn test_walk_respects_ignore() {
        let dir = make_temp_tree();
        let rules = IgnoreRules::parse("sub");
        assert_eq!(walk_dir(dir.path(), &rules).len(), 2);
    }

    #[test]
    fn test_walk_is_sorted() {
        let dir = make_temp_tree();
        let files = walk_dir(dir.path(), &IgnoreRules::empty());
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted, "walk_dir must return sorted paths");
    }
}

//! Parts library: scan, import and remove collections of DXF/DWG symbol files.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::cmd::bad;

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 10_000;

/// A single DXF/DWG file in the library.
#[derive(Clone, Debug)]
pub struct LibraryEntry {
    pub name: String,
    pub path: PathBuf,
}

/// A folder (possibly with subfolders) of library entries.
#[derive(Clone, Debug)]
pub struct LibraryFolder {
    pub name: String,
    pub path: PathBuf,
    pub entries: Vec<LibraryEntry>,
    pub children: Vec<LibraryFolder>,
}

fn is_drawing(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("dxf" | "dwg"))
}

/// Scan `root` into a tree of folders and drawing entries.
#[cfg(not(target_arch = "wasm32"))]
pub fn scan_library(root: &Path) -> Result<LibraryFolder> {
    let mut count = 0usize;
    scan_dir(root, 0, &mut count)
}

#[cfg(not(target_arch = "wasm32"))]
fn scan_dir(dir: &Path, depth: usize, count: &mut usize) -> Result<LibraryFolder> {
    let name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
    let mut entries = Vec::new();
    let mut children = Vec::new();

    let read = std::fs::read_dir(dir).map_err(|e| bad("library", format!("{}: {e}", dir.display())))?;
    let mut items: Vec<_> = read.filter_map(|r| r.ok()).collect();
    items.sort_by_key(|a| a.file_name());

    for item in items {
        if *count >= MAX_ENTRIES {
            break;
        }
        let path = item.path();
        let ft = match item.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() && depth < MAX_DEPTH {
            match scan_dir(&path, depth + 1, count) {
                Ok(child) if !child.entries.is_empty() || !child.children.is_empty() => {
                    children.push(child);
                }
                _ => {}
            }
        } else if ft.is_file() && is_drawing(&path) {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            entries.push(LibraryEntry { name: stem, path });
            *count += 1;
        }
    }

    Ok(LibraryFolder { name, path: dir.to_path_buf(), entries, children })
}

/// Copy `source` folder into `libraries_dir`, returning the destination path.
/// Only `.dxf` and `.dwg` files are copied; other files are skipped.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_library(source: &Path, libraries_dir: &Path) -> Result<PathBuf> {
    if !source.is_dir() {
        return Err(bad("library", format!("{} is not a directory", source.display())));
    }
    let folder_name = source.file_name().ok_or_else(|| bad("library", "invalid folder name"))?;
    let mut dest = libraries_dir.join(folder_name);

    if dest.exists() {
        for i in 2u32..100 {
            let candidate = libraries_dir.join(format!("{}_{i}", folder_name.to_string_lossy()));
            if !candidate.exists() {
                dest = candidate;
                break;
            }
        }
        if dest.exists() {
            return Err(bad("library", "too many libraries with the same name"));
        }
    }

    std::fs::create_dir_all(&dest).map_err(|e| bad("library", format!("create {}: {e}", dest.display())))?;
    copy_drawings(source, &dest)?;
    Ok(dest)
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_drawings(src: &Path, dst: &Path) -> Result<()> {
    let read = std::fs::read_dir(src).map_err(|e| bad("library", format!("{}: {e}", src.display())))?;
    let mut items: Vec<_> = read.filter_map(|r| r.ok()).collect();
    items.sort_by_key(|a| a.file_name());

    for item in items {
        let path = item.path();
        let ft = match item.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            let sub = dst.join(item.file_name());
            std::fs::create_dir_all(&sub).map_err(|e| bad("library", format!("create {}: {e}", sub.display())))?;
            copy_drawings(&path, &sub)?;
        } else if ft.is_file() && is_drawing(&path) {
            let target = dst.join(item.file_name());
            std::fs::copy(&path, &target).map_err(|e| bad("library", format!("copy {}: {e}", path.display())))?;
        }
    }
    Ok(())
}

/// Remove a library by name from `libraries_dir`.
#[cfg(not(target_arch = "wasm32"))]
pub fn remove_library(name: &str, libraries_dir: &Path) -> Result<()> {
    let target = libraries_dir.join(name);
    if !target.starts_with(libraries_dir) || !target.is_dir() {
        return Err(bad("library", format!("library \"{name}\" not found")));
    }
    std::fs::remove_dir_all(&target).map_err(|e| bad("library", format!("remove {}: {e}", target.display())))?;
    Ok(())
}

/// List installed library folder names.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_libraries(libraries_dir: &Path) -> Result<Vec<String>> {
    let read = match std::fs::read_dir(libraries_dir) {
        Ok(r) => r,
        Err(_) => return Ok(Vec::new()),
    };
    let mut names: Vec<String> = read
        .filter_map(|r| r.ok())
        .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_empty_dir() {
        let tmp = std::env::temp_dir().join("cadcraft_test_lib_empty");
        let _ = std::fs::create_dir_all(&tmp);
        let folder = scan_library(&tmp).unwrap_or_else(|_| LibraryFolder {
            name: String::new(),
            path: tmp.clone(),
            entries: Vec::new(),
            children: Vec::new(),
        });
        assert!(folder.entries.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn is_drawing_checks() {
        assert!(is_drawing(Path::new("foo.dxf")));
        assert!(is_drawing(Path::new("bar.DWG")));
        assert!(!is_drawing(Path::new("baz.txt")));
        assert!(!is_drawing(Path::new("no_ext")));
    }
}

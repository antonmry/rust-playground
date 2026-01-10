use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn discover_mp4s(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in folder.read_dir().context("read directory")? {
        let entry = entry.context("read directory entry")?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if is_mp4(&path) {
            files.push(path);
        }
    }
    files.sort_by(|a, b| {
        let a_name = a.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let b_name = b.file_name().and_then(|s| s.to_str()).unwrap_or("");
        a_name.cmp(b_name)
    });
    Ok(files)
}

fn is_mp4(path: &Path) -> bool {
    match path.extension().and_then(OsStr::to_str) {
        Some(ext) => ext.eq_ignore_ascii_case("mp4"),
        None => false,
    }
}

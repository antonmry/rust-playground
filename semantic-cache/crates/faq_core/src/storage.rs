use crate::model::FaqEntry;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

pub fn save_entries_jsonl(path: &Path, entries: &[FaqEntry]) -> Result<()> {
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut writer = BufWriter::new(file);

    for entry in entries {
        let line = serde_json::to_string(entry).context("serialize faq entry")?;
        writer
            .write_all(line.as_bytes())
            .context("write entry line")?;
        writer.write_all(b"\n").context("write newline")?;
    }

    writer.flush().context("flush output")
}

pub fn load_entries_jsonl(path: &Path) -> Result<Vec<FaqEntry>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line.context("read jsonl line")?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: FaqEntry = serde_json::from_str(&line).context("parse faq entry json")?;
        entries.push(entry);
    }

    Ok(entries)
}

use crate::model::{Export, FileCuts, Segment, fmt_time_hhmmss_millis};
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub fn export_markers_json(folder: &Path, cuts: &BTreeMap<String, Vec<Segment>>) -> Result<()> {
    let export = build_export(cuts);
    let path = folder.join("markers.json");
    let file = File::create(&path).with_context(|| format!("create {path:?}"))?;
    serde_json::to_writer_pretty(file, &export).context("write markers.json")?;
    Ok(())
}

pub fn export_per_file_csv(folder: &Path, cuts: &BTreeMap<String, Vec<Segment>>) -> Result<()> {
    for (path, segments) in cuts {
        if segments.is_empty() {
            continue;
        }
        let basename = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let filename = format!("{basename}.cuts.csv");
        let out_path = folder.join(filename);
        let file = File::create(&out_path).with_context(|| format!("create {out_path:?}"))?;
        let mut writer = BufWriter::new(file);
        for segment in segments {
            let start = fmt_time_hhmmss_millis(segment.start);
            let end = fmt_time_hhmmss_millis(segment.end);
            writeln!(writer, "{start},{end}").context("write CSV line")?;
        }
    }
    Ok(())
}

fn build_export(cuts: &BTreeMap<String, Vec<Segment>>) -> Export {
    let mut files = Vec::new();
    for (path, segments) in cuts {
        files.push(FileCuts {
            path: path.to_string(),
            segments: segments.clone(),
        });
    }
    Export {
        version: 1,
        created_at: Utc::now(),
        files,
    }
}

pub fn export_all(folder: &Path, cuts: &BTreeMap<String, Vec<Segment>>) -> Result<()> {
    export_markers_json(folder, cuts)?;
    export_per_file_csv(folder, cuts)?;
    Ok(())
}

pub fn load_markers_json(folder: &Path) -> Result<Option<Export>> {
    let path = folder.join("markers.json");
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(&path).with_context(|| format!("open {path:?}"))?;
    let export = serde_json::from_reader(file).context("read markers.json")?;
    Ok(Some(export))
}

pub fn cuts_from_export(export: &Export) -> BTreeMap<String, Vec<Segment>> {
    let mut cuts = BTreeMap::new();
    for file in &export.files {
        cuts.insert(file.path.clone(), file.segments.clone());
    }
    cuts
}

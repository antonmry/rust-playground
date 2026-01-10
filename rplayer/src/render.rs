use crate::model::Segment;
use anyhow::{Context, Result, anyhow};
use chrono::Local;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub enum RenderEvent {
    Started { total: usize },
    SegmentDone { current: usize, total: usize },
    Concatenating,
    Done(PathBuf),
    Error(String),
}

pub fn render_highlights_with_progress(
    folder: &Path,
    files: &[PathBuf],
    cuts: &BTreeMap<String, Vec<Segment>>,
    progress: Option<Sender<RenderEvent>>,
) -> Result<PathBuf> {
    let segments = collect_segments_in_order(files, cuts);
    if segments.is_empty() {
        return Err(anyhow!("no markers to render"));
    }

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let output_dir = folder.join("output");
    fs::create_dir_all(&output_dir).context("create output directory")?;
    let output = output_dir.join(format!("output_{timestamp}.mp4"));

    let temp_dir = std::env::temp_dir().join(format!(
        "rplayer_segments_{}_{}",
        std::process::id(),
        timestamp
    ));

    let result = (|| {
        fs::create_dir_all(&temp_dir).context("create temp segment dir")?;
        let mut dims_cache: HashMap<String, (i64, i64)> = HashMap::new();
        let total = segments.len();
        if let Some(sender) = progress.as_ref() {
            let _ = sender.send(RenderEvent::Started { total });
        }

        let mut segment_paths = Vec::new();
        for (idx, (path, segment)) in segments.iter().enumerate() {
            let segment_path = temp_dir.join(format!("segment_{idx:04}.mp4"));
            let dims = match dims_cache.get(path) {
                Some(dims) => *dims,
                None => {
                    let dims = get_video_dims(path)?;
                    dims_cache.insert(path.clone(), dims);
                    dims
                }
            };
            render_segment_with_crop(path, segment, &segment_path, dims)
                .with_context(|| format!("render segment {idx} from {path}"))?;
            segment_paths.push(segment_path);
            if let Some(sender) = progress.as_ref() {
                let _ = sender.send(RenderEvent::SegmentDone {
                    current: idx + 1,
                    total,
                });
            }
        }

        let list_path = temp_dir.join("concat_list.txt");
        write_concat_list(&list_path, &segment_paths)?;
        if let Some(sender) = progress.as_ref() {
            let _ = sender.send(RenderEvent::Concatenating);
        }

        concat_segments(&list_path, &output).context("concat segments")?;
        Ok(output)
    })();

    if let Err(err) = fs::remove_dir_all(&temp_dir) {
        crate::log::log_error(&format!("failed to remove temp dir {temp_dir:?}: {err}"));
    }

    if let Some(sender) = progress.as_ref() {
        match &result {
            Ok(path) => {
                let _ = sender.send(RenderEvent::Done(path.clone()));
            }
            Err(err) => {
                let _ = sender.send(RenderEvent::Error(format!("{err:#}")));
            }
        }
    }

    result
}

fn collect_segments_in_order(
    files: &[PathBuf],
    cuts: &BTreeMap<String, Vec<Segment>>,
) -> Vec<(String, Segment)> {
    let mut segments = Vec::new();
    for path in files {
        let path_str = path.to_string_lossy().into_owned();
        if let Some(cuts) = cuts.get(&path_str) {
            for segment in cuts {
                segments.push((path_str.clone(), segment.clone()));
            }
        }
    }
    segments
}

const TARGET_FPS: &str = "60";

fn render_segment_with_crop(
    input: &str,
    segment: &Segment,
    output: &Path,
    dims: (i64, i64),
) -> Result<()> {
    let start = segment.start.to_string();
    let end = segment.end.to_string();
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-ss")
        .arg(start)
        .arg("-to")
        .arg(end)
        .arg("-i")
        .arg(input);
    let filter = build_video_filter(segment, dims);
    cmd.arg("-vf").arg(filter);
    let status = cmd
        .arg("-fps_mode")
        .arg("cfr")
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("18")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-color_range")
        .arg("tv")
        .arg("-colorspace")
        .arg("bt709")
        .arg("-color_primaries")
        .arg("bt709")
        .arg("-color_trc")
        .arg("bt709")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-ar")
        .arg("48000")
        .arg("-ac")
        .arg("2")
        .arg("-shortest")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run ffmpeg segment")?;

    if !status.success() {
        return Err(anyhow!("ffmpeg failed for segment {input}"));
    }
    Ok(())
}

fn write_concat_list(list_path: &Path, segments: &[PathBuf]) -> Result<()> {
    let file = File::create(list_path).context("create concat list")?;
    let mut writer = BufWriter::new(file);
    for path in segments {
        writeln!(writer, "file '{}'", path.display()).context("write concat list")?;
    }
    Ok(())
}

fn concat_segments(list_path: &Path, output: &Path) -> Result<()> {
    let status = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(list_path)
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run ffmpeg concat")?;

    if !status.success() {
        return Err(anyhow!("ffmpeg concat failed"));
    }
    Ok(())
}

fn get_video_dims(input: &str) -> Result<(i64, i64)> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height")
        .arg("-of")
        .arg("csv=p=0:s=x")
        .arg(input)
        .output()
        .context("run ffprobe")?;
    if !output.status.success() {
        return Err(anyhow!("ffprobe failed for {input}"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.trim();
    let mut parts = line.split('x');
    let width: i64 = parts
        .next()
        .ok_or_else(|| anyhow!("ffprobe missing width"))?
        .parse()
        .context("parse width")?;
    let height: i64 = parts
        .next()
        .ok_or_else(|| anyhow!("ffprobe missing height"))?
        .parse()
        .context("parse height")?;
    Ok((width, height))
}

fn build_video_filter(segment: &Segment, dims: (i64, i64)) -> String {
    let zoom = segment.zoom.max(1.0);
    let (width, height) = dims;
    if zoom == 1.0 && segment.pan_x == 0.0 && segment.pan_y == 0.0 {
        return format!("fps={TARGET_FPS}");
    }
    let crop_w = (width as f64 / zoom).round().max(1.0) as i64;
    let crop_h = (height as f64 / zoom).round().max(1.0) as i64;
    let max_x = ((width - crop_w) as f64 / 2.0).max(0.0);
    let max_y = ((height - crop_h) as f64 / 2.0).max(0.0);
    let offset_x = (segment.pan_x.clamp(-1.0, 1.0) * max_x).round();
    let offset_y = (segment.pan_y.clamp(-1.0, 1.0) * max_y).round();
    let mut x = ((width - crop_w) as f64 / 2.0 + offset_x).round() as i64;
    let mut y = ((height - crop_h) as f64 / 2.0 + offset_y).round() as i64;
    x = x.clamp(0, width - crop_w);
    y = y.clamp(0, height - crop_h);
    format!("crop={crop_w}:{crop_h}:{x}:{y},scale={width}:{height},fps={TARGET_FPS}")
}

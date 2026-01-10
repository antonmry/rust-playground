use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    #[serde(default = "default_zoom")]
    pub zoom: f64,
    #[serde(default = "default_pan")]
    pub pan_x: f64,
    #[serde(default = "default_pan")]
    pub pan_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCuts {
    pub path: String,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub files: Vec<FileCuts>,
}

fn default_zoom() -> f64 {
    1.0
}

fn default_pan() -> f64 {
    0.0
}

pub fn fmt_time_hhmmss_millis(seconds: f64) -> String {
    let mut total_ms = (seconds * 1000.0).round() as i64;
    if total_ms < 0 {
        total_ms = 0;
    }
    let millis = total_ms % 1000;
    let total_seconds = total_ms / 1000;
    let secs = total_seconds % 60;
    let mins = (total_seconds / 60) % 60;
    let hours = total_seconds / 3600;
    format!("{hours:02}:{mins:02}:{secs:02}.{millis:03}")
}

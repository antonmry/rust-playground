use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub fn request(ipc_path: &Path, command: Value) -> Result<Value> {
    let payload = json!({
        "command": command,
        "request_id": 1,
    });
    let mut stream = UnixStream::connect(ipc_path).context("connect to mpv IPC")?;
    stream
        .write_all(payload.to_string().as_bytes())
        .context("write mpv IPC request")?;
    stream.write_all(b"\n").context("write newline")?;
    stream.flush().context("flush IPC request")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("read mpv IPC response")?;
    if line.trim().is_empty() {
        return Err(anyhow!("empty mpv IPC response"));
    }
    let value: Value = serde_json::from_str(&line).context("parse mpv IPC response")?;
    let error = value
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("unknown");
    if error != "success" {
        return Err(anyhow!("mpv IPC error: {error}"));
    }
    Ok(value.get("data").cloned().unwrap_or(Value::Null))
}

pub fn send_cmd(ipc_path: &Path, command: Value) -> Result<()> {
    let _ = request(ipc_path, command)?;
    Ok(())
}

pub fn get_f64(ipc_path: &Path, property: &str) -> Result<f64> {
    let data = request(ipc_path, json!(["get_property", property]))?;
    data.as_f64()
        .ok_or_else(|| anyhow!("property {property} is not f64"))
}

pub fn get_bool(ipc_path: &Path, property: &str) -> Result<bool> {
    let data = request(ipc_path, json!(["get_property", property]))?;
    data.as_bool()
        .ok_or_else(|| anyhow!("property {property} is not bool"))
}

pub fn get_i64(ipc_path: &Path, property: &str) -> Result<i64> {
    let data = request(ipc_path, json!(["get_property", property]))?;
    if let Some(value) = data.as_i64() {
        return Ok(value);
    }
    if let Some(value) = data.as_f64() {
        return Ok(value.round() as i64);
    }
    Err(anyhow!("property {property} is not numeric"))
}

pub fn get_string(ipc_path: &Path, property: &str) -> Result<String> {
    let data = request(ipc_path, json!(["get_property", property]))?;
    data.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("property {property} is not string"))
}

pub fn set_bool(ipc_path: &Path, property: &str, value: bool) -> Result<()> {
    send_cmd(ipc_path, json!(["set_property", property, value]))
}

pub fn set_f64(ipc_path: &Path, property: &str, value: f64) -> Result<()> {
    send_cmd(ipc_path, json!(["set_property", property, value]))
}

pub fn seek_rel(ipc_path: &Path, seconds: f64) -> Result<()> {
    send_cmd(ipc_path, json!(["seek", seconds, "relative"]))
}

pub fn playlist_next(ipc_path: &Path) -> Result<()> {
    send_cmd(ipc_path, json!(["playlist-next", "force"]))
}

pub fn playlist_prev(ipc_path: &Path) -> Result<()> {
    send_cmd(ipc_path, json!(["playlist-prev", "force"]))
}

pub fn cycle_pause(ipc_path: &Path) -> Result<()> {
    send_cmd(ipc_path, json!(["cycle", "pause"]))
}

pub fn cycle_mute(ipc_path: &Path) -> Result<()> {
    send_cmd(ipc_path, json!(["cycle", "mute"]))
}

pub fn quit(ipc_path: &Path) -> Result<()> {
    send_cmd(ipc_path, json!(["quit"]))
}

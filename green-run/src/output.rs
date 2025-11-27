use std::io::{self, Write};

use serde::Serialize;

use crate::error::{EnergyError, Result};
use crate::runner::RunResult;

pub fn print_result(format: &str, result: &RunResult) -> Result<()> {
    match format {
        "text" => print_text(result),
        "json" => print_json(result),
        other => Err(EnergyError::InvalidArg(format!(
            "Unknown output format: {other}"
        ))),
    }
}

fn print_text(result: &RunResult) -> Result<()> {
    let total_energy_j = match (result.cpu_energy_j, result.gpu_energy_j) {
        (None, None) => None,
        _ => Some(result.cpu_energy_j.unwrap_or(0.0) + result.gpu_energy_j.unwrap_or(0.0)),
    };
    let total_energy_kwh = total_energy_j.map(|v| v / 3_600_000.0);
    let avg_cpu_power_w = result
        .cpu_energy_j
        .map(|v| average_power(v, result.duration_s));
    let avg_gpu_power_w = result
        .gpu_energy_j
        .map(|v| average_power(v, result.duration_s));
    let avg_total_power_w = total_energy_j.map(|v| average_power(v, result.duration_s));

    let mut out = io::stdout();
    writeln!(
        out,
        "Command: {}",
        shell_words::join(result.command.iter().map(|s| s.as_str()))
    )?;
    writeln!(out, "duration_s: {:.6}", result.duration_s)?;
    writeln!(out, "cpu_energy_j: {}", format_opt(result.cpu_energy_j))?;
    writeln!(out, "gpu_energy_j: {}", format_opt(result.gpu_energy_j))?;
    writeln!(out, "total_energy_kwh: {}", format_opt(total_energy_kwh))?;
    writeln!(out, "total_energy_j: {}", format_opt(total_energy_j))?;
    writeln!(out, "avg_cpu_power_w: {}", format_opt(avg_cpu_power_w))?;
    writeln!(out, "avg_gpu_power_w: {}", format_opt(avg_gpu_power_w))?;
    writeln!(out, "avg_total_power_w: {}", format_opt(avg_total_power_w))?;
    writeln!(out)?;

    writeln!(
        out,
        "Exit code: {}",
        result.exit_status.code().unwrap_or(-1)
    )?;
    Ok(())
}

#[derive(Serialize)]
struct JsonResult<'a> {
    command: &'a [String],
    duration_s: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_energy_j: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_energy_j: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_energy_j: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_energy_kwh: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_cpu_power_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_gpu_power_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avg_total_power_w: Option<f64>,
    exit_code: i32,
}

fn print_json(result: &RunResult) -> Result<()> {
    let total_energy = match (result.cpu_energy_j, result.gpu_energy_j) {
        (None, None) => None,
        _ => Some(result.cpu_energy_j.unwrap_or(0.0) + result.gpu_energy_j.unwrap_or(0.0)),
    };
    let json_result = JsonResult {
        command: &result.command,
        duration_s: result.duration_s,
        cpu_energy_j: result.cpu_energy_j,
        gpu_energy_j: result.gpu_energy_j,
        total_energy_j: total_energy,
        total_energy_kwh: total_energy.map(|v| v / 3_600_000.0),
        avg_cpu_power_w: result
            .cpu_energy_j
            .map(|v| average_power(v, result.duration_s)),
        avg_gpu_power_w: result
            .gpu_energy_j
            .map(|v| average_power(v, result.duration_s)),
        avg_total_power_w: total_energy.map(|v| average_power(v, result.duration_s)),
        exit_code: result.exit_status.code().unwrap_or(-1),
    };
    let out = serde_json::to_string_pretty(&json_result)?;
    println!("{out}");
    Ok(())
}

fn average_power(energy_j: f64, duration_s: f64) -> f64 {
    if duration_s > 0.0 {
        energy_j / duration_s
    } else {
        0.0
    }
}

fn format_opt(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{:.6}", v),
        None => "N/A".to_string(),
    }
}

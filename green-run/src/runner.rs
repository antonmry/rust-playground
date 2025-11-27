use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::cli::Cli;
use crate::energy::{EnergyBackends, NodeEnergyBackend};
use crate::error::{EnergyError, Result};

pub struct RunResult {
    pub command: Vec<String>,
    pub duration_s: f64,
    pub cpu_energy_j: Option<f64>,
    pub gpu_energy_j: Option<f64>,
    pub exit_status: ExitStatus,
}

pub fn run_command(cli: &Cli) -> Result<RunResult> {
    cli.validate()
        .map_err(|msg| EnergyError::InvalidArg(msg.to_string()))?;

    let mut backend = EnergyBackends::new(cli.cpu, cli.gpu, cli.rapl_root.clone())?;
    backend.start()?;

    let mut cmd = Command::new(&cli.command[0]);
    cmd.args(&cli.command[1..]);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let mut child = cmd.spawn()?;

    let sample_interval: Duration = cli.sample_interval.into();
    let start = Instant::now();
    let mut last_tick = start;
    let mut sample_error: Option<EnergyError> = None;

    loop {
        match child.try_wait()? {
            Some(status) => {
                let now = Instant::now();
                let dt = (now - last_tick).as_secs_f64();
                if sample_error.is_none() && dt > 0.0 {
                    if let Err(err) = backend.sample(dt) {
                        sample_error = Some(err);
                    }
                }

                let stop_result = backend.stop();
                if let Some(err) = sample_error {
                    // Sampling failed mid-run; honor failure after the child exits.
                    return Err(err);
                }
                stop_result?;

                let duration_s = (now - start).as_secs_f64();
                return Ok(RunResult {
                    command: cli.command.clone(),
                    duration_s,
                    cpu_energy_j: backend.cpu_energy_joules(),
                    gpu_energy_j: backend.gpu_energy_joules(),
                    exit_status: status,
                });
            }
            None => {
                std::thread::sleep(sample_interval);
                let now = Instant::now();
                let dt = (now - last_tick).as_secs_f64();
                if sample_error.is_none() && dt > 0.0 {
                    if let Err(err) = backend.sample(dt) {
                        sample_error = Some(err);
                    }
                }
                last_tick = now;
            }
        }
    }
}

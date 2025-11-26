use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "energy_run",
    version,
    about = "Measure CPU/GPU energy for a command"
)]
pub struct Cli {
    #[arg(long, default_value = "100ms")]
    pub sample_interval: humantime::Duration,

    #[arg(long, default_value = "text")]
    pub output: String,

    // Only negative toggles are exposed; defaults to enabled.
    #[arg(long = "no-cpu", default_value_t = true, action = clap::ArgAction::SetFalse)]
    pub cpu: bool,

    #[arg(long = "no-gpu", default_value_t = true, action = clap::ArgAction::SetFalse)]
    pub gpu: bool,

    #[arg(long)]
    pub rapl_root: Option<PathBuf>,

    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

impl Cli {
    pub fn validate(&self) -> Result<(), String> {
        if self.sample_interval.as_ref().is_zero() {
            return Err("sample-interval must be > 0".to_string());
        }

        if self.command.is_empty() {
            return Err("command is required".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_defaults() {
        let args = ["energy_run", "--", "echo", "hi"];
        let cli = Cli::try_parse_from(args).expect("parse");
        assert_eq!(cli.output, "text");
        assert!(cli.cpu);
        assert!(cli.gpu);
        assert_eq!(cli.command, vec!["echo", "hi"]);
    }

    #[test]
    fn parses_negative_toggles() {
        let args = [
            "energy_run",
            "--no-cpu",
            "--no-gpu",
            "--output",
            "json",
            "--",
            "cmd",
        ];
        let cli = Cli::try_parse_from(args).expect("parse");
        assert!(!cli.cpu);
        assert!(!cli.gpu);
        assert_eq!(cli.output, "json");
    }
}

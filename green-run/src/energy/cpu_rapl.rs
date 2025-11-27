use std::env::consts::ARCH;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::energy::NodeEnergyBackend;
use crate::error::{EnergyError, Result};
use crate::util::handle_wrap;

struct RaplDomain {
    energy_path: PathBuf,
    max_uj: u64,
    initial_uj: Option<u64>,
}

pub struct CpuRapl {
    domains: Vec<RaplDomain>,
    total_j: Option<f64>,
}

impl CpuRapl {
    pub fn discover(root: Option<PathBuf>) -> Result<Self> {
        if ARCH != "x86_64" && !cfg!(test) {
            return Err(EnergyError::BackendUnavailable(format!(
                "CPU energy requires Intel x86_64 with RAPL; current architecture is {}. Use --no-cpu or run on an Intel host with RAPL support.",
                ARCH
            )));
        }

        let root = root.unwrap_or_else(|| PathBuf::from("/sys/class/powercap"));
        if !root.exists() {
            return Err(EnergyError::BackendUnavailable(format!(
                "CPU energy requires RAPL sysfs under {} (not found). Use --no-cpu or ensure intel_rapl modules are loaded on Intel hardware.",
                root.display()
            )));
        }
        let mut domains = Vec::new();

        for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
            if entry.file_name() != "energy_uj" {
                continue;
            }
            let energy_path = entry.into_path();
            let max_path = energy_path.with_file_name("max_energy_range_uj");
            if !max_path.exists() {
                continue;
            }
            let max_uj = read_u64(&max_path)?;
            domains.push(RaplDomain {
                energy_path,
                max_uj,
                initial_uj: None,
            });
        }

        if domains.is_empty() {
            return Err(EnergyError::BackendUnavailable(format!(
                "CPU energy requires RAPL sysfs entries; none found under {}. Use --no-cpu or load intel_rapl modules on Intel hardware.",
                root.display()
            )));
        }

        Ok(Self {
            domains,
            total_j: None,
        })
    }
}

impl NodeEnergyBackend for CpuRapl {
    fn start(&mut self) -> Result<()> {
        for dom in &mut self.domains {
            let value = read_u64(&dom.energy_path)?;
            dom.initial_uj = Some(value);
        }
        Ok(())
    }

    fn sample(&mut self, _dt_seconds: f64) -> Result<()> {
        // RAPL is read-once: no action needed per sample.
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let mut total_uj: u64 = 0;
        for dom in &mut self.domains {
            let initial = dom.initial_uj.ok_or_else(|| {
                EnergyError::InvalidArg("RAPL domain not initialized".to_string())
            })?;
            let current = read_u64(&dom.energy_path)?;
            let delta = handle_wrap(current, initial, dom.max_uj);
            total_uj += delta;
        }
        self.total_j = Some(total_uj as f64 / 1_000_000.0);
        Ok(())
    }

    fn cpu_energy_joules(&self) -> Option<f64> {
        self.total_j
    }

    fn gpu_energy_joules(&self) -> Option<f64> {
        None
    }
}

fn read_u64(path: &Path) -> Result<u64> {
    let contents = fs::read_to_string(path)?;
    let trimmed = contents.trim();
    trimmed
        .parse::<u64>()
        .map_err(|e| EnergyError::InvalidArg(format!("{}: {}", path.display(), e)))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::CpuRapl;
    use crate::energy::NodeEnergyBackend;

    #[test]
    fn rapl_wraparound() {
        let dir = TempDir::new().unwrap();
        let rapl0 = dir.path().join("intel-rapl:0");
        fs::create_dir_all(&rapl0).unwrap();
        fs::write(rapl0.join("energy_uj"), "190").unwrap();
        fs::write(rapl0.join("max_energy_range_uj"), "200").unwrap();

        let rapl1 = dir.path().join("intel-rapl:1");
        fs::create_dir_all(&rapl1).unwrap();
        fs::write(rapl1.join("energy_uj"), "50").unwrap();
        fs::write(rapl1.join("max_energy_range_uj"), "200").unwrap();

        let mut backend = CpuRapl::discover(Some(dir.path().to_path_buf())).unwrap();
        backend.start().unwrap();

        // Move counters forward with wrap on the first domain.
        fs::write(rapl0.join("energy_uj"), "10").unwrap();
        fs::write(rapl1.join("energy_uj"), "70").unwrap();

        backend.stop().unwrap();
        let energy = backend.cpu_energy_joules().unwrap();
        // (wrap 10 vs 190 at max 200 => 20) + (70-50 => 20) = 40 microjoules
        assert!((energy - 0.00004).abs() < 1e-9);
    }
}

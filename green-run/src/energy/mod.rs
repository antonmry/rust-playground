use std::path::PathBuf;

use crate::error::Result;

pub mod cpu_rapl;
pub mod gpu_nvml;
pub mod mock;

pub trait NodeEnergyBackend {
    fn start(&mut self) -> Result<()>;
    fn sample(&mut self, dt_seconds: f64) -> Result<()>;
    fn stop(&mut self) -> Result<()>;

    fn cpu_energy_joules(&self) -> Option<f64>;
    fn gpu_energy_joules(&self) -> Option<f64>;
}

pub struct EnergyBackends {
    cpu: Option<cpu_rapl::CpuRapl>,
    gpu: Option<gpu_nvml::GpuNvml>,
}

impl EnergyBackends {
    pub fn new(enable_cpu: bool, enable_gpu: bool, rapl_root: Option<PathBuf>) -> Result<Self> {
        let cpu = if enable_cpu {
            Some(cpu_rapl::CpuRapl::discover(rapl_root)?)
        } else {
            None
        };

        let gpu = if enable_gpu {
            Some(gpu_nvml::GpuNvml::new()?)
        } else {
            None
        };

        Ok(Self { cpu, gpu })
    }
}

impl NodeEnergyBackend for EnergyBackends {
    fn start(&mut self) -> Result<()> {
        if let Some(cpu) = self.cpu.as_mut() {
            cpu.start()?;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.start()?;
        }
        Ok(())
    }

    fn sample(&mut self, dt_seconds: f64) -> Result<()> {
        if let Some(cpu) = self.cpu.as_mut() {
            cpu.sample(dt_seconds)?;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.sample(dt_seconds)?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(cpu) = self.cpu.as_mut() {
            cpu.stop()?;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.stop()?;
        }
        Ok(())
    }

    fn cpu_energy_joules(&self) -> Option<f64> {
        self.cpu.as_ref().and_then(|c| c.cpu_energy_joules())
    }

    fn gpu_energy_joules(&self) -> Option<f64> {
        self.gpu.as_ref().and_then(|g| g.gpu_energy_joules())
    }
}

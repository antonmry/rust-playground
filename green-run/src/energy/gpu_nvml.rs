use std::sync::Arc;

use nvml_wrapper::Nvml;

use crate::energy::NodeEnergyBackend;
use crate::error::{EnergyError, Result};

trait PowerSampler: Send + Sync {
    fn sample_power_w(&self) -> Result<f64>;
}

struct NvmlPowerSampler {
    nvml: Arc<Nvml>,
    index: u32,
}

impl PowerSampler for NvmlPowerSampler {
    fn sample_power_w(&self) -> Result<f64> {
        let device = self.nvml.device_by_index(self.index)?;
        let mw = device.power_usage()?;
        Ok(mw as f64 / 1000.0)
    }
}

pub struct GpuNvml {
    // Keep NVML alive for real devices; None for test samplers.
    _nvml: Option<Arc<Nvml>>,
    samplers: Vec<Box<dyn PowerSampler>>,
    last_power_w: f64,
    energy_j: f64,
}

impl GpuNvml {
    pub fn new() -> Result<Self> {
        let nvml = Arc::new(Nvml::init().map_err(|e| {
            EnergyError::BackendUnavailable(format!(
                "GPU energy requires NVIDIA NVML; failed to initialize NVML ({e}). Use --no-gpu or install NVIDIA drivers/hardware."
            ))
        })?);
        let count = nvml.device_count().map_err(|e| {
            EnergyError::BackendUnavailable(format!(
                "GPU energy requires NVIDIA NVML; failed to list devices ({e}). Use --no-gpu or install NVIDIA drivers/hardware."
            ))
        })?;
        let mut samplers: Vec<Box<dyn PowerSampler>> = Vec::new();
        for idx in 0..count {
            samplers.push(Box::new(NvmlPowerSampler {
                nvml: nvml.clone(),
                index: idx,
            }));
        }
        if samplers.is_empty() {
            return Err(EnergyError::BackendUnavailable(
                "GPU energy requires an NVIDIA GPU; NVML found no devices. Use --no-gpu or attach an NVIDIA GPU with drivers installed."
                    .to_string(),
            ));
        }
        Self::from_samplers_internal(Some(nvml), samplers)
    }

    #[cfg(test)]
    fn from_mock_samplers(samplers: Vec<Box<dyn PowerSampler>>) -> Result<Self> {
        Self::from_samplers_internal(None, samplers)
    }

    fn from_samplers_internal(
        nvml: Option<Arc<Nvml>>,
        samplers: Vec<Box<dyn PowerSampler>>,
    ) -> Result<Self> {
        if samplers.is_empty() {
            return Err(EnergyError::BackendUnavailable(
                "No GPU samplers provided".to_string(),
            ));
        }
        let initial_power = average_power(&samplers)?;
        Ok(Self {
            _nvml: nvml,
            samplers,
            last_power_w: initial_power,
            energy_j: 0.0,
        })
    }
}

impl NodeEnergyBackend for GpuNvml {
    fn start(&mut self) -> Result<()> {
        // Initial power already sampled during construction.
        Ok(())
    }

    fn sample(&mut self, dt_seconds: f64) -> Result<()> {
        self.energy_j += self.last_power_w * dt_seconds;
        self.last_power_w = average_power(&self.samplers)?;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // Caller is responsible for sampling final interval before stop.
        Ok(())
    }

    fn cpu_energy_joules(&self) -> Option<f64> {
        None
    }

    fn gpu_energy_joules(&self) -> Option<f64> {
        Some(self.energy_j)
    }
}

fn average_power(samplers: &[Box<dyn PowerSampler>]) -> Result<f64> {
    let mut total = 0.0;
    for sampler in samplers {
        total += sampler.sample_power_w()?;
    }
    Ok(total / samplers.len() as f64)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::{GpuNvml, PowerSampler};
    use crate::energy::NodeEnergyBackend;
    use crate::error::Result;

    struct FakeSampler {
        power: f64,
    }

    impl FakeSampler {
        fn new(power: f64) -> Self {
            Self { power }
        }
    }

    impl PowerSampler for FakeSampler {
        fn sample_power_w(&self) -> Result<f64> {
            Ok(self.power)
        }
    }

    #[test]
    fn integrates_energy_over_time() {
        let samplers: Vec<Box<dyn PowerSampler>> = vec![
            Box::new(FakeSampler::new(10.0)),
            Box::new(FakeSampler::new(10.0)),
        ];
        let mut gpu = GpuNvml::from_mock_samplers(samplers).unwrap();
        gpu.start().unwrap();
        gpu.sample(1.0).unwrap();
        gpu.sample(2.0).unwrap();
        gpu.stop().unwrap();
        let e = gpu.gpu_energy_joules().unwrap();
        // initial avg power 10W -> after 1s => 10J; after another 2s => 20J; total 30J
        assert!((e - 30.0).abs() < 1e-6);
    }
}

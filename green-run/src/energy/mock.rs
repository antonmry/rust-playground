use crate::energy::NodeEnergyBackend;
use crate::error::Result;

#[allow(dead_code)]
pub struct MockEnergy {
    pub cpu_power_w: f64,
    pub gpu_power_w: f64,
    pub cpu_energy_j: f64,
    pub gpu_energy_j: f64,
}

#[allow(dead_code)]
impl MockEnergy {
    pub fn new(cpu_power_w: f64, gpu_power_w: f64) -> Self {
        Self {
            cpu_power_w,
            gpu_power_w,
            cpu_energy_j: 0.0,
            gpu_energy_j: 0.0,
        }
    }
}

impl NodeEnergyBackend for MockEnergy {
    fn start(&mut self) -> Result<()> {
        Ok(())
    }

    fn sample(&mut self, dt_seconds: f64) -> Result<()> {
        self.cpu_energy_j += self.cpu_power_w * dt_seconds;
        self.gpu_energy_j += self.gpu_power_w * dt_seconds;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn cpu_energy_joules(&self) -> Option<f64> {
        Some(self.cpu_energy_j)
    }

    fn gpu_energy_joules(&self) -> Option<f64> {
        Some(self.gpu_energy_j)
    }
}

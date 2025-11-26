use std::io;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EnergyError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("NVML error: {0}")]
    Nvml(#[from] nvml_wrapper::error::NvmlError),

    #[error("Invalid argument: {0}")]
    InvalidArg(String),

    #[error("Backend unavailable: {0}")]
    BackendUnavailable(String),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, EnergyError>;

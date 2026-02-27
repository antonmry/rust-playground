use crate::eval::EvalSummary;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DEFAULT_THRESHOLD: f32 = 0.55;
pub const DEFAULT_REQUIRED_PASS_RATE: f32 = 0.85;
pub const DEFAULT_MODEL_ID: &str = "nomic-embed-text-v2-moe";
pub const DEFAULT_MODEL_REVISION: &str = "local-gguf";
pub const DEFAULT_MODEL_PATH: &str = "./models/nomic-embed-text-v2-moe.Q4_K_M.gguf";
pub const DEFAULT_EMBEDDING_DIM: usize = 768;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationStatus {
    WaitingRuntime,
    Evaluating,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleEvaluationRun {
    pub run_id: String,
    pub dataset: String,
    pub threshold: f32,
    pub required_pass_rate: f32,
    pub status: OrchestrationStatus,
    pub requested_at: DateTime<Utc>,
    pub runtime_ready_at: Option<DateTime<Utc>>,
    pub started_eval_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_cases: Option<usize>,
    pub passed_cases: Option<usize>,
    pub failed_cases: Option<usize>,
    pub pass_rate: Option<f32>,
    pub error: Option<String>,
}

impl CandleEvaluationRun {
    pub fn start(run_id: String, dataset: String, threshold: Option<f32>) -> Self {
        Self {
            run_id,
            dataset,
            threshold: threshold.unwrap_or(DEFAULT_THRESHOLD),
            required_pass_rate: DEFAULT_REQUIRED_PASS_RATE,
            status: OrchestrationStatus::WaitingRuntime,
            requested_at: Utc::now(),
            runtime_ready_at: None,
            started_eval_at: None,
            completed_at: None,
            total_cases: None,
            passed_cases: None,
            failed_cases: None,
            pass_rate: None,
            error: None,
        }
    }

    pub fn on_runtime_ready(&mut self) {
        if self.status != OrchestrationStatus::WaitingRuntime {
            return;
        }
        let now = Utc::now();
        self.status = OrchestrationStatus::Evaluating;
        self.runtime_ready_at = Some(now);
        self.started_eval_at = Some(now);
    }

    pub fn on_runtime_boot_failed(&mut self, reason: impl Into<String>) {
        if self.status != OrchestrationStatus::WaitingRuntime {
            return;
        }
        self.status = OrchestrationStatus::Failed;
        self.error = Some(reason.into());
        self.completed_at = Some(Utc::now());
    }

    pub fn on_eval_completed(&mut self, summary: &EvalSummary, required_pass_rate: f32) {
        if self.status != OrchestrationStatus::Evaluating {
            return;
        }
        self.total_cases = Some(summary.total);
        self.passed_cases = Some(summary.passed);
        self.failed_cases = Some(summary.failed);
        self.pass_rate = Some(summary.pass_rate);
        self.required_pass_rate = required_pass_rate;
        self.completed_at = Some(Utc::now());

        if summary.pass_rate >= required_pass_rate {
            self.status = OrchestrationStatus::Completed;
            self.error = None;
        } else {
            self.status = OrchestrationStatus::Failed;
            self.error = Some("pass_rate_below_required".to_string());
        }
    }

    pub fn meets_threshold(&self) -> bool {
        self.pass_rate.unwrap_or(0.0) >= self.required_pass_rate
    }
}

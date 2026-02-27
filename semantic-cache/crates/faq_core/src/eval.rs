use crate::model::{Decision, FaqEntry};
use crate::retrieval::decide;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub case_id: String,
    pub question: String,
    pub expected_decision: Decision,
    pub expected_faq_id: Option<String>,
    pub min_similarity: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalOutcome {
    pub case_id: String,
    pub passed: bool,
    pub actual_decision: Decision,
    pub actual_faq_id: Option<String>,
    pub score: f32,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub pass_rate: f32,
    pub outcomes: Vec<EvalOutcome>,
}

pub struct CaseExpectation;

impl CaseExpectation {
    pub fn matches(
        expected_decision: Decision,
        expected_faq_id: Option<&str>,
        min_similarity: Option<f32>,
        actual_decision: Decision,
        actual_faq_id: Option<&str>,
        score: f32,
    ) -> bool {
        if expected_decision != actual_decision {
            return false;
        }

        if let Some(expected) = expected_faq_id {
            if actual_faq_id != Some(expected) {
                return false;
            }
        }

        if let Some(min_sim) = min_similarity {
            if score < min_sim {
                return false;
            }
        }

        true
    }
}

pub fn evaluate_cases<E>(
    embedder: &E,
    entries: &[FaqEntry],
    cases: &[EvalCase],
    threshold: f32,
) -> anyhow::Result<EvalSummary>
where
    E: crate::embed::EmbeddingProvider,
{
    let mut outcomes = Vec::with_capacity(cases.len());

    for case in cases {
        let start = Instant::now();
        let query_embedding = embedder.embed(&case.question)?;
        let result = decide(&query_embedding, entries, threshold);
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

        let passed = CaseExpectation::matches(
            case.expected_decision,
            case.expected_faq_id.as_deref(),
            case.min_similarity,
            result.decision,
            result.entry_id.as_deref(),
            result.score,
        );

        outcomes.push(EvalOutcome {
            case_id: case.case_id.clone(),
            passed,
            actual_decision: result.decision,
            actual_faq_id: result.entry_id,
            score: result.score,
            latency_ms,
        });
    }

    let total = outcomes.len();
    let passed = outcomes.iter().filter(|o| o.passed).count();
    let failed = total.saturating_sub(passed);
    let pass_rate = if total == 0 {
        0.0
    } else {
        passed as f32 / total as f32
    };

    Ok(EvalSummary {
        total,
        passed,
        failed,
        pass_rate,
        outcomes,
    })
}

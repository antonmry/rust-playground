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

/// Deserialise a field that can be absent, null, or a string.
/// Absent → `None`, null → `Some(None)`, `"text"` → `Some(Some("text"))`.
fn deserialize_optional_nullable_string<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // If the key is present in JSON, this function is called.
    // null → Value::Null → Ok(Some(None))
    // "text" → Value::String → Ok(Some(Some("text")))
    let val: Option<String> = Option::deserialize(deserializer)?;
    Ok(Some(val))
}

/// Raw deserialization target that accepts both the old FAQ-ID format and the
/// new expected-answer format.  Call `into_eval_case()` to normalise.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvalCase {
    pub case_id: String,
    // Old format uses "question", new format uses "input_question"
    pub question: Option<String>,
    pub input_question: Option<String>,
    // Old format fields
    pub expected_decision: Option<Decision>,
    pub expected_faq_id: Option<String>,
    // New format: absent → None, null → Some(None) (Miss), "text" → Some(Some(text)) (Hit).
    #[serde(default, deserialize_with = "deserialize_optional_nullable_string")]
    pub expected_answer: Option<Option<String>>,
    pub min_similarity: Option<f32>,
}

impl RawEvalCase {
    pub fn into_eval_case(self) -> anyhow::Result<EvalCase> {
        let question = self.question.or(self.input_question).ok_or_else(|| {
            anyhow::anyhow!(
                "case {}: missing 'question' or 'input_question'",
                self.case_id
            )
        })?;

        if let Some(decision) = self.expected_decision {
            // Old format: expected_decision is explicit
            return Ok(EvalCase {
                case_id: self.case_id,
                question,
                expected_decision: decision,
                expected_faq_id: self.expected_faq_id,
                min_similarity: self.min_similarity,
            });
        }

        // New format: derive decision from expected_answer
        // Some(None) means null → Miss, Some(Some(_)) means non-null → Hit
        if let Some(maybe_answer) = &self.expected_answer {
            let decision = if maybe_answer.is_some() {
                Decision::Hit
            } else {
                Decision::Miss
            };
            return Ok(EvalCase {
                case_id: self.case_id,
                question,
                expected_decision: decision,
                expected_faq_id: None,
                min_similarity: self.min_similarity,
            });
        }

        anyhow::bail!(
            "case {}: must have either 'expected_decision' (old format) or 'expected_answer' (new format)",
            self.case_id
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalOutcome {
    pub case_id: String,
    pub passed: bool,
    pub actual_decision: Decision,
    pub actual_faq_id: Option<String>,
    pub actual_answer: Option<String>,
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
            actual_answer: result.answer,
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

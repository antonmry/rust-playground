use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaqEntry {
    pub id: String,
    pub question: String,
    pub answer: String,
    pub embedding: Vec<f32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub product: Option<String>,
    pub locale: Option<String>,
    pub tags: Vec<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Hit,
    Miss,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalMatch {
    pub entry_id: Option<String>,
    pub answer: Option<String>,
    pub score: f32,
    pub decision: Decision,
}

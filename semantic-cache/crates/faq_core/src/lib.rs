pub mod candle_embed;
pub mod cluster;
pub mod embed;
pub mod eval;
pub mod minilm_embed;
pub mod model;
pub mod orchestration;
pub mod qwen3_embed;
pub mod retrieval;
pub mod storage;

pub use candle_embed::CandleEmbeddingProvider;
pub use cluster::{cluster_questions, read_squad_parquet, QuestionCluster, SquadRow};
pub use embed::{EmbeddingProvider, HashEmbeddingProvider};
pub use eval::{evaluate_cases, CaseExpectation, EvalCase, EvalOutcome, EvalSummary, RawEvalCase};
pub use minilm_embed::MiniLmEmbeddingProvider;
pub use model::{Decision, FaqEntry, RetrievalMatch};
pub use orchestration::{
    CandleEvaluationRun, OrchestrationStatus, DEFAULT_EMBEDDING_DIM, DEFAULT_MODEL_ID,
    DEFAULT_MODEL_PATH, DEFAULT_MODEL_REVISION, DEFAULT_REQUIRED_PASS_RATE, DEFAULT_THRESHOLD,
};
pub use qwen3_embed::Qwen3EmbeddingProvider;
pub use retrieval::{cosine_similarity, decide, top_k, top_match};
pub use storage::{load_entries_jsonl, save_entries_jsonl};

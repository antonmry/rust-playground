use std::path::Path;

use anyhow::{Context, Result};
use arrow::array::{Array, AsArray, RecordBatch};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::embed::EmbeddingProvider;
use crate::retrieval::cosine_similarity;

/// A single row extracted from a SQuAD-style parquet file.
#[derive(Debug, Clone)]
pub struct SquadRow {
    pub id: String,
    pub title: String,
    pub context: String,
    pub question: String,
    pub answer_texts: Vec<String>,
}

/// A cluster of similar questions.
#[derive(Debug, Clone)]
pub struct QuestionCluster {
    /// Index of the representative question (first one added).
    pub representative: usize,
    /// Indices into the original row list.
    pub members: Vec<usize>,
    /// Centroid embedding (mean of member embeddings).
    pub centroid: Vec<f32>,
}

/// Read all rows from a SQuAD v2 parquet file.
pub fn read_squad_parquet(path: &Path) -> Result<Vec<SquadRow>> {
    let file =
        std::fs::File::open(path).with_context(|| format!("open parquet: {}", path.display()))?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file).context("build parquet reader")?;
    let reader = builder.build().context("open parquet batch reader")?;

    let mut rows = Vec::new();
    for batch_result in reader {
        let batch: RecordBatch = batch_result.context("read parquet batch")?;
        let n = batch.num_rows();

        let id_col = batch
            .column_by_name("id")
            .context("missing column 'id'")?
            .as_string::<i32>();
        let title_col = batch
            .column_by_name("title")
            .context("missing column 'title'")?
            .as_string::<i32>();
        let context_col = batch
            .column_by_name("context")
            .context("missing column 'context'")?
            .as_string::<i32>();
        let question_col = batch
            .column_by_name("question")
            .context("missing column 'question'")?
            .as_string::<i32>();

        // answers is a struct { text: list<string>, answer_start: list<int32> }
        let answers_col = batch
            .column_by_name("answers")
            .context("missing column 'answers'")?;
        let answers_struct = answers_col.as_struct();
        let text_list_col = answers_struct
            .column_by_name("text")
            .context("missing answers.text")?;
        let text_list = text_list_col.as_list::<i32>();

        for i in 0..n {
            let answer_texts: Vec<String> = if text_list.is_valid(i) {
                let values = text_list.value(i);
                let str_arr = values.as_string::<i32>();
                (0..str_arr.len())
                    .filter_map(|j| {
                        if str_arr.is_valid(j) {
                            Some(str_arr.value(j).to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            rows.push(SquadRow {
                id: id_col.value(i).to_string(),
                title: title_col.value(i).to_string(),
                context: context_col.value(i).to_string(),
                question: question_col.value(i).to_string(),
                answer_texts,
            });
        }
    }

    Ok(rows)
}

/// Greedy single-pass clustering.
///
/// Each question is embedded, then assigned to the cluster whose centroid is
/// most similar (above `threshold`). If no cluster qualifies, a new one is
/// created. The centroid is updated as a running mean after each assignment.
pub fn cluster_questions(
    rows: &[SquadRow],
    embedder: &dyn EmbeddingProvider,
    threshold: f32,
) -> Result<Vec<QuestionCluster>> {
    let mut clusters: Vec<QuestionCluster> = Vec::new();
    let total = rows.len();

    for (i, row) in rows.iter().enumerate() {
        if (i + 1) % 500 == 0 || i + 1 == total {
            eprintln!("  embedding {}/{} ...", i + 1, total);
        }

        let emb = embedder
            .embed(&row.question)
            .with_context(|| format!("embed question {}", i))?;

        // Find closest cluster
        let mut best_idx: Option<usize> = None;
        let mut best_sim: f32 = threshold;

        for (ci, cluster) in clusters.iter().enumerate() {
            let sim = cosine_similarity(&emb, &cluster.centroid);
            if sim > best_sim {
                best_sim = sim;
                best_idx = Some(ci);
            }
        }

        match best_idx {
            Some(ci) => {
                let cluster = &mut clusters[ci];
                cluster.members.push(i);
                // Update centroid as running mean
                let n = cluster.members.len() as f32;
                for (j, val) in emb.iter().enumerate() {
                    cluster.centroid[j] = cluster.centroid[j] * ((n - 1.0) / n) + val / n;
                }
            }
            None => {
                clusters.push(QuestionCluster {
                    representative: i,
                    members: vec![i],
                    centroid: emb,
                });
            }
        }
    }

    // Sort by cluster size descending (most recurring first)
    clusters.sort_by(|a, b| b.members.len().cmp(&a.members.len()));
    Ok(clusters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::HashEmbeddingProvider;

    #[test]
    fn cluster_groups_similar_questions() {
        let rows = vec![
            SquadRow {
                id: "1".into(),
                title: "T".into(),
                context: "C".into(),
                question: "What is the capital of France?".into(),
                answer_texts: vec!["Paris".into()],
            },
            SquadRow {
                id: "2".into(),
                title: "T".into(),
                context: "C".into(),
                question: "What is the capital of France?".into(),
                answer_texts: vec!["Paris".into()],
            },
            SquadRow {
                id: "3".into(),
                title: "T".into(),
                context: "C".into(),
                question: "How tall is Mount Everest?".into(),
                answer_texts: vec!["8849m".into()],
            },
        ];

        let embedder = HashEmbeddingProvider::new(64);
        let clusters = cluster_questions(&rows, &embedder, 0.5).unwrap();

        // The two identical questions should be in the same cluster
        assert!(clusters.len() >= 2);
        let biggest = &clusters[0];
        assert_eq!(biggest.members.len(), 2);
    }
}

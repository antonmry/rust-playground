use std::path::Path;

use anyhow::{Context, Result};
use arrow::array::{Array, AsArray, RecordBatch};
use nalgebra::DMatrix;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};

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

// ---------------------------------------------------------------------------
// Visualization output types
// ---------------------------------------------------------------------------

/// Metadata about a clustering + projection run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterMeta {
    pub input_path: String,
    pub threshold: f32,
    pub projection_method: String,
    pub timestamp: String,
    pub point_count: usize,
}

/// Summary of one cluster for the output JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSummary {
    pub cluster_id: usize,
    pub size: usize,
    pub representative_index: usize,
}

/// A single projected point for JSON/HTML output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectedPoint {
    pub id: String,
    pub question: String,
    pub cluster_id: Option<usize>,
    pub x: f32,
    pub y: f32,
    pub title: String,
    pub answer_preview: String,
    pub score_to_centroid: f32,
}

/// Top-level visualization output schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterVisualization {
    pub meta: ClusterMeta,
    pub clusters: Vec<ClusterSummary>,
    pub points: Vec<ProjectedPoint>,
}

// ---------------------------------------------------------------------------
// Parquet reader
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Clustering
// ---------------------------------------------------------------------------

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
    let total = rows.len();
    let mut embeddings = Vec::with_capacity(total);

    for (i, row) in rows.iter().enumerate() {
        if (i + 1) % 500 == 0 || i + 1 == total {
            eprintln!("  embedding {}/{} ...", i + 1, total);
        }
        embeddings.push(
            embedder
                .embed(&row.question)
                .with_context(|| format!("embed question {}", i))?,
        );
    }

    Ok(cluster_embeddings(&embeddings, threshold))
}

/// Greedy single-pass clustering on pre-computed embeddings.
///
/// `embeddings[i]` corresponds to row `i`. Each embedding is assigned to the
/// cluster whose centroid is most similar (above `threshold`). If none
/// qualifies, a new cluster is created.
pub fn cluster_embeddings(embeddings: &[Vec<f32>], threshold: f32) -> Vec<QuestionCluster> {
    let mut clusters: Vec<QuestionCluster> = Vec::new();

    for (i, emb) in embeddings.iter().enumerate() {
        let mut best_idx: Option<usize> = None;
        let mut best_sim: f32 = threshold;

        for (ci, cluster) in clusters.iter().enumerate() {
            let sim = cosine_similarity(emb, &cluster.centroid);
            if sim > best_sim {
                best_sim = sim;
                best_idx = Some(ci);
            }
        }

        match best_idx {
            Some(ci) => {
                let cluster = &mut clusters[ci];
                cluster.members.push(i);
                let n = cluster.members.len() as f32;
                for (j, val) in emb.iter().enumerate() {
                    cluster.centroid[j] = cluster.centroid[j] * ((n - 1.0) / n) + val / n;
                }
            }
            None => {
                clusters.push(QuestionCluster {
                    representative: i,
                    members: vec![i],
                    centroid: emb.clone(),
                });
            }
        }
    }

    clusters.sort_by(|a, b| b.members.len().cmp(&a.members.len()));
    clusters
}

// ---------------------------------------------------------------------------
// PCA projection
// ---------------------------------------------------------------------------

/// Project high-dimensional embeddings to 2D using PCA via SVD.
///
/// Returns N `(x, y)` pairs corresponding to each input embedding.
pub fn project_pca_2d(embeddings: &[Vec<f32>]) -> Result<Vec<(f32, f32)>> {
    if embeddings.is_empty() {
        return Ok(Vec::new());
    }

    let n = embeddings.len();
    let d = embeddings[0].len();

    if n == 1 {
        return Ok(vec![(0.0, 0.0)]);
    }

    // Build N×D matrix
    let mut data = DMatrix::<f32>::zeros(n, d);
    for (i, emb) in embeddings.iter().enumerate() {
        for (j, &val) in emb.iter().enumerate() {
            data[(i, j)] = val;
        }
    }

    // Center: subtract column means
    let col_means: Vec<f32> = (0..d).map(|j| data.column(j).sum() / n as f32).collect();
    for j in 0..d {
        for i in 0..n {
            data[(i, j)] -= col_means[j];
        }
    }

    // SVD — only need U (left singular vectors)
    let svd = data.svd(true, false);
    let u = svd.u.context("SVD did not compute U matrix")?;

    let s0 = if !svd.singular_values.is_empty() {
        svd.singular_values[0]
    } else {
        1.0
    };
    let s1 = if svd.singular_values.len() > 1 {
        svd.singular_values[1]
    } else {
        1.0
    };

    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        let x = u[(i, 0)] * s0;
        let y = if u.ncols() > 1 { u[(i, 1)] * s1 } else { 0.0 };
        points.push((x, y));
    }

    Ok(points)
}

// ---------------------------------------------------------------------------
// Downsampling
// ---------------------------------------------------------------------------

/// Compute indices for uniform downsampling to at most `max_points`.
pub fn downsample_indices(total: usize, max_points: usize) -> Vec<usize> {
    if total <= max_points {
        return (0..total).collect();
    }
    let step = total as f64 / max_points as f64;
    (0..max_points)
        .map(|i| (i as f64 * step).floor() as usize)
        .collect()
}

// ---------------------------------------------------------------------------
// Visualization builder
// ---------------------------------------------------------------------------

/// Build the full visualization data structure.
pub fn build_visualization(
    rows: &[SquadRow],
    clusters: &[QuestionCluster],
    embeddings: &[Vec<f32>],
    input_path: &str,
    threshold: f32,
) -> Result<ClusterVisualization> {
    // Map row index → cluster index
    let mut row_to_cluster: Vec<Option<usize>> = vec![None; rows.len()];
    for (ci, cluster) in clusters.iter().enumerate() {
        for &member_idx in &cluster.members {
            if member_idx < row_to_cluster.len() {
                row_to_cluster[member_idx] = Some(ci);
            }
        }
    }

    let coords = project_pca_2d(embeddings)?;

    let cluster_summaries: Vec<ClusterSummary> = clusters
        .iter()
        .enumerate()
        .map(|(ci, c)| ClusterSummary {
            cluster_id: ci,
            size: c.members.len(),
            representative_index: c.representative,
        })
        .collect();

    let mut points = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let cluster_id = row_to_cluster[i];
        let score = if let Some(ci) = cluster_id {
            cosine_similarity(&embeddings[i], &clusters[ci].centroid)
        } else {
            0.0
        };
        let answer_preview = row
            .answer_texts
            .first()
            .map(|a| {
                if a.len() > 100 {
                    format!("{}…", &a[..a.floor_char_boundary(100)])
                } else {
                    a.clone()
                }
            })
            .unwrap_or_default();

        let (x, y) = coords[i];
        points.push(ProjectedPoint {
            id: row.id.clone(),
            question: row.question.clone(),
            cluster_id,
            x,
            y,
            title: row.title.clone(),
            answer_preview,
            score_to_centroid: score,
        });
    }

    let meta = ClusterMeta {
        input_path: input_path.to_string(),
        threshold,
        projection_method: "pca".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        point_count: points.len(),
    };

    Ok(ClusterVisualization {
        meta,
        clusters: cluster_summaries,
        points,
    })
}

// ---------------------------------------------------------------------------
// HTML scatter renderer
// ---------------------------------------------------------------------------

/// Render a standalone HTML scatter plot from a `ClusterVisualization`.
///
/// Uses Plotly.js via CDN with the JSON data inlined.
pub fn render_html_scatter(viz: &ClusterVisualization) -> Result<String> {
    let json_data = serde_json::to_string(viz).context("serialize visualization to JSON")?;

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Cluster Visualization</title>
<script src="https://cdn.plot.ly/plotly-2.35.0.min.js"></script>
<style>
  body {{ font-family: sans-serif; margin: 20px; }}
  #plot {{ width: 100%; height: 80vh; }}
  .meta {{ color: #666; font-size: 0.9em; margin-bottom: 10px; }}
</style>
</head>
<body>
<h2>Cluster Scatter Plot</h2>
<div class="meta">
  Input: {input} | Threshold: {threshold} | Points: {count} | Projection: {proj} | Generated: {ts}
</div>
<div id="plot"></div>
<script>
const data = {json};

// Group points by cluster_id
const clusters = {{}};
data.points.forEach(p => {{
  const key = p.cluster_id === null ? 'outlier' : 'Cluster ' + p.cluster_id;
  if (!clusters[key]) clusters[key] = {{ x: [], y: [], text: [] }};
  clusters[key].x.push(p.x);
  clusters[key].y.push(p.y);
  clusters[key].text.push(
    '<b>' + p.question.replace(/</g, '&lt;') + '</b><br>' +
    'Cluster: ' + (p.cluster_id === null ? 'none' : p.cluster_id) + '<br>' +
    'Title: ' + p.title.replace(/</g, '&lt;') + '<br>' +
    'Answer: ' + p.answer_preview.replace(/</g, '&lt;') + '<br>' +
    'Similarity: ' + p.score_to_centroid.toFixed(4)
  );
}});

const traces = Object.entries(clusters).map(([name, c]) => ({{
  x: c.x, y: c.y, text: c.text,
  mode: 'markers',
  type: 'scatter',
  name: name + ' (' + c.x.length + ')',
  hoverinfo: 'text',
  marker: {{ size: 6, opacity: 0.7 }}
}}));

Plotly.newPlot('plot', traces, {{
  title: 'Question Clusters (PCA projection)',
  xaxis: {{ title: 'PC1' }},
  yaxis: {{ title: 'PC2' }},
  hovermode: 'closest',
  legend: {{ title: {{ text: 'Clusters' }} }}
}}, {{ responsive: true }});
</script>
</body>
</html>"#,
        input = viz.meta.input_path,
        threshold = viz.meta.threshold,
        count = viz.meta.point_count,
        proj = viz.meta.projection_method,
        ts = viz.meta.timestamp,
        json = json_data,
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

        assert!(clusters.len() >= 2);
        let biggest = &clusters[0];
        assert_eq!(biggest.members.len(), 2);
    }

    #[test]
    fn test_project_pca_2d_basic() {
        let embeddings = vec![
            vec![1.0, 0.0, 0.0],
            vec![1.1, 0.1, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.1, 1.1, 0.0],
        ];

        let points = project_pca_2d(&embeddings).unwrap();
        assert_eq!(points.len(), 4);

        // Pair (0,1) should be closer than (0,2)
        let d01 =
            ((points[0].0 - points[1].0).powi(2) + (points[0].1 - points[1].1).powi(2)).sqrt();
        let d02 =
            ((points[0].0 - points[2].0).powi(2) + (points[0].1 - points[2].1).powi(2)).sqrt();
        assert!(d01 < d02, "near pair should be closer: d01={d01} d02={d02}");
    }

    #[test]
    fn test_project_pca_2d_empty() {
        let points = project_pca_2d(&[]).unwrap();
        assert!(points.is_empty());
    }

    #[test]
    fn test_project_pca_2d_single_point() {
        let points = project_pca_2d(&[vec![1.0, 2.0, 3.0]]).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], (0.0, 0.0));
    }

    #[test]
    fn test_downsample_indices() {
        let idx = downsample_indices(100, 10);
        assert_eq!(idx.len(), 10);
        assert_eq!(idx[0], 0);

        let idx = downsample_indices(5, 100);
        assert_eq!(idx, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_render_html_contains_plotly() {
        let viz = ClusterVisualization {
            meta: ClusterMeta {
                input_path: "test.parquet".into(),
                threshold: 0.8,
                projection_method: "pca".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                point_count: 1,
            },
            clusters: vec![ClusterSummary {
                cluster_id: 0,
                size: 1,
                representative_index: 0,
            }],
            points: vec![ProjectedPoint {
                id: "1".into(),
                question: "test?".into(),
                cluster_id: Some(0),
                x: 0.0,
                y: 0.0,
                title: "T".into(),
                answer_preview: "A".into(),
                score_to_centroid: 1.0,
            }],
        };

        let html = render_html_scatter(&viz).unwrap();
        assert!(html.contains("plotly"));
        assert!(html.contains("<html"));
        assert!(html.contains("test?"));
    }
}

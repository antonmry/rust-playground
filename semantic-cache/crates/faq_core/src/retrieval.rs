use crate::model::{Decision, FaqEntry, RetrievalMatch};

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let (dot, na, nb) = a
        .iter()
        .zip(b.iter())
        .fold((0.0f32, 0.0f32, 0.0f32), |(d, aa, bb), (x, y)| {
            (d + (x * y), aa + (x * x), bb + (y * y))
        });

    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

pub fn top_k<'a>(
    query_embedding: &[f32],
    entries: &'a [FaqEntry],
    k: usize,
) -> Vec<(&'a FaqEntry, f32)> {
    let mut scored: Vec<(&FaqEntry, f32)> = entries
        .iter()
        .map(|entry| (entry, cosine_similarity(query_embedding, &entry.embedding)))
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().take(k).collect()
}

pub fn top_match<'a>(
    query_embedding: &[f32],
    entries: &'a [FaqEntry],
) -> Option<(&'a FaqEntry, f32)> {
    top_k(query_embedding, entries, 1).into_iter().next()
}

pub fn decide(query_embedding: &[f32], entries: &[FaqEntry], threshold: f32) -> RetrievalMatch {
    match top_match(query_embedding, entries) {
        Some((entry, score)) if score >= threshold => RetrievalMatch {
            entry_id: Some(entry.id.clone()),
            answer: Some(entry.answer.clone()),
            score,
            decision: Decision::Hit,
        },
        Some((entry, score)) => RetrievalMatch {
            entry_id: Some(entry.id.clone()),
            answer: None,
            score,
            decision: Decision::Miss,
        },
        None => RetrievalMatch {
            entry_id: None,
            answer: None,
            score: 0.0,
            decision: Decision::Miss,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FaqEntry;
    use chrono::Utc;

    fn mk_entry(id: &str, emb: Vec<f32>) -> FaqEntry {
        FaqEntry {
            id: id.to_string(),
            question: String::new(),
            answer: format!("answer-{id}"),
            embedding: emb,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            expires_at: None,
            product: None,
            locale: None,
            tags: Vec::new(),
            version: None,
            source: None,
            verified: None,
        }
    }

    #[test]
    fn cosine_works_for_unit_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let c = vec![0.0, 1.0, 0.0];

        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
        assert!((cosine_similarity(&a, &c) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn top_match_selects_best_entry() {
        let entries = vec![
            mk_entry("e1", vec![1.0, 0.0]),
            mk_entry("e2", vec![0.0, 1.0]),
        ];
        let (entry, score) = top_match(&[0.9, 0.1], &entries).expect("match");

        assert_eq!(entry.id, "e1");
        assert!(score > 0.9);
    }

    #[test]
    fn decide_applies_threshold() {
        let entries = vec![mk_entry("e1", vec![1.0, 0.0])];

        let hit = decide(&[1.0, 0.0], &entries, 0.8);
        assert_eq!(hit.decision, Decision::Hit);
        assert_eq!(hit.answer.as_deref(), Some("answer-e1"));

        let miss = decide(&[0.2, 0.9], &entries, 0.8);
        assert_eq!(miss.decision, Decision::Miss);
        assert_eq!(miss.answer, None);
        assert_eq!(miss.entry_id.as_deref(), Some("e1"));
    }
}

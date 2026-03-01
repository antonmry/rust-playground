# Open Questions — Semantic FAQ Cache

## Data & input

1. **What dataset should we use for real evaluation?** SQuAD v2 is a reading
   comprehension dataset — its "clusters" are mostly adversarial pairs, not
   organically repeated questions. A real FAQ use case needs actual user
   questions (support tickets, chatbot logs, search queries).

2. **What time window should be used?** Should the cache consider all historical
   questions, or only recent ones (e.g., last 30/90 days)? Older questions may
   reflect outdated products or policies.

3. **Can we detect whether an FAQ has a transient component?** Some questions
   are seasonal or event-driven ("What are Black Friday hours?"). We need a
   mechanism to set a maximum TTL or expiry date per FAQ entry so stale answers
   get purged automatically.

## Clustering & threshold tuning

4. **What similarity threshold is right for our domain?** Experiments show:
   - 0.80 → topic-level grouping (too broad for FAQ matching)
   - 0.90 → thematic clusters (good for FAQ candidate extraction)
   - 0.95 → true paraphrases (good for deduplication)
The right value depends on how strictly we want to match before returning a
cached answer.

5. **How easy is it to generate the list of most recurring questions?** The
   `cluster` command already does this — it groups questions by similarity and
   ranks by cluster size. The top clusters are the most-asked questions. For
   production, we need to feed it real user queries instead of SQuAD data.

6. **Should we support threshold sweeps?** Instead of picking a single
   threshold, the tool could run at multiple thresholds and show how cluster
   count/size changes — helping operators find the sweet spot for their domain.

## Answer quality & verification

7. **How do we ensure the answer is correct?** Options:
   - **Human review**: sample interactions where the cached answer was served,
     check for user confirmation/satisfaction signals.
   - **LLM verification**: send the question + cached answer to an LLM with a
     prompt like "Does this answer address the question? Reply YES or NO."
     Single-token response keeps cost low.
   - **Feedback loop**: track whether users ask follow-up questions after
     receiving a cached answer (indicates the answer was insufficient).

8. **Similarity scores from retrieval may be low — how do we handle borderline
   cases?** A question may semantically match an FAQ but the cosine score hovers
   near the threshold. Two options:
   - **Dual-gate**: require both a minimum similarity score AND an LLM
     confirmation that the answer addresses the question.
   - **Tiered response**: high similarity → serve cached answer directly; medium
     similarity → serve with "Did this help?" prompt; low similarity → don't
     serve.

## Architecture & deployment

9. **Should the index be pre-built or dynamic?** Current design pre-builds the
   index from a JSONL file. For production, we may need to add/remove FAQ
   entries without rebuilding the entire index.

10. **How do we handle multiple languages?** MiniLM is English-only. For
    multilingual support, we'd need a multilingual model (e.g.,
    `paraphrase-multilingual-MiniLM-L12-v2`) or per-language indexes.

11. **What's the latency budget?** Current query path: embed question
    (~~0ms for MiniLM) + brute-force cosine scan ~~<1ms for <10K entries). For
    larger indexes, we may need approximate nearest neighbor (ANN) search.

12. **How do we version FAQ entries?** If an answer changes (e.g., updated
    pricing), do we keep the old version? Do we need an audit trail?

## Original questions

- What time window should be used?
- Can we detect whether an FAQ has a transient component, so that we can set a
  maximum date to delete that query from the database?
- How easy is it to generate the list of most recurring questions?
- How do we ensure the answer is correct? (One possible approach is to review
  interactions with some users, looking for confirmation.)
- Similarity scores from the search may be low. Another confirmation option
  would be to send the full solution content along with a prompt so the LLM can
  tell us whether the answer actually matches the question, returning a
  single-token response.

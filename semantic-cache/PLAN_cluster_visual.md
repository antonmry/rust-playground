# Cluster Visualization Plan

## Goal

Add a scatter-plot visualization for `faq_cli cluster` so we can inspect semantic proximity between questions after clustering.

## User Outcome

After running `cluster`, users should be able to:

1. Open an interactive plot.
2. See each question as one point.
3. Understand semantic closeness by distance on the chart.
4. Identify cluster quality visually (tight groups, overlap, outliers).

## Proposed CLI Additions

Add optional flags to `cluster`:

1. `--json-out <path>`: write structured clustering + projection output.
2. `--plot-out <path>`: write a standalone HTML scatter plot.
3. `--projection <pca|tsne|umap>`: choose 2D projection method.
4. `--max-points <n>`: optional downsampling cap for very large datasets.

If neither `--json-out` nor `--plot-out` is provided, keep current text output behavior unchanged.

## Data Flow

1. Read parquet rows (existing).
2. Compute embeddings (existing).
3. Cluster embeddings (existing).
4. Project embeddings to 2D (new).
5. Build output records (new) with:
   - row id
   - question text
   - cluster id
   - centroid similarity
   - x, y coordinates
6. Write JSON (new, optional).
7. Render HTML scatter from JSON (new, optional).

## Output Schema (JSON)

Top-level fields:

1. `meta`: input path, threshold, projection method, timestamp, point count.
2. `clusters`: cluster id, size, representative row index/id.
3. `points`: one record per question with `id`, `question`, `cluster_id`, `x`, `y`, and optional metadata (`title`, `answer_preview`, `score_to_centroid`).

## HTML Plot Behavior

1. Color points by `cluster_id`.
2. Hover tooltip shows `question`, `cluster_id`, and similarity metadata.
3. Zoom/pan enabled.
4. Legend with cluster sizes.
5. Outliers or singleton clusters remain visible.

Implementation options:

1. Lightweight standalone HTML + embedded JS + inline JSON.
2. Use a browser library via CDN (e.g., Plotly) for fast implementation.

## Projection Strategy

Phase 1:

1. Implement `PCA` first (deterministic, low complexity).
2. Keep interface projection-ready for `t-SNE`/`UMAP`.

Phase 2:

1. Add `UMAP` (preferred neighborhood preservation for cluster inspection).
2. Add `t-SNE` if needed for separation-focused diagnostics.

## Performance Considerations

1. For very large datasets, projection can be expensive; support `--max-points`.
2. Keep raw embeddings out of HTML to reduce file size.
3. Include a clear warning when downsampling is active.

## Backward Compatibility

1. Existing `cluster` usage continues to work without changes.
2. Existing text output remains default.
3. New outputs are opt-in flags.

## Validation

1. Unit tests for JSON serialization shape.
2. Unit tests for deterministic projection output (where applicable).
3. CLI integration test:
   - run `cluster --json-out ... --plot-out ...`
   - assert files exist and required fields are present.
4. Manual validation:
   - open HTML
   - hover points
   - verify cluster colors and counts match text summary.

## Deliverables

1. CLI support for `--json-out`, `--plot-out`, `--projection`, `--max-points`.
2. JSON export implementation.
3. Standalone HTML scatter renderer.
4. README updates with example command and screenshot guidance.

## Open Decisions

1. Default projection method (`pca` vs `umap`).
2. Whether to include all metadata in tooltips by default or behind a compact mode.
3. Whether HTML should bundle JSON inline or load a sidecar file.

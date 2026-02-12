use std::path::Path;

use anyhow::{Context, Result};
use datafusion::arrow::json::ArrayWriter;
use datafusion::prelude::*;

pub struct SqlExecutor {
    pub ctx: SessionContext,
}

impl SqlExecutor {
    pub async fn new() -> Result<Self> {
        let config = SessionConfig::new().with_information_schema(true);
        let ctx = SessionContext::new_with_config(config);
        Ok(Self { ctx })
    }

    pub async fn register_csv(&self, table: &str, path: &str) -> Result<()> {
        self.ctx
            .register_csv(table, path, CsvReadOptions::default())
            .await
            .with_context(|| format!("Failed to register CSV file '{path}' as table '{table}'"))
    }

    pub async fn register_parquet(&self, table: &str, path: &str) -> Result<()> {
        self.ctx
            .register_parquet(table, path, ParquetReadOptions::default())
            .await
            .with_context(|| format!("Failed to register Parquet file '{path}' as table '{table}'"))
    }

    pub async fn register_json(&self, table: &str, path: &str) -> Result<()> {
        self.ctx
            .register_json(table, path, NdJsonReadOptions::default())
            .await
            .with_context(|| format!("Failed to register JSON file '{path}' as table '{table}'"))
    }

    pub async fn register_file(&self, table: &str, path: &str) -> Result<()> {
        if !Path::new(path).exists() {
            anyhow::bail!("File not found: '{path}'");
        }

        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        match ext.as_deref() {
            Some("csv") => self.register_csv(table, path).await,
            Some("parquet") | Some("pq") => self.register_parquet(table, path).await,
            Some("json") | Some("ndjson") => self.register_json(table, path).await,
            Some("avro") => {
                anyhow::bail!(
                    "Avro registration is not available in the current offline build environment"
                )
            }
            Some(other) => anyhow::bail!("Unsupported file format '.{other}'. Supported: .csv, .parquet, .pq, .json, .ndjson, .avro"),
            None => anyhow::bail!("File '{path}' has no extension. Cannot determine format."),
        }
    }

    pub async fn execute_sql(&self, sql: &str) -> Result<DataFrame> {
        self.ctx
            .sql(sql)
            .await
            .with_context(|| format!("Failed to execute SQL: {sql}"))
    }

    pub async fn execute_sql_json(&self, sql: &str) -> Result<String> {
        let df = self.execute_sql(sql).await?;
        let batches = df.collect().await.context("Failed to collect results")?;
        let refs = batches.iter().collect::<Vec<_>>();
        let mut writer = ArrayWriter::new(Vec::new());
        writer.write_batches(&refs)?;
        writer.finish()?;
        let out = writer.into_inner();
        let json = String::from_utf8(out).context("JSON output was not valid UTF-8")?;
        Ok(json)
    }
}

pub fn is_mutation_sql(sql: &str) -> bool {
    let trimmed = sql.trim();

    // Skip leading comments
    let effective = skip_comments(trimmed);

    // EXPLAIN should not be flagged
    if effective.starts_with("EXPLAIN ") || effective.starts_with("EXPLAIN\n") {
        return false;
    }

    // Check for WITH ... <mutation> pattern
    if effective.starts_with("WITH ") || effective.starts_with("WITH\n") {
        let upper = effective.to_uppercase();
        for kw in MUTATION_KEYWORDS {
            if upper.contains(kw) && !is_keyword_in_identifier_context(&upper, kw) {
                return true;
            }
        }
        return false;
    }

    // Check first keyword
    let first_word = effective
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("")
        .to_uppercase();

    MUTATION_KEYWORDS.contains(&first_word.as_str())
}

const MUTATION_KEYWORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "TRUNCATE", "COPY", "IMPORT",
    "EXPORT", "ATTACH", "DETACH",
];

fn skip_comments(s: &str) -> &str {
    let mut rest = s;
    loop {
        rest = rest.trim_start();
        if rest.starts_with("--") {
            if let Some(pos) = rest.find('\n') {
                rest = &rest[pos + 1..];
            } else {
                return "";
            }
        } else if rest.starts_with("/*") {
            if let Some(pos) = rest.find("*/") {
                rest = &rest[pos + 2..];
            } else {
                return "";
            }
        } else {
            return rest;
        }
    }
}

fn is_keyword_in_identifier_context(upper: &str, keyword: &str) -> bool {
    // Check if keyword appears only as part of an identifier (e.g., "insert_log")
    for (idx, _) in upper.match_indices(keyword) {
        let before = if idx > 0 {
            upper.as_bytes()[idx - 1]
        } else {
            b' '
        };
        let after_idx = idx + keyword.len();
        let after = if after_idx < upper.len() {
            upper.as_bytes()[after_idx]
        } else {
            b' '
        };

        let before_is_boundary = !before.is_ascii_alphanumeric() && before != b'_';
        let after_is_boundary = !after.is_ascii_alphanumeric() && after != b'_';

        if before_is_boundary && after_is_boundary {
            return false; // Found a standalone keyword usage
        }
    }
    true // All occurrences are part of identifiers
}

/// Parse a file spec of the form `[table_name=]path` into (table_name, path).
pub fn parse_file_spec(spec: &str) -> Result<(String, String)> {
    if let Some((name, path)) = spec.split_once('=') {
        if name.is_empty() {
            anyhow::bail!("Empty table name in file spec: '{spec}'");
        }
        Ok((name.to_string(), path.to_string()))
    } else {
        let path = Path::new(spec);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Cannot determine table name from path: '{spec}'"))?;
        Ok((stem.to_string(), spec.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_data_path(name: &str) -> String {
        format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[tokio::test]
    async fn test_csv_select() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        let json = exec
            .execute_sql_json("SELECT * FROM products ORDER BY id")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["name"], "Widget");
    }

    #[tokio::test]
    async fn test_parquet_select() {
        generate_test_parquet();
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_parquet("sales", &test_data_path("sales.parquet"))
            .await
            .unwrap();
        let df = exec
            .execute_sql("SELECT COUNT(*) AS cnt FROM sales")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();
        assert_eq!(batches[0].num_rows(), 1);
    }

    #[tokio::test]
    async fn test_json_select() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_json("events", &test_data_path("events.json"))
            .await
            .unwrap();
        let json = exec.execute_sql_json("SELECT * FROM events").await.unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(!rows.is_empty());
    }

    #[tokio::test]
    async fn test_execute_sql_returns_dataframe() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        let df = exec.execute_sql("SELECT name FROM products").await.unwrap();
        let batches = df.collect().await.unwrap();
        assert!(batches[0].num_rows() > 0);
    }

    #[tokio::test]
    async fn test_json_output_format() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        let json = exec
            .execute_sql_json("SELECT id, name FROM products")
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_array());
    }

    #[test]
    fn test_mutation_detection() {
        assert!(is_mutation_sql("INSERT INTO t VALUES (1)"));
        assert!(is_mutation_sql("  insert INTO t VALUES (1)"));
        assert!(is_mutation_sql("UPDATE t SET x = 1"));
        assert!(is_mutation_sql("DELETE FROM t WHERE id = 1"));
        assert!(is_mutation_sql("DROP TABLE t"));
        assert!(is_mutation_sql("ALTER TABLE t ADD COLUMN x INT"));
        assert!(is_mutation_sql("CREATE TABLE t (x INT)"));
        assert!(is_mutation_sql("TRUNCATE TABLE t"));
        // CTE with mutation
        assert!(is_mutation_sql(
            "WITH cte AS (SELECT 1) INSERT INTO t SELECT * FROM cte"
        ));
    }

    #[test]
    fn test_non_mutation() {
        assert!(!is_mutation_sql("SELECT * FROM t"));
        assert!(!is_mutation_sql("  select 1"));
        assert!(!is_mutation_sql("SHOW TABLES"));
        assert!(!is_mutation_sql("EXPLAIN SELECT 1"));
        assert!(!is_mutation_sql("WITH cte AS (SELECT 1) SELECT * FROM cte"));
    }

    #[tokio::test]
    async fn test_invalid_sql_error() {
        let exec = SqlExecutor::new().await.unwrap();
        let result = exec.execute_sql_json("NOT VALID SQL AT ALL").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_result() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        let json = exec
            .execute_sql_json("SELECT * FROM products WHERE id = 9999")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(rows.len(), 0);
    }

    #[tokio::test]
    async fn test_information_schema() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        let json = exec
            .execute_sql_json(
                "SELECT table_name FROM information_schema.tables WHERE table_name = 'products'",
            )
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(rows.len(), 1);
    }

    // Corner cases

    #[tokio::test]
    async fn test_long_query() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        let long_sql = format!(
            "SELECT * FROM products WHERE name = '{}'",
            "a".repeat(10_000)
        );
        let result = exec.execute_sql_json(&long_sql).await;
        // Should either succeed (empty result) or return a clear error — must not panic
        if let Ok(json) = result {
            let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
            assert_eq!(rows.len(), 0);
        }
    }

    #[tokio::test]
    async fn test_unicode_data() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products_unicode.csv"))
            .await
            .unwrap();
        let json = exec
            .execute_sql_json("SELECT * FROM products ORDER BY id")
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert!(rows[0]["name"].as_str().unwrap().contains("日本語"));
    }

    #[tokio::test]
    async fn test_multiple_statements() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();
        // DataFusion handles only one statement at a time via ctx.sql()
        let result = exec.execute_sql_json("SELECT 1; SELECT 2").await;
        // May error — that's acceptable; must not panic
        let _ = result;
    }

    #[tokio::test]
    async fn test_concurrent_queries() {
        let exec = Arc::new(SqlExecutor::new().await.unwrap());
        exec.register_csv("products", &test_data_path("products.csv"))
            .await
            .unwrap();

        let mut handles = vec![];
        for i in 0..10 {
            let exec = exec.clone();
            handles.push(tokio::spawn(async move {
                exec.execute_sql_json(&format!("SELECT {i} AS n"))
                    .await
                    .unwrap()
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_large_result_set() {
        let exec = SqlExecutor::new().await.unwrap();
        let df = exec
            .execute_sql("SELECT value FROM generate_series(1, 10000)")
            .await
            .unwrap();
        let batches = df.collect().await.unwrap();
        let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(row_count, 10_000);
    }

    #[tokio::test]
    async fn test_empty_csv() {
        let exec = SqlExecutor::new().await.unwrap();
        exec.register_csv("empty", &test_data_path("empty.csv"))
            .await
            .unwrap();
        let json = exec.execute_sql_json("SELECT * FROM empty").await.unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn test_mutation_detection_tricky() {
        // Leading whitespace and mixed case
        assert!(is_mutation_sql("   INSERT INTO t VALUES (1)"));
        assert!(is_mutation_sql("\n\tDELETE FROM t"));

        // Keyword as table name (should NOT be flagged as mutation)
        assert!(!is_mutation_sql("SELECT * FROM insert_log"));
        assert!(!is_mutation_sql("SELECT delete_count FROM stats"));

        // Comments before statement
        assert!(is_mutation_sql("-- comment\nINSERT INTO t VALUES (1)"));

        // EXPLAIN should not be flagged
        assert!(!is_mutation_sql("EXPLAIN INSERT INTO t VALUES (1)"));
    }

    /// Generate the test parquet file
    fn generate_test_parquet() {
        use arrow::array::{Float64Array, Int64Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;
        use std::fs::File;

        let path = format!("{}/tests/data/sales.parquet", env!("CARGO_MANIFEST_DIR"));
        if Path::new(&path).exists() {
            return;
        }

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("product_id", DataType::Int64, false),
            Field::new("amount", DataType::Float64, false),
            Field::new("sale_date", DataType::Utf8, false),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
                Arc::new(Int64Array::from(vec![1, 2, 1, 3, 2])),
                Arc::new(Float64Array::from(vec![
                    99.99, 149.99, 99.99, 24.99, 149.99,
                ])),
                Arc::new(StringArray::from(vec![
                    "2024-01-15",
                    "2024-01-16",
                    "2024-01-17",
                    "2024-01-18",
                    "2024-01-19",
                ])),
            ],
        )
        .unwrap();

        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }
}

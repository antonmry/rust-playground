use std::sync::Arc;

use axum::Router;
use rmcp::{
    handler::server::ServerHandler,
    model::*,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use crate::sql_executor::{is_mutation_sql, SqlExecutor};

#[derive(Debug, Clone)]
pub struct FinalQueryResult {
    pub sql: String,
    pub summary: Option<String>,
    pub datasources: Option<String>,
}

fn get_instructions(safe_mode: bool) -> String {
    let restrictions = if safe_mode {
        "RESTRICTIONS (READ-ONLY MODE):\n\
        - ONLY use SELECT queries to read data\n\
        - DO NOT use INSERT, UPDATE, DELETE, DROP, ALTER, CREATE, TRUNCATE\n\
        - DO NOT use COPY, IMPORT, EXPORT, ATTACH, DETACH\n\
        - DO NOT modify schema or data in any way\n\n"
    } else {
        "MODE: Full access (mutations allowed)\n\
        You may use any SQL statements including INSERT, UPDATE, DELETE, CREATE, etc.\n\n"
    };

    let catalog_exploration = "\
        CATALOG EXPLORATION (do this first!):\n\
        DataFusion uses standard information_schema. Use these queries to understand the data:\n\
        - SELECT table_catalog, table_schema, table_name FROM information_schema.tables;\n\
        - SELECT column_name, data_type FROM information_schema.columns WHERE table_name = 'x';\n\
        - SHOW TABLES;\n\n\
        IMPORTANT: Always explore the catalog first to understand what data is available!\n\n";

    format!(
        "DataFusion MCP Server for ACP/CLAUDE SQL generation.\n\n\
        You are being invoked from a DataFusion-based SQL environment.\n\
        Your job is to generate a SQL query that answers the user's question.\n\n\
        TOOLS:\n\
        1) `run_sql` - Execute SQL to explore the catalog/schema and test queries\n\
        2) `final_query` - YOU MUST CALL THIS at the end with your final SQL answer\n\n\
        {restrictions}\
        {catalog}\
        Workflow:\n\
        1. EXPLORE: Use run_sql to query information_schema and understand available tables/columns\n\
        2. INVESTIGATE: Look at sample data with SELECT * FROM table LIMIT 5\n\
        3. BUILD: Construct and test your query\n\
        4. SUBMIT: ALWAYS call final_query with the SQL that answers the question\n\n\
        Tips:\n\
        - DataFusion SQL dialect (supports CTEs, window functions, standard SQL)\n\
        - Keep LIMIT small during exploration\n\
        - Use information_schema for catalog queries (no PRAGMA, no DESCRIBE)",
        restrictions = restrictions,
        catalog = catalog_exploration
    )
}

#[derive(Clone)]
struct DataFusionMcpService {
    executor: Arc<SqlExecutor>,
    safe_mode: bool,
    show_sql: bool,
    final_result_tx: mpsc::Sender<FinalQueryResult>,
}

#[derive(Deserialize)]
struct RunSqlParams {
    sql: String,
}

#[derive(Deserialize)]
struct FinalQueryParams {
    sql: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    datasources: Option<String>,
}

impl ServerHandler for DataFusionMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(get_instructions(self.safe_mode)),
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let safe_mode = self.safe_mode;

        let run_sql_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "The SQL query to execute"
                }
            },
            "required": ["sql"],
            "additionalProperties": false
        });

        let run_sql_desc = if safe_mode {
            "Execute a DataFusion SQL query for exploration. READ-ONLY MODE: Only SELECT queries allowed."
        } else {
            "Execute a DataFusion SQL query. Use this to explore the schema and test queries."
        };

        let run_sql_tool = Tool::new("run_sql", run_sql_desc, rmcp::model::object(run_sql_schema));

        let final_query_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "The final SQL query that answers the user's question"
                },
                "summary": {
                    "type": "string",
                    "description": "A brief summary of the analysis"
                },
                "datasources": {
                    "type": "string",
                    "description": "Description of data sources used"
                }
            },
            "required": ["sql"],
            "additionalProperties": false
        });

        let final_query_desc = if safe_mode {
            "REQUIRED: Call this with the final query. READ-ONLY MODE: Only SELECT queries allowed."
        } else {
            "REQUIRED: Call this with the final SQL that answers the user's question."
        };

        let final_query_tool = Tool::new(
            "final_query",
            final_query_desc,
            rmcp::model::object(final_query_schema),
        );

        std::future::ready(Ok(ListToolsResult {
            tools: vec![run_sql_tool, final_query_tool],
            next_cursor: None,
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        let executor = self.executor.clone();
        let safe_mode = self.safe_mode;
        let show_sql = self.show_sql;
        let final_result_tx = self.final_result_tx.clone();

        async move {
            let args = request
                .arguments
                .ok_or_else(|| McpError::invalid_params("missing arguments", None))?;

            match request.name.as_ref() {
                "run_sql" => {
                    let params: RunSqlParams = serde_json::from_value(Value::Object(args))
                        .map_err(|e| {
                            McpError::invalid_params(format!("bad arguments: {e}"), None)
                        })?;

                    if show_sql {
                        eprintln!("\n[Explore SQL]\n{}", params.sql);
                    }

                    if safe_mode && is_mutation_sql(&params.sql) {
                        return Ok(CallToolResult::success(vec![Content::text(
                            r#"{"error": "Safe mode is enabled. Mutation queries (INSERT, UPDATE, DELETE, DROP, etc.) are not allowed."}"#,
                        )]));
                    }

                    let result = match executor.execute_sql_json(&params.sql).await {
                        Ok(json) => json,
                        Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
                    };

                    Ok(CallToolResult::success(vec![Content::text(result)]))
                }
                "final_query" => {
                    let params: FinalQueryParams = serde_json::from_value(Value::Object(args))
                        .map_err(|e| {
                            McpError::invalid_params(format!("bad arguments: {e}"), None)
                        })?;

                    let final_result = FinalQueryResult {
                        sql: params.sql.clone(),
                        summary: params.summary,
                        datasources: params.datasources,
                    };
                    let _ = final_result_tx.try_send(final_result);

                    let result = serde_json::json!({ "FINAL_SQL": params.sql });
                    Ok(CallToolResult::success(vec![Content::text(
                        result.to_string(),
                    )]))
                }
                _ => Err(McpError::invalid_params(
                    format!("Unknown tool: {}", request.name),
                    None,
                )),
            }
        }
    }
}

pub async fn start_mcp_http_server(
    executor: Arc<SqlExecutor>,
    safe_mode: bool,
    show_sql: bool,
) -> anyhow::Result<(u16, oneshot::Sender<()>, mpsc::Receiver<FinalQueryResult>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let port = addr.port();

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (final_result_tx, final_result_rx) = mpsc::channel::<FinalQueryResult>(1);

    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig {
        sse_keep_alive: Some(std::time::Duration::from_secs(15)),
        stateful_mode: false,
    };

    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(DataFusionMcpService {
                executor: executor.clone(),
                safe_mode,
                show_sql,
                final_result_tx: final_result_tx.clone(),
            })
        },
        session_manager,
        config,
    );

    let app = Router::new().fallback_service(tower::ServiceBuilder::new().service(mcp_service));

    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });

        if let Err(e) = server.await {
            eprintln!("MCP HTTP server error: {}", e);
        }
    });

    Ok((port, shutdown_tx, final_result_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::time::{sleep, timeout, Duration};

    async fn start_server_or_skip(
        exec: Arc<SqlExecutor>,
        safe_mode: bool,
    ) -> Option<(u16, oneshot::Sender<()>, mpsc::Receiver<FinalQueryResult>)> {
        match start_mcp_http_server(exec, safe_mode, false).await {
            Ok(server) => Some(server),
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("operation not permitted") || msg.contains("permission denied") {
                    return None;
                }
                panic!("unexpected error starting MCP server: {e}");
            }
        }
    }

    async fn post_mcp_request(
        port: u16,
        payload: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://127.0.0.1:{port}/"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("HTTP {status}: {body}");
        }

        extract_first_sse_data_json(&body)
    }

    fn extract_first_sse_data_json(sse_body: &str) -> anyhow::Result<serde_json::Value> {
        for line in sse_body.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                return Ok(serde_json::from_str(data)?);
            }
            if let Some(data) = line.strip_prefix("data:") {
                return Ok(serde_json::from_str(data.trim())?);
            }
        }
        anyhow::bail!("No SSE data line in response: {sse_body}");
    }

    fn first_tool_text(response_json: &serde_json::Value) -> Option<&str> {
        response_json
            .get("result")?
            .get("content")?
            .as_array()?
            .first()?
            .get("text")?
            .as_str()
    }

    #[tokio::test]
    async fn test_mcp_server_starts() {
        let exec = Arc::new(SqlExecutor::new().await.unwrap());
        let Some((port, shutdown_tx, _rx)) = start_server_or_skip(exec, true).await else {
            return;
        };
        assert!(port > 0);
        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn test_run_sql_tool_over_http() {
        let exec = Arc::new(SqlExecutor::new().await.unwrap());
        let Some((port, shutdown_tx, _rx)) = start_server_or_skip(exec, false).await else {
            return;
        };
        sleep(Duration::from_millis(50)).await;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "run_sql",
                "arguments": {
                    "sql": "SELECT 1 AS one"
                }
            }
        });

        let response = post_mcp_request(port, payload).await.unwrap();
        let text = first_tool_text(&response).unwrap_or_default();
        assert!(
            text.contains("\"one\":1"),
            "unexpected tool response text: {text}"
        );

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn test_safe_mode_blocks_mutation_over_http() {
        let exec = Arc::new(SqlExecutor::new().await.unwrap());
        let Some((port, shutdown_tx, _rx)) = start_server_or_skip(exec, true).await else {
            return;
        };
        sleep(Duration::from_millis(50)).await;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "run_sql",
                "arguments": {
                    "sql": "DROP TABLE t"
                }
            }
        });

        let response = post_mcp_request(port, payload).await.unwrap();
        let text = first_tool_text(&response).unwrap_or_default();
        assert!(
            text.contains("Safe mode is enabled"),
            "expected safe mode error, got: {text}"
        );

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn test_safe_mode_off_allows_mutation_over_http() {
        let exec = Arc::new(SqlExecutor::new().await.unwrap());
        let Some((port, shutdown_tx, _rx)) = start_server_or_skip(exec, false).await else {
            return;
        };
        sleep(Duration::from_millis(50)).await;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "run_sql",
                "arguments": {
                    "sql": "CREATE VIEW mcp_safe_off_v AS SELECT 1 AS x"
                }
            }
        });

        let response = post_mcp_request(port, payload).await.unwrap();
        let text = first_tool_text(&response).unwrap_or_default();
        assert!(
            !text.contains("Safe mode is enabled"),
            "mutation should not be blocked when safe mode is off: {text}"
        );
        assert!(
            !text.contains("\"error\""),
            "mutation in safe_mode=false should succeed, got: {text}"
        );

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn test_final_query_tool_captures_result_over_http() {
        let exec = Arc::new(SqlExecutor::new().await.unwrap());
        let Some((port, shutdown_tx, mut rx)) = start_server_or_skip(exec, true).await else {
            return;
        };
        sleep(Duration::from_millis(50)).await;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "final_query",
                "arguments": {
                    "sql": "SELECT 1",
                    "summary": "test summary",
                    "datasources": "none"
                }
            }
        });

        let _response = post_mcp_request(port, payload).await.unwrap();
        let result = timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .expect("final_query result should be sent");
        assert_eq!(result.sql, "SELECT 1");
        assert_eq!(result.summary.as_deref(), Some("test summary"));
        assert_eq!(result.datasources.as_deref(), Some("none"));

        let _ = shutdown_tx.send(());
    }
}

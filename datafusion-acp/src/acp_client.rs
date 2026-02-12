use std::env;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use acp::Agent;
use agent_client_protocol as acp;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::mcp_server::{start_mcp_http_server, FinalQueryResult};
use crate::sql_executor::SqlExecutor;

#[derive(Debug, Clone)]
pub struct AcpConfig {
    pub agent_command: Option<String>,
    pub debug: bool,
    pub show_messages: bool,
    pub show_sql: bool,
    pub show_summary: bool,
    pub show_datasources: bool,
    pub timeout_secs: u64,
    pub safe_mode: bool,
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            agent_command: None,
            debug: false,
            show_messages: false,
            show_sql: false,
            show_summary: false,
            show_datasources: false,
            timeout_secs: 300,
            safe_mode: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AcpResult {
    pub sql: String,
    pub summary: Option<String>,
    pub datasources: Option<String>,
}

fn resolve_agent_command(agent: &str) -> Result<(String, Vec<String>)> {
    if agent.starts_with('/') || agent.starts_with("./") || agent.starts_with("../") {
        return Ok((agent.to_string(), vec![]));
    }

    let expanded = match agent {
        "claude-code" | "claude" => "claude-code-acp",
        other => other,
    };

    if which::which(expanded).is_ok() {
        return Ok((expanded.to_string(), vec![]));
    }

    if which::which("bunx").is_ok() {
        return Ok(("bunx".to_string(), vec![expanded.to_string()]));
    }

    if which::which("npx").is_ok() {
        return Ok(("npx".to_string(), vec![expanded.to_string()]));
    }

    Err(anyhow!(
        "Neither '{}' nor bunx/npx were found in PATH",
        expanded
    ))
}

fn read_channel_result(rx: &mut mpsc::Receiver<FinalQueryResult>) -> Option<FinalQueryResult> {
    rx.try_recv().ok()
}

pub async fn run_acp(
    query: &str,
    executor: Arc<SqlExecutor>,
    config: &AcpConfig,
) -> Result<AcpResult> {
    let default_agent = env::var("ACP_AGENT").unwrap_or_else(|_| "claude-code".to_string());
    let agent_setting = config.agent_command.clone().unwrap_or(default_agent);

    let (agent_cmd, agent_args) = resolve_agent_command(&agent_setting)?;

    let (port, shutdown_tx, mut final_result_rx) =
        start_mcp_http_server(executor, config.safe_mode, config.show_sql).await?;

    let acp_future = run_acp_flow(
        &agent_cmd,
        &agent_args,
        query,
        port,
        config.debug,
        config.show_messages,
        config.safe_mode,
    );

    let sql = tokio::time::timeout(
        std::time::Duration::from_secs(config.timeout_secs),
        acp_future,
    )
    .await
    .context("ACP flow timed out")??;

    let final_result = read_channel_result(&mut final_result_rx);
    let _ = shutdown_tx.send(());

    if config.show_summary {
        if let Some(fr) = &final_result {
            if let Some(summary) = &fr.summary {
                eprintln!("\n[Summary]\n{}", summary);
                let _ = std::io::stderr().flush();
            }
        }
    }

    if config.show_datasources {
        if let Some(fr) = &final_result {
            if let Some(datasources) = &fr.datasources {
                eprintln!("\n[Datasources]\n{}", datasources);
                let _ = std::io::stderr().flush();
            }
        }
    }

    if config.show_sql {
        eprintln!("\n[Final SQL]\n{}", sql);
        let _ = std::io::stderr().flush();
    }

    let sql = final_result.as_ref().map(|r| r.sql.clone()).unwrap_or(sql);

    Ok(AcpResult {
        sql,
        summary: final_result.as_ref().and_then(|r| r.summary.clone()),
        datasources: final_result.and_then(|r| r.datasources),
    })
}

async fn run_acp_flow(
    agent_cmd: &str,
    agent_args: &[String],
    query: &str,
    mcp_port: u16,
    debug: bool,
    show_messages: bool,
    safe_mode: bool,
) -> Result<String> {
    let final_sql = Arc::new(Mutex::new(None::<String>));
    let cancelled = Arc::new(AtomicBool::new(false));
    let message_buffer = Arc::new(Mutex::new(String::new()));

    let client = AcpClient {
        final_sql: final_sql.clone(),
        cancelled,
        message_buffer,
        debug,
        show_messages,
    };

    let mut child = Command::new(agent_cmd)
        .args(agent_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("Failed to spawn agent '{}'", agent_cmd))?;

    let outgoing = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Failed to get agent stdin"))?
        .compat_write();
    let incoming = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to get agent stdout"))?
        .compat();

    if debug {
        if let Some(stderr) = child.stderr.take() {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    eprintln!("acp agent stderr: {line}");
                }
            });
        }
    }

    let local = tokio::task::LocalSet::new();
    let query_owned = query.to_string();

    let prompt_result = local
        .run_until(async move {
            let (conn, handle_io) = acp::ClientSideConnection::new(
                client,
                outgoing,
                incoming,
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );

            tokio::task::spawn_local(handle_io);

            conn.initialize(acp::InitializeRequest::new(acp::ProtocolVersion::LATEST))
                .await
                .context("ACP initialize failed")?;

            let mcp_url = format!("http://127.0.0.1:{}/mcp", mcp_port);
            let mcp_server = acp::McpServerHttp::new("datafusion", &mcp_url);
            let mcp_servers = vec![acp::McpServer::Http(mcp_server)];

            let cwd = env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            let mode_rules = if safe_mode {
                "- Generate ONLY read-only SELECT queries. Never INSERT, UPDATE, DELETE, DROP, CREATE, etc.\n"
            } else {
                "- You may use any SQL statements including INSERT, UPDATE, DELETE, CREATE, etc.\n"
            };

            let system_prompt = format!(
                "You are operating as a SQL generation assistant within DataFusion. {}Always call final_query with your answer.",
                mode_rules
            );

            let session_meta: serde_json::Map<String, serde_json::Value> =
                serde_json::from_value(serde_json::json!({
                    "disableBuiltInTools": true,
                    "systemPrompt": {
                        "append": system_prompt
                    }
                }))
                .context("failed to build ACP session metadata")?;

            let new_sess = conn
                .new_session(
                    acp::NewSessionRequest::new(cwd)
                        .mcp_servers(mcp_servers)
                        .meta(session_meta),
                )
                .await
                .context("ACP new_session failed")?;

            conn.prompt(acp::PromptRequest::new(
                new_sess.session_id,
                vec![query_owned.into()],
            ))
            .await
            .context("ACP prompt failed")?;

            Ok::<(), anyhow::Error>(())
        })
        .await;

    child.kill().await.ok();
    prompt_result?;

    let maybe_sql = final_sql
        .lock()
        .map_err(|_| anyhow!("mutex poisoned"))?
        .clone()
        .ok_or_else(|| anyhow!("Agent did not call final_query"));
    maybe_sql
}

struct AcpClient {
    final_sql: Arc<Mutex<Option<String>>>,
    cancelled: Arc<AtomicBool>,
    message_buffer: Arc<Mutex<String>>,
    debug: bool,
    show_messages: bool,
}

#[async_trait(?Send)]
impl acp::Client for AcpClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> std::result::Result<acp::RequestPermissionResponse, acp::Error> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            ));
        }

        use acp::PermissionOptionKind as K;
        let choice = args
            .options
            .iter()
            .find(|o| matches!(o.kind, K::AllowOnce))
            .cloned()
            .or_else(|| args.options.first().cloned());

        match choice {
            Some(o) => Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                    o.option_id,
                )),
            )),
            None => Err(acp::Error::invalid_params()),
        }
    }

    async fn write_text_file(
        &self,
        _args: acp::WriteTextFileRequest,
    ) -> std::result::Result<acp::WriteTextFileResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn read_text_file(
        &self,
        _args: acp::ReadTextFileRequest,
    ) -> std::result::Result<acp::ReadTextFileResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn create_terminal(
        &self,
        _args: acp::CreateTerminalRequest,
    ) -> std::result::Result<acp::CreateTerminalResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn terminal_output(
        &self,
        _args: acp::TerminalOutputRequest,
    ) -> std::result::Result<acp::TerminalOutputResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn release_terminal(
        &self,
        _args: acp::ReleaseTerminalRequest,
    ) -> std::result::Result<acp::ReleaseTerminalResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn wait_for_terminal_exit(
        &self,
        _args: acp::WaitForTerminalExitRequest,
    ) -> anyhow::Result<acp::WaitForTerminalExitResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn kill_terminal_command(
        &self,
        _args: acp::KillTerminalCommandRequest,
    ) -> anyhow::Result<acp::KillTerminalCommandResponse, acp::Error> {
        Err(acp::Error::method_not_found())
    }

    async fn session_notification(
        &self,
        args: acp::SessionNotification,
    ) -> anyhow::Result<(), acp::Error> {
        use acp::SessionUpdate as SU;

        if self.cancelled.load(Ordering::SeqCst) {
            return Ok(());
        }

        match args.update {
            SU::AgentMessageChunk(chunk) => {
                let text = match chunk.content {
                    acp::ContentBlock::Text(t) => t.text,
                    acp::ContentBlock::Image(_) => "<image>".into(),
                    acp::ContentBlock::Audio(_) => "<audio>".into(),
                    acp::ContentBlock::ResourceLink(link) => link.uri,
                    acp::ContentBlock::Resource(_) => "<resource>".into(),
                    _ => "<unknown>".into(),
                };

                if self.debug {
                    eprintln!("acp: agent message chunk: {text}");
                }

                if self.show_messages && !text.trim().is_empty() {
                    let mut buffer = self
                        .message_buffer
                        .lock()
                        .map_err(|_| acp::Error::internal_error())?;
                    let was_empty = buffer.is_empty();
                    buffer.push_str(&text);
                    if was_empty {
                        eprint!("\n[Agent] ");
                    }
                    eprint!("{text}");
                    let _ = std::io::stderr().flush();
                }
            }
            SU::ToolCall(tc) => {
                if self.debug {
                    eprintln!(
                        "acp: tool_call id={} title='{}' status={:?}",
                        tc.tool_call_id.0, tc.title, tc.status
                    );
                }
                if tc.title.contains("final_query") {
                    if let Some(raw_input) = tc.raw_input {
                        if let Some(sql) = raw_input.get("sql").and_then(|v| v.as_str()) {
                            *self
                                .final_sql
                                .lock()
                                .map_err(|_| acp::Error::internal_error())? = Some(sql.to_string());
                        }
                    }
                }
            }
            SU::ToolCallUpdate(_) => {}
            _ => {}
        }

        Ok(())
    }
}

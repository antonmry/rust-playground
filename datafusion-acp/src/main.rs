mod acp_client;
mod claude_statement;
mod mcp_server;
mod sql_executor;

use std::collections::HashSet;
use std::sync::Arc;

use acp_client::{run_acp, AcpConfig};
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use claude_statement::{register_claude_table_function, ClaudeParser, ClaudeStatement};
use datafusion::arrow::csv;
use datafusion::arrow::util::pretty::pretty_format_batches;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use sql_executor::{parse_file_spec, SqlExecutor};

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Csv,
}

#[derive(Debug, Parser)]
#[command(
    name = "datafusion-acp",
    version,
    about = "ACP-backed SQL assistant using DataFusion"
)]
struct Cli {
    #[arg(help = "Natural language query (omit for REPL mode)")]
    query: Option<String>,

    #[arg(short, long = "file", value_name = "PATH", action = clap::ArgAction::Append)]
    file: Vec<String>,

    #[arg(short = 'o', long = "format", value_enum, default_value_t = OutputFormat::Table)]
    format: OutputFormat,

    #[arg(short = 'a', long = "agent", env = "ACP_AGENT")]
    agent: Option<String>,

    #[arg(
        short = 't',
        long = "timeout",
        env = "ACP_TIMEOUT",
        default_value_t = 300
    )]
    timeout: u64,

    #[arg(long = "safe-mode", env = "ACP_SAFE_MODE", default_value_t = true)]
    safe_mode: bool,

    #[arg(long = "no-safe-mode", default_value_t = false)]
    no_safe_mode: bool,

    #[arg(long)]
    debug: bool,

    #[arg(long = "show-messages")]
    show_messages: bool,

    #[arg(long = "show-sql")]
    show_sql: bool,

    #[arg(long = "show-summary")]
    show_summary: bool,

    #[arg(long = "show-datasources")]
    show_datasources: bool,
}

fn build_acp_config(cli: &Cli) -> AcpConfig {
    AcpConfig {
        agent_command: cli.agent.clone(),
        debug: cli.debug,
        show_messages: cli.show_messages,
        show_sql: cli.show_sql,
        show_summary: cli.show_summary,
        show_datasources: cli.show_datasources,
        timeout_secs: cli.timeout,
        safe_mode: if cli.no_safe_mode {
            false
        } else {
            cli.safe_mode
        },
    }
}

async fn register_files(executor: &SqlExecutor, specs: &[String]) -> Result<()> {
    let mut names = HashSet::new();
    for spec in specs {
        let (name, path) = parse_file_spec(spec)?;
        if !std::path::Path::new(&path).exists() {
            anyhow::bail!("File not found: '{}'", path);
        }
        if !names.insert(name.clone()) {
            anyhow::bail!(
                "Duplicate table name '{}'. Use explicit table_name=path to disambiguate.",
                name
            );
        }
        executor
            .register_file(&name, &path)
            .await
            .with_context(|| format!("Failed to load '{}={}'", name, path))?;
    }
    Ok(())
}

async fn print_dataframe(executor: &SqlExecutor, sql: &str, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let json = executor.execute_sql_json(sql).await?;
            println!("{json}");
        }
        OutputFormat::Table => {
            let df = executor.execute_sql(sql).await?;
            let batches = df.collect().await?;
            println!("{}", pretty_format_batches(&batches)?);
        }
        OutputFormat::Csv => {
            let df = executor.execute_sql(sql).await?;
            let batches = df.collect().await?;
            let mut out = Vec::new();
            {
                let mut writer = csv::Writer::new(&mut out);
                for batch in &batches {
                    writer.write(batch)?;
                }
            }
            print!("{}", String::from_utf8_lossy(&out));
        }
    }
    Ok(())
}

async fn run_one_shot(cli: &Cli) -> Result<()> {
    let query = cli
        .query
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("query is required in one-shot mode"))?;

    let executor = Arc::new(SqlExecutor::new().await?);
    register_files(&executor, &cli.file).await?;

    let config = build_acp_config(cli);
    let result = run_acp(query, executor.clone(), &config).await?;
    let _ = (&result.summary, &result.datasources);
    print_dataframe(&executor, &result.sql, &cli.format).await
}

async fn run_repl(cli: &Cli) -> Result<()> {
    let executor = Arc::new(SqlExecutor::new().await?);
    register_files(&executor, &cli.file).await?;

    let config = Arc::new(build_acp_config(cli));
    register_claude_table_function(&executor.ctx, executor.clone(), config.clone())?;

    let mut rl = DefaultEditor::new()?;

    loop {
        let line = match rl.readline("datafusion-acp> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed.to_ascii_lowercase().as_str(), "exit" | "quit") {
            break;
        }

        rl.add_history_entry(trimmed).ok();

        let mut parser = ClaudeParser::new(trimmed)?;
        match parser.parse_statement()? {
            ClaudeStatement::Claude(nl) => {
                let result = run_acp(&nl, executor.clone(), &config).await?;
                print_dataframe(&executor, &result.sql, &cli.format).await?;
            }
            ClaudeStatement::DFStatement(stmt) => {
                drop(stmt);
                print_dataframe(&executor, trimmed, &cli.format).await?;
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .try_init()
        .ok();

    if cli.query.is_some() {
        run_one_shot(&cli).await
    } else {
        run_repl(&cli).await
    }
}

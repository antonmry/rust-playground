# datafusion-acp

A pure Rust CLI that connects natural language queries to
[Apache DataFusion](https://datafusion.apache.org/) via the
[Agent Client Protocol (ACP)](https://github.com/anthropics/agent-client-protocol).
Inspired by [duckdb-acp](./duckdb-acp/), but with no C++, no FFI —
everything in Rust.

## Building

```bash
cargo build --release
```

The binary is at `target/release/datafusion-acp`.

## Quick Start

```bash
# One-shot: ask a natural language question over a CSV
datafusion-acp --file products.csv "what are the cheapest products?"

# Interactive REPL
datafusion-acp --file products.csv
```

## Usage

```
datafusion-acp [OPTIONS] [QUERY]

Arguments:
  [QUERY]  Natural language query (omit for REPL mode)

Options:
  -f, --file <PATH>         Load data file (repeatable). Format: [table_name=]path
  -o, --format <FMT>        Output format: table (default), json, csv
  -a, --agent <CMD>         Agent command [env: ACP_AGENT] [default: claude-code]
  -t, --timeout <SECS>      Agent timeout in seconds [env: ACP_TIMEOUT] [default: 300]
      --safe-mode            Block mutation queries (default: true) [env: ACP_SAFE_MODE]
      --no-safe-mode         Allow mutation queries
      --debug                Enable debug output
      --show-messages        Stream agent thinking
      --show-sql             Print generated SQL before executing
      --show-summary         Print analysis summary
      --show-datasources     Print data sources used
  -h, --help                Print help
  -V, --version             Print version
```

## Authentication

`datafusion-acp` itself does not handle authentication. It spawns an
**agent process** (default: `claude-code`) which manages its own auth.

For the default `claude-code` agent, set your Anthropic API key:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

You can get your key at: https://console.anthropic.com/settings/keys

If you use a different agent (via `--agent` or `ACP_AGENT`), that agent
will have its own authentication mechanism.

Use `--debug` to troubleshoot auth issues — it prints the resolved agent
command and process output:

```bash
datafusion-acp --debug --file data.csv "show me the data"
```

## Loading Data

Files are registered as DataFusion tables. The format is auto-detected by
extension:

| Extension              | Format      |
|------------------------|-------------|
| `.csv`                 | CSV         |
| `.parquet`, `.pq`      | Parquet     |
| `.json`, `.ndjson`     | JSON/NDJSON |

The default table name is the file stem (e.g. `products.csv` becomes table
`products`). Use `table_name=path` to override:

```bash
# Table "products" (default from file stem)
datafusion-acp --file products.csv

# Table "items" (explicit name)
datafusion-acp --file items=products.csv

# Multiple files
datafusion-acp --file products.csv --file sales.parquet
```

Duplicate table names produce an error — use explicit names to disambiguate.

## Examples

### One-Shot Query

```bash
datafusion-acp --file tests/data/products.csv "list all products sorted by price"
```

### REPL Mode

```bash
$ datafusion-acp --file tests/data/products.csv
datafusion-acp> SELECT * FROM products ORDER BY price;
datafusion-acp> CLAUDE what are the most expensive products?
datafusion-acp> SELECT * FROM claude('show me hardware products');
datafusion-acp> exit
```

In the REPL you can:
- Run standard SQL directly
- Use `CLAUDE <question>` to ask the agent in natural language
- Use `SELECT * FROM claude('<question>')` as a table function
- Type `exit`, `quit`, or press Ctrl-D to leave

### Output Formats

```bash
# Pretty table (default)
echo "SELECT * FROM products;" | datafusion-acp --file tests/data/products.csv

# JSON
echo "SELECT * FROM products;" | datafusion-acp --file tests/data/products.csv --format json

# CSV
echo "SELECT * FROM products;" | datafusion-acp --file tests/data/products.csv --format csv
```

### Safe Mode

Safe mode (enabled by default) blocks mutation queries like `INSERT`,
`UPDATE`, `DELETE`, `DROP`, etc.:

```bash
# Mutations blocked (default)
datafusion-acp --file data.csv "delete all records"

# Mutations allowed
datafusion-acp --file data.csv --no-safe-mode "create a summary table"
```

### Environment Variables

CLI flags can be set via environment variables:

```bash
export ACP_AGENT=my-custom-agent
export ACP_TIMEOUT=60
export ACP_SAFE_MODE=true

datafusion-acp --file data.csv "summarise the data"
```

## Testing

### Unit and Integration Tests

```bash
# Run all tests (no agent required)
cargo test

# Check formatting and linting
cargo fmt -- --check
cargo clippy -- -D warnings
```

### CLI Smoke Tests

These test file loading and SQL execution without an agent:

```bash
# Load a CSV and query it
echo "SELECT * FROM products ORDER BY id;" | cargo run -- --file tests/data/products.csv

# Load with explicit table name
echo "SELECT * FROM items ORDER BY id;" | cargo run -- --file items=tests/data/products.csv

# Load Parquet
echo "SELECT COUNT(*) FROM sales;" | cargo run -- --file tests/data/sales.parquet

# Load JSON
echo "SELECT COUNT(*) FROM events;" | cargo run -- --file tests/data/events.json

# JSON output
echo "SELECT * FROM products;" | cargo run -- --file tests/data/products.csv --format json

# CSV output
echo "SELECT * FROM products;" | cargo run -- --file tests/data/products.csv --format csv

# REPL exits on EOF
echo "" | cargo run -- --file tests/data/products.csv

# REPL executes SQL
echo "SELECT 1 + 1 AS result;" | cargo run --
```

### Error Cases

```bash
# Nonexistent file — exits with error
cargo run -- --file /nonexistent/path.csv

# Unknown extension — exits with error
cargo run -- --file tests/data/unknown.xyz

# Duplicate table names — exits with error
cargo run -- --file tests/data/products.csv --file tests/data/products.csv
```

### End-to-End Tests (Requires Agent)

These require `ANTHROPIC_API_KEY` and a running agent:

```bash
# Run gated integration tests
ACP_INTEGRATION_TEST=1 ANTHROPIC_API_KEY=sk-... cargo test -- --ignored --nocapture

# One-shot with agent
cargo run -- --file tests/data/products.csv "list all products sorted by price"

# REPL with agent
echo "CLAUDE list all product names" | cargo run -- --file tests/data/products.csv

# Debug output
cargo run -- --file tests/data/products.csv --debug --show-sql --show-summary "list all products"
```

## Architecture

```
CLI (main.rs)
├── acp_client.rs     — ACP agent flow (spawn, protocol, session)
├── sql_executor.rs   — DataFusion SessionContext wrapper
├── mcp_server.rs     — Embedded MCP HTTP server (Axum + rmcp)
└── claude_statement.rs — CLAUDE parser + claude() table function
```

The agent communicates with an embedded MCP HTTP server that exposes two
tools: `run_sql` (execute SQL) and `final_query` (capture the agent's
chosen SQL). The CLI orchestrates the flow and displays results.

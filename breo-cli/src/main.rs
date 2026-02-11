use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::engine::{
    ArgValueCandidates, ArgValueCompleter, CompletionCandidate, PathCompleter,
};
use clap_complete::env::CompleteEnv;
use serde::Deserialize;
use skim::prelude::*;
use std::fs;
use std::io::{self, Cursor, IsTerminal, Read as _};
use std::path::PathBuf;
use std::process::Command;

#[derive(Deserialize)]
#[serde(default)]
struct Config {
    sandbox: bool,
    sandbox_name: String,
    push: bool,
    agent: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sandbox: true,
            sandbox_name: "default".into(),
            push: true,
            agent: "claude".into(),
        }
    }
}

fn load_config() -> Config {
    let path = breo_dir().join("config.toml");
    match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

fn list_models() -> Vec<CompletionCandidate> {
    vec![
        // Claude
        CompletionCandidate::new("opus").help(Some("Claude Opus 4.6 (200k)".into())),
        CompletionCandidate::new("sonnet").help(Some("Claude Sonnet 4.5 (200k)".into())),
        CompletionCandidate::new("haiku").help(Some("Claude Haiku 4.5 (200k)".into())),
        // OpenAI
        CompletionCandidate::new("gpt-5").help(Some("GPT-5 (400k)".into())),
        CompletionCandidate::new("gpt-5-mini").help(Some("GPT-5 mini (400k)".into())),
        CompletionCandidate::new("o3").help(Some("o3 (200k)".into())),
        CompletionCandidate::new("o4-mini").help(Some("o4-mini (200k)".into())),
        // Gemini
        CompletionCandidate::new("gemini-2.5-pro").help(Some("Gemini 2.5 Pro (1M)".into())),
        CompletionCandidate::new("gemini-2.5-flash").help(Some("Gemini 2.5 Flash (1M)".into())),
    ]
}

fn list_conversations() -> Vec<CompletionCandidate> {
    let dir = conversations_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return vec![];
    };
    entries
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().to_string();
            let name = name.strip_suffix(".md")?;
            Some(CompletionCandidate::new(name.to_string()))
        })
        .collect()
}

#[derive(Clone, ValueEnum)]
enum Backend {
    Claude,
    Codex,
    Gemini,
}

#[derive(Parser)]
#[command(
    name = "breo",
    about = "Chat with an LLM, keeping conversation in a markdown file"
)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    /// The message to send
    message: Option<String>,

    /// Send to a specific conversation without switching the active one
    #[arg(short, long, add = ArgValueCandidates::new(list_conversations))]
    conversation: Option<String>,

    /// Model to use (e.g. sonnet, opus, o3, gpt-5, or a full model ID)
    #[arg(short, long, add = ArgValueCandidates::new(list_models))]
    model: Option<String>,

    /// Agent to use
    #[arg(short, long, value_enum)]
    agent: Option<Backend>,

    /// Files to attach to the prompt
    #[arg(short, long, num_args = 1.., add = ArgValueCompleter::new(PathCompleter::file()))]
    files: Vec<PathBuf>,

    /// Lima instance name for sandbox
    #[arg(short, long)]
    sandbox: Option<String>,

    /// Disable sandbox mode
    #[arg(long)]
    no_sandbox: bool,

    /// Disable auto-push after commit
    #[arg(long)]
    no_push: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new conversation and switch to it
    New { name: String },
    /// Switch to an existing conversation
    Switch {
        #[arg(add = ArgValueCandidates::new(list_conversations))]
        name: String,
    },
    /// List all conversations
    List,
    /// Fuzzy-pick a conversation (for shell integration)
    Pick,
    /// Print shell setup for fuzzy TAB completion
    Setup {
        /// Shell type
        #[arg(value_enum)]
        shell: ShellType,
    },
    /// Compact a conversation by summarizing it to save context
    Compact {
        /// Conversation to compact (defaults to active)
        #[arg(add = ArgValueCandidates::new(list_conversations))]
        name: Option<String>,
    },
}

#[derive(Clone, ValueEnum)]
enum ShellType {
    Bash,
    Zsh,
    Fish,
}

fn breo_dir() -> PathBuf {
    dirs::config_dir()
        .expect("could not determine config directory")
        .join("breo")
}

fn conversations_dir() -> PathBuf {
    breo_dir().join("conversations")
}

fn ensure_breo_dir() {
    let base = breo_dir();
    let conv_dir = conversations_dir();
    if !conv_dir.exists()
        && let Err(e) = fs::create_dir_all(&conv_dir)
    {
        eprintln!("Failed to create {}: {e}", conv_dir.display());
        std::process::exit(1);
    }

    // git init if .git doesn't exist
    if !base.join(".git").exists() {
        let _ = Command::new("git")
            .arg("init")
            .current_dir(&base)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    // Create default config.toml if missing
    let config_path = base.join("config.toml");
    if !config_path.exists() {
        let default_config =
            "sandbox = true\nsandbox_name = \"default\"\npush = true\nagent = \"claude\"\n";
        let _ = fs::write(&config_path, default_config);
    }
}

fn active_file_path() -> PathBuf {
    breo_dir().join("active")
}

fn get_active() -> String {
    let path = active_file_path();
    if path.exists() {
        fs::read_to_string(&path)
            .unwrap_or_else(|_| "default".into())
            .trim()
            .to_string()
    } else {
        "default".into()
    }
}

fn set_active(name: &str) {
    let path = active_file_path();
    if let Err(e) = fs::write(&path, name) {
        eprintln!("Failed to write {}: {e}", path.display());
        std::process::exit(1);
    }
}

fn conversation_path(name: &str) -> PathBuf {
    conversations_dir().join(format!("{name}.md"))
}

fn context_window(model: Option<&str>, backend: &Backend) -> usize {
    if let Some(m) = model {
        let m = m.to_lowercase();
        // Claude models
        if m.contains("opus") || m.contains("sonnet") || m.contains("haiku") {
            return 200_000;
        }
        // OpenAI models
        if m.contains("gpt-5") {
            return 400_000;
        }
        if m.contains("o3") || m.contains("o4-mini") {
            return 200_000;
        }
        // Gemini models
        if m.contains("gemini") {
            return 1_000_000;
        }
    }
    // Default per backend
    match backend {
        Backend::Claude => 200_000,   // claude-opus-4-6
        Backend::Codex => 400_000,    // gpt-5
        Backend::Gemini => 1_000_000, // gemini-2.5-pro
    }
}

fn estimate_tokens(text: &str) -> usize {
    // ~4 chars per token is a reasonable approximation for English text
    text.len() / 4
}

fn count_exchanges(text: &str) -> usize {
    text.matches("## User").count()
}

fn format_tokens(tokens: usize) -> String {
    if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn is_committed(path: &std::path::Path) -> bool {
    Command::new("git")
        .arg("diff")
        .arg("--quiet")
        .arg("HEAD")
        .arg("--")
        .arg(path)
        .current_dir(breo_dir())
        .status()
        .is_ok_and(|s| s.success())
}

fn print_context_summary(
    content: &str,
    name: &str,
    model: Option<&str>,
    backend: &Backend,
    path: &std::path::Path,
) {
    let window = context_window(model, backend);
    let exchanges = count_exchanges(content);
    let tokens_used = estimate_tokens(content);
    let tokens_remaining = window.saturating_sub(tokens_used);
    let pct_used = (tokens_used as f64 / window as f64 * 100.0) as usize;

    let dirty = if is_committed(path) {
        ""
    } else {
        " | uncommitted"
    };

    eprintln!(
        "\n[{name}] {exchanges} exchanges | ~{} tokens used | ~{} remaining ({pct_used}% used){dirty}",
        format_tokens(tokens_used),
        format_tokens(tokens_remaining),
    );
}

fn cmd_new(name: &str) {
    ensure_breo_dir();
    let path = conversation_path(name);
    if path.exists() {
        eprintln!("Conversation '{name}' already exists");
        std::process::exit(1);
    }
    let header = format!("# Conversation: {name}\n\n");
    if let Err(e) = fs::write(&path, &header) {
        eprintln!("Failed to create {}: {e}", path.display());
        std::process::exit(1);
    }
    set_active(name);
    println!("Created and switched to conversation: {name}");
}

fn cmd_pick() {
    let dir = conversations_dir();
    if !dir.exists() {
        std::process::exit(1);
    }
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|_| std::process::exit(1))
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().to_string();
            name.strip_suffix(".md").map(String::from)
        })
        .collect();
    names.sort();

    if names.is_empty() {
        std::process::exit(1);
    }

    let active = get_active();
    let input = names
        .iter()
        .map(|n| {
            if *n == active {
                format!("* {n}")
            } else {
                format!("  {n}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let options = SkimOptionsBuilder::default()
        .prompt("conversation> ".to_string())
        .build()
        .unwrap();

    let item_reader = SkimItemReader::default();
    let items = item_reader.of_bufread(Cursor::new(input));

    let Ok(output) = Skim::run_with(options, Some(items)) else {
        std::process::exit(1);
    };
    if output.is_abort {
        std::process::exit(1);
    }

    if let Some(item) = output.selected_items.first() {
        let name = item
            .output()
            .trim()
            .trim_start_matches("* ")
            .trim_start()
            .to_string();
        print!("{name}");
    }
}

fn cmd_switch(name: &str) {
    let path = conversation_path(name);
    if !path.exists() {
        eprintln!("Conversation '{name}' does not exist");
        std::process::exit(1);
    }
    set_active(name);
    println!("Switched to conversation: {name}");
}

fn cmd_list() {
    let dir = conversations_dir();
    if !dir.exists() {
        println!("No conversations yet.");
        return;
    }
    let active = get_active();
    let mut entries: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to read {}: {e}", dir.display());
            std::process::exit(1);
        })
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            name.strip_suffix(".md").map(String::from)
        })
        .collect();
    entries.sort();

    if entries.is_empty() {
        println!("No conversations yet.");
        return;
    }

    for name in &entries {
        if *name == active {
            println!("* {name}");
        } else {
            println!("  {name}");
        }
    }
}

fn cmd_setup(shell: &ShellType) {
    let script = match shell {
        ShellType::Bash => {
            r#"# breo bash completion with fuzzy pick
# 1. Source clap completions (defines _clap_complete_breo)
source <(COMPLETE=bash breo)

# 2. Override with our skim-powered wrapper
_breo_with_pick() {
    local prev="${COMP_WORDS[COMP_CWORD-1]}"

    if [[ "$prev" == "-c" || "$prev" == "--conversation" ]] || \
       [[ ("${COMP_WORDS[1]}" == "switch" || "${COMP_WORDS[1]}" == "compact") && $COMP_CWORD -eq 2 ]]; then
        local pick
        pick=$(breo pick </dev/tty 2>/dev/tty)
        if [[ -n "$pick" ]]; then
            COMPREPLY=("${pick} ")
        fi
        return
    fi

    _clap_complete_breo "$@"
}
complete -o nospace -o bashdefault -o nosort -F _breo_with_pick breo"#
        }
        ShellType::Zsh => {
            r#"# breo zsh completion with fuzzy pick
# 1. Source clap completions (defines _clap_dynamic_completer_breo)
source <(COMPLETE=zsh breo)

# 2. Override with our skim-powered wrapper
_breo_with_pick() {
    if [[ "${words[-2]}" == "-c" || "${words[-2]}" == "--conversation" ]] || \
       [[ ("${words[2]}" == "switch" || "${words[2]}" == "compact") && $CURRENT -eq 3 ]]; then
        local pick
        pick=$(breo pick </dev/tty 2>/dev/tty)
        if [[ -n "$pick" ]]; then
            compadd -S ' ' -- "$pick"
        fi
        return
    fi
    _clap_dynamic_completer_breo "$@"
}
compdef _breo_with_pick breo"#
        }
        ShellType::Fish => {
            r#"# breo fish completion with fuzzy pick
source (COMPLETE=fish breo | psub)

function __breo_pick_conversation
    breo pick </dev/tty 2>/dev/tty
end

complete -c breo -l conversation -s c -x -a '(__breo_pick_conversation)'
complete -c breo -n '__fish_seen_subcommand_from switch' -x -a '(__breo_pick_conversation)'
complete -c breo -n '__fish_seen_subcommand_from compact' -x -a '(__breo_pick_conversation)'"#
        }
    };
    println!("{script}");
}

fn build_command(backend: &Backend, prompt: &str, model: Option<&str>) -> Command {
    match backend {
        Backend::Claude => {
            let mut cmd = Command::new("claude");
            cmd.arg("--dangerously-skip-permissions");
            cmd.arg("--print").arg(prompt);
            if let Some(model) = model {
                cmd.arg("--model").arg(model);
            }
            cmd
        }
        Backend::Codex => {
            let mut cmd = Command::new("codex");
            cmd.arg("--full-auto");
            cmd.arg("exec").arg(prompt);
            if let Some(model) = model {
                cmd.arg("--model").arg(model);
            }
            cmd
        }
        Backend::Gemini => {
            let mut cmd = Command::new("gemini");
            cmd.arg("--yolo");
            cmd.arg("--prompt").arg(prompt);
            if let Some(model) = model {
                cmd.arg("--model").arg(model);
            }
            cmd
        }
    }
}

fn build_sandbox_command(
    sandbox_name: &str,
    backend: &Backend,
    prompt: &str,
    model: Option<&str>,
) -> Command {
    let mut cmd = Command::new("limactl");
    cmd.arg("shell").arg(sandbox_name);

    match backend {
        Backend::Claude => {
            cmd.arg("claude")
                .arg("--dangerously-skip-permissions")
                .arg("--print")
                .arg(prompt);
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
        }
        Backend::Codex => {
            cmd.arg("codex").arg("--full-auto").arg("exec").arg(prompt);
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
        }
        Backend::Gemini => {
            cmd.arg("gemini").arg("--yolo").arg("--prompt").arg(prompt);
            if let Some(m) = model {
                cmd.arg("--model").arg(m);
            }
        }
    }
    cmd
}

fn execute_command(cmd: Command, backend: &Backend) -> (String, String, bool) {
    let bin = backend_name(backend);
    let mut cmd = cmd;
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Failed to run {bin}: {e}");
            std::process::exit(1);
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

fn backend_name(backend: &Backend) -> &'static str {
    match backend {
        Backend::Claude => "claude",
        Backend::Codex => "codex",
        Backend::Gemini => "gemini",
    }
}

fn read_attached_files(files: &[PathBuf]) -> String {
    let mut attachments = String::new();
    for path in files {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to read {}: {e}", path.display());
                std::process::exit(1);
            }
        };
        attachments.push_str(&format!(
            "\n### File: {}\n```\n{content}\n```\n",
            path.display()
        ));
    }
    attachments
}

fn git_commit_conversation(path: &std::path::Path, message: &str, push: bool) {
    let base = breo_dir();
    let status = Command::new("git")
        .arg("add")
        .arg(path)
        .current_dir(&base)
        .status();
    if let Ok(s) = status
        && s.success()
    {
        let committed = Command::new("git")
            .arg("commit")
            .arg("-m")
            .arg(message)
            .current_dir(&base)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());

        if push && committed {
            let _ = Command::new("git")
                .arg("push")
                .current_dir(&base)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
}

fn cmd_compact(name: Option<&str>, sandbox: Option<&str>, push: bool) {
    let active = get_active();
    let name = name.unwrap_or(&active);
    let path = conversation_path(name);

    if !path.exists() {
        eprintln!("Conversation '{name}' does not exist");
        std::process::exit(1);
    }

    let content = fs::read_to_string(&path).unwrap_or_default();
    let tokens_before = estimate_tokens(&content);
    let exchanges_before = count_exchanges(&content);

    if exchanges_before == 0 {
        eprintln!("Nothing to compact in '{name}'");
        return;
    }

    let prompt = format!(
        "You are compacting a conversation for future LLM context. \
         Summarize the following conversation into a concise briefing that an LLM can use \
         to seamlessly resume the conversation. Preserve:\n\
         - The original intent and goals\n\
         - Key decisions made and their rationale\n\
         - Important code snippets, file paths, commands, and technical details\n\
         - Errors encountered and their solutions\n\
         - Current state and what was being worked on last\n\n\
         Give significantly more weight to recent exchanges as they represent the current working state.\n\
         Output ONLY the summary as markdown, starting with '# Conversation: {name} (compacted)'.\n\
         Do not include any preamble or explanation.\n\n---\n\n{content}"
    );

    eprintln!("Compacting '{name}'...");

    let backend = Backend::Claude;
    let cmd = if let Some(vm) = sandbox {
        build_sandbox_command(vm, &backend, &prompt, None)
    } else {
        build_command(&backend, &prompt, None)
    };
    let (stdout, stderr, success) = execute_command(cmd, &backend);

    if !success {
        let bin = backend_name(&backend);
        eprintln!("{bin} failed: {stderr}");
        std::process::exit(1);
    }

    let summary = stdout.trim_end();

    let compacted = format!("{summary}\n\n");
    if let Err(e) = fs::write(&path, &compacted) {
        eprintln!("Failed to write {}: {e}", path.display());
        std::process::exit(1);
    }

    git_commit_conversation(&path, &format!("breo: compact '{name}'"), push);

    let tokens_after = estimate_tokens(&compacted);
    let saved = tokens_before.saturating_sub(tokens_after);
    let window = context_window(None, &backend);
    let remaining = window.saturating_sub(tokens_after);
    let pct_saved = if tokens_before > 0 {
        (saved as f64 / tokens_before as f64 * 100.0) as usize
    } else {
        0
    };

    eprintln!(
        "\n[{name}] Compacted {exchanges_before} exchanges\n\
         ~{} -> ~{} tokens ({pct_saved}% saved)\n\
         ~{} tokens remaining",
        format_tokens(tokens_before),
        format_tokens(tokens_after),
        format_tokens(remaining),
    );
}

fn cmd_send(
    message: &str,
    target: Option<&str>,
    model: Option<&str>,
    backend: &Backend,
    files: &[PathBuf],
    sandbox: Option<&str>,
    push: bool,
) {
    ensure_breo_dir();
    let active = get_active();
    let name = target.unwrap_or(&active);
    let path = conversation_path(name);

    let existing = if path.exists() {
        fs::read_to_string(&path).unwrap_or_default()
    } else {
        if target.is_none() {
            set_active(name);
        }
        format!("# Conversation: {name}\n\n")
    };

    let attachments = read_attached_files(files);
    let prompt = format!("{existing}## User\n{message}\n{attachments}");

    let cmd = if let Some(vm) = sandbox {
        build_sandbox_command(vm, backend, &prompt, model)
    } else {
        build_command(backend, &prompt, model)
    };
    let (stdout, stderr, success) = execute_command(cmd, backend);

    if !success {
        let bin = backend_name(backend);
        eprintln!("{bin} failed: {stderr}");
        std::process::exit(1);
    }

    let response = stdout.trim_end();

    println!("{response}");

    let content = format!("{prompt}\n## Assistant\n{response}\n\n");
    if let Err(e) = fs::write(&path, &content) {
        eprintln!("Failed to write {}: {e}", path.display());
        std::process::exit(1);
    }

    git_commit_conversation(&path, &format!("breo: message to '{name}'"), push);

    print_context_summary(&content, name, model, backend, &path);
}

fn main() {
    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    let config = load_config();

    let backend = cli.agent.unwrap_or(match config.agent.as_str() {
        "codex" => Backend::Codex,
        "gemini" => Backend::Gemini,
        _ => Backend::Claude,
    });

    let sandbox_name: Option<String> = if cli.no_sandbox {
        None
    } else if let Some(name) = cli.sandbox {
        Some(name)
    } else if config.sandbox {
        Some(config.sandbox_name.clone())
    } else {
        None
    };
    let sandbox = sandbox_name.as_deref();

    let push = if cli.no_push { false } else { config.push };

    match (cli.message, cli.command) {
        (_, Some(Commands::New { name })) => cmd_new(&name),
        (_, Some(Commands::Switch { name })) => cmd_switch(&name),
        (_, Some(Commands::List)) => cmd_list(),
        (_, Some(Commands::Pick)) => cmd_pick(),
        (_, Some(Commands::Setup { shell })) => cmd_setup(&shell),
        (_, Some(Commands::Compact { name })) => cmd_compact(name.as_deref(), sandbox, push),
        (Some(message), None) => cmd_send(
            &message,
            cli.conversation.as_deref(),
            cli.model.as_deref(),
            &backend,
            &cli.files,
            sandbox,
            push,
        ),
        (None, None) => {
            // Try reading from stdin if it's piped
            if !io::stdin().is_terminal() {
                let mut input = String::new();
                io::stdin().read_to_string(&mut input).unwrap_or_default();
                let input = input.trim();
                if !input.is_empty() {
                    cmd_send(
                        input,
                        cli.conversation.as_deref(),
                        cli.model.as_deref(),
                        &backend,
                        &cli.files,
                        sandbox,
                        push,
                    );
                    return;
                }
            }
            Cli::command().print_help().unwrap();
            println!();
        }
    }
}

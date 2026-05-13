use clap::{Args, Parser, Subcommand, ValueEnum};

/// OpenCode - The open source coding agent.
#[derive(Parser, Debug)]
#[command(
    name = "opencode",
    about = "The open source coding agent.",
    version,
    long_about = None,
    disable_version_flag = true,
    subcommand_required = false,
)]
pub struct Cli {
    /// Print logs to stderr
    #[arg(long)]
    pub print_logs: bool,

    /// Log level
    #[arg(long, value_enum)]
    pub log_level: Option<LogLevel>,

    /// Run without external plugins
    #[arg(long)]
    pub pure: bool,

    /// Print version number
    #[arg(short = 'v', long, global = true)]
    pub version: bool,

    /// Generate shell completion script
    #[arg(long, hide = true)]
    pub completion: Option<clap_complete::Shell>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the OpenCode terminal user interface
    Tui(TuiArgs),

    /// Run opencode with a message
    Run(Box<RunArgs>),

    /// Generate OpenAPI spec
    Generate,

    /// Manage console account (login/logout/switch/orgs/open)
    Console {
        #[command(subcommand)]
        subcommand: ConsoleSubcommand,
    },

    /// Manage AI providers and credentials (alias: auth)
    #[command(alias = "auth")]
    Providers {
        #[command(subcommand)]
        subcommand: ProvidersSubcommand,
    },

    /// Manage agents
    Agent {
        #[command(subcommand)]
        subcommand: AgentSubcommand,
    },

    /// Upgrade opencode to the latest or a specific version
    Upgrade(UpgradeArgs),

    /// Uninstall opencode and remove all related files
    Uninstall(UninstallArgs),

    /// List all available models
    Models(ModelsArgs),

    /// Start a headless opencode server
    Serve(NetworkArgs),

    /// Start opencode server and open web interface
    Web(NetworkArgs),

    /// Show token usage and cost statistics
    Stats(StatsArgs),

    /// Debugging and troubleshooting tools
    Debug {
        #[command(subcommand)]
        subcommand: DebugSubcommand,
    },

    /// Manage MCP (Model Context Protocol) servers
    Mcp {
        #[command(subcommand)]
        subcommand: McpSubcommand,
    },

    /// Manage GitHub agent
    Github {
        #[command(subcommand)]
        subcommand: GithubSubcommand,
    },

    /// Export session data as JSON
    Export(ExportArgs),

    /// Import session data from JSON file or URL
    Import(ImportArgs),

    /// Fetch and checkout a GitHub PR branch, then run opencode
    Pr(PrArgs),

    /// Manage sessions
    Session {
        #[command(subcommand)]
        subcommand: SessionSubcommand,
    },

    /// Install a plugin and update your config
    #[command(alias = "plug")]
    Plugin(PluginArgs),

    /// Database tools
    Db {
        #[command(subcommand)]
        subcommand: DbSubcommand,
    },

    /// Start ACP (Agent Client Protocol) server
    Acp(AcpArgs),

    /// Attach a terminal to a running opencode server
    Attach(AttachArgs),
}

// ─── TUI ─────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct TuiArgs {
    /// Project path
    pub project: Option<String>,

    /// Continue the last session
    #[arg(short = 'c', long)]
    pub r#continue: bool,

    /// Session ID to continue
    #[arg(short = 's', long)]
    pub session: Option<String>,

    /// Fork the session when continuing (use with --continue or --session)
    #[arg(long)]
    pub fork: bool,

    /// Prompt to use
    #[arg(long)]
    pub prompt: Option<String>,

    /// Model to use in the form of provider/model
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// Agent to use
    #[arg(long)]
    pub agent: Option<String>,

    #[command(flatten)]
    pub network: NetworkArgsMinimal,
}

#[derive(Args, Debug)]
pub struct NetworkArgsMinimal {
    /// Port to listen on
    #[arg(long)]
    pub port: Option<u16>,

    /// Hostname to listen on
    #[arg(long)]
    pub hostname: Option<String>,

    /// Enable mDNS discovery
    #[arg(long)]
    pub mdns: bool,

    /// Custom mDNS domain name
    #[arg(long)]
    pub mdns_domain: Option<String>,

    /// Additional browser origin(s) to allow CORS
    #[arg(long)]
    pub cors: Vec<String>,
}

// ─── Run ─────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Message to send
    #[arg(value_name = "MESSAGE")]
    pub message: Vec<String>,

    /// The command to run, use message for args
    #[arg(long)]
    pub command: Option<String>,

    /// Continue the last session
    #[arg(short = 'c', long)]
    pub r#continue: bool,

    /// Session ID to continue
    #[arg(short = 's', long)]
    pub session: Option<String>,

    /// Fork the session when continuing (use with --continue or --session)
    #[arg(long)]
    pub fork: bool,

    /// Share the session
    #[arg(long)]
    pub share: bool,

    /// Model to use in the form of provider/model
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// Agent to use
    #[arg(long)]
    pub agent: Option<String>,

    /// File(s) to attach to message
    #[arg(short = 'f', long)]
    pub file: Option<Vec<String>>,

    /// Format: default (formatted) or json (raw JSON events)
    #[arg(long, value_enum, default_value = "default")]
    pub format: RunFormat,

    /// Title for the session
    #[arg(long)]
    pub title: Option<String>,

    /// Attach to a running opencode server
    #[arg(long)]
    pub attach: Option<String>,

    /// Basic auth password (defaults to OPENCODE_SERVER_PASSWORD)
    #[arg(short = 'p', long)]
    pub password: Option<String>,

    /// Basic auth username (defaults to OPENCODE_SERVER_USERNAME or 'opencode')
    #[arg(short = 'u', long)]
    pub username: Option<String>,

    /// Directory to run in, or path on remote server if attaching
    #[arg(long)]
    pub dir: Option<String>,

    /// Port for the local server
    #[arg(long)]
    pub port: Option<u16>,

    /// Model variant (provider-specific reasoning effort, e.g., high, max, minimal)
    #[arg(long)]
    pub variant: Option<String>,

    /// Show thinking blocks
    #[arg(long)]
    pub thinking: Option<bool>,

    /// Run in direct interactive mode
    #[arg(short = 'i', long)]
    pub interactive: bool,

    /// Auto-approve permissions that are not explicitly denied
    #[arg(long)]
    pub dangerously_skip_permissions: bool,

    /// Enable direct interactive demo slash commands
    #[arg(long)]
    pub demo: bool,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum RunFormat {
    Default,
    Json,
}

// ─── Console ─────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum ConsoleSubcommand {
    /// Log in to console
    Login(ConsoleLoginArgs),
    /// Log out from console
    Logout(ConsoleLogoutArgs),
    /// Switch active org
    Switch,
    /// List orgs
    Orgs,
    /// Open active console account
    Open,
}

#[derive(Args, Debug)]
pub struct ConsoleLoginArgs {
    /// Server URL
    #[arg(value_name = "URL")]
    pub url: String,
}

#[derive(Args, Debug)]
pub struct ConsoleLogoutArgs {
    /// Account email to log out from
    pub email: Option<String>,
}

// ─── Providers (auth) ────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum ProvidersSubcommand {
    /// List providers and credentials
    #[command(alias = "ls")]
    List,
    /// Log in to a provider
    Login {
        /// Provider login URL
        url: Option<String>,

        /// Provider ID or name to log in to
        #[arg(short = 'p', long)]
        provider: Option<String>,

        /// Login method label
        #[arg(short = 'm', long)]
        method: Option<String>,
    },
    /// Log out of a provider
    Logout {
        /// Provider ID or name to log out from
        #[arg(short = 'p', long)]
        provider: Option<String>,
    },
}

// ─── Agent ───────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum AgentSubcommand {
    /// Create a new agent
    Create(AgentCreateArgs),
    /// List all available agents
    List,
}

#[derive(Args, Debug)]
pub struct AgentCreateArgs {
    /// Directory path to generate the agent file
    #[arg(long)]
    pub path: Option<String>,

    /// What the agent should do
    #[arg(long)]
    pub description: Option<String>,

    /// Agent mode
    #[arg(long, value_enum)]
    pub mode: Option<AgentMode>,

    /// Comma-separated list of permissions to allow (default: all)
    #[arg(long)]
    pub permissions: Option<String>,

    /// Model to use, in provider/model format
    #[arg(short = 'm', long)]
    pub model: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum AgentMode {
    All,
    Primary,
    Subagent,
}

// ─── Upgrade ─────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// Version to upgrade to (e.g., '0.1.48' or 'v0.1.48')
    pub target: Option<String>,

    /// Installation method to use
    #[arg(short = 'm', long, value_enum)]
    pub method: Option<InstallMethod>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum InstallMethod {
    Curl,
    Npm,
    Pnpm,
    Bun,
    Brew,
    Choco,
    Scoop,
}

// ─── Uninstall ───────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct UninstallArgs {
    /// Keep configuration files
    #[arg(short = 'c', long, default_value = "false")]
    pub keep_config: bool,

    /// Keep session data and snapshots
    #[arg(short = 'd', long, default_value = "false")]
    pub keep_data: bool,

    /// Show what would be removed without removing
    #[arg(long, default_value = "false")]
    pub dry_run: bool,

    /// Skip confirmation prompts
    #[arg(short = 'f', long, default_value = "false")]
    pub force: bool,
}

// ─── Models ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ModelsArgs {
    /// Provider ID to filter models by
    pub provider: Option<String>,

    /// Show more verbose model output (includes metadata like costs)
    #[arg(long)]
    pub verbose: bool,

    /// Refresh the models cache from models.dev
    #[arg(long)]
    pub refresh: bool,
}

// ─── Network (shared: serve, web, acp) ───────────────────────────────────────

#[derive(Args, Debug)]
pub struct NetworkArgs {
    /// Port to listen on
    #[arg(long)]
    pub port: Option<u16>,

    /// Hostname to listen on
    #[arg(long)]
    pub hostname: Option<String>,

    /// Enable mDNS discovery
    #[arg(long)]
    pub mdns: bool,

    /// Custom mDNS domain name
    #[arg(long)]
    pub mdns_domain: Option<String>,

    /// Additional browser origin(s) to allow CORS
    #[arg(long)]
    pub cors: Vec<String>,
}

// ─── Stats ───────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct StatsArgs {
    /// Show stats for the last N days (default: all time)
    #[arg(long)]
    pub days: Option<u64>,

    /// Number of tools to show (default: all)
    #[arg(long)]
    pub tools: Option<usize>,

    /// Show model statistics (pass a number for top N, flag for all)
    #[arg(long, num_args = 0..=1, require_equals = false, default_missing_value = "all")]
    pub models: Option<String>,

    /// Filter by project (default: all projects, empty string: current project)
    #[arg(long)]
    pub project: Option<String>,
}

// ─── Debug ───────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum DebugSubcommand {
    /// Show merged configuration with source attribution
    Config,
    /// Show debug information
    Info,
    /// Show global paths (data, config, cache, state)
    Paths,
    /// Wait indefinitely (for debugging)
    Wait,
    /// List all available skills
    Skill,
    /// Print startup timing
    Startup,
    /// Show agent configuration details
    Agent(DebugAgentArgs),
    /// LSP debugging utilities
    Lsp {
        #[command(subcommand)]
        subcommand: DebugLspSubcommand,
    },
    /// Ripgrep debugging utilities
    Rg {
        #[command(subcommand)]
        subcommand: DebugRgSubcommand,
    },
    /// File system debugging utilities
    File {
        #[command(subcommand)]
        subcommand: DebugFileSubcommand,
    },
    /// Snapshot debugging utilities
    Snapshot {
        #[command(subcommand)]
        subcommand: DebugSnapshotSubcommand,
    },
    /// Show LSP symbols for a query
    /// (kept for backwards compatibility at top level)
    Symbols(DebugSymbolsArgs),
    /// Show LSP document symbols
    DocumentSymbols(DebugDocumentSymbolsArgs),
    /// Show LSP diagnostics for a file
    /// (kept for backwards compatibility at top level)
    Diagnostics(DebugDiagnosticsArgs),
}

#[derive(Subcommand, Debug)]
pub enum DebugLspSubcommand {
    /// Get diagnostics for a file
    Diagnostics(DebugDiagnosticsArgs),
    /// Search workspace symbols
    Symbols(DebugSymbolsArgs),
    /// Get symbols from a document
    DocumentSymbols(DebugDocumentSymbolsArgs),
}

#[derive(Args, Debug)]
pub struct DebugDiagnosticsArgs {
    /// File path
    #[arg(value_name = "FILE")]
    pub file: String,
}

#[derive(Args, Debug)]
pub struct DebugSymbolsArgs {
    /// Symbol query
    #[arg(value_name = "QUERY")]
    pub query: String,
}

#[derive(Args, Debug)]
pub struct DebugDocumentSymbolsArgs {
    /// Document URI
    #[arg(value_name = "URI")]
    pub uri: String,
}

#[derive(Args, Debug)]
pub struct DebugAgentArgs {
    /// Agent name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Tool id to execute
    #[arg(long)]
    pub tool: Option<String>,

    /// Tool params as JSON
    #[arg(long)]
    pub params: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum DebugRgSubcommand {
    /// Show file tree using ripgrep
    Tree(DebugRgTreeArgs),
    /// List files using ripgrep
    Files(DebugRgFilesArgs),
    /// Search file contents using ripgrep
    Search(DebugRgSearchArgs),
}

#[derive(Args, Debug)]
pub struct DebugRgTreeArgs {
    /// Limit number of tree entries
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args, Debug)]
pub struct DebugRgFilesArgs {
    /// Filter files by query
    #[arg(long)]
    pub query: Option<String>,

    /// Glob pattern to match files
    #[arg(long)]
    pub glob: Option<String>,

    /// Limit number of results
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args, Debug)]
pub struct DebugRgSearchArgs {
    /// Search pattern
    #[arg(value_name = "PATTERN")]
    pub pattern: String,

    /// File glob patterns
    #[arg(long)]
    pub glob: Vec<String>,

    /// Limit number of results
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Subcommand, Debug)]
pub enum DebugFileSubcommand {
    /// Read file contents as JSON
    Read(DebugFileReadArgs),
    /// Show file status information
    Status,
    /// List files in a directory
    List(DebugFileListArgs),
    /// Search files by query
    Search(DebugFileSearchArgs),
    /// Show directory tree
    Tree(DebugFileTreeArgs),
}

#[derive(Args, Debug)]
pub struct DebugFileReadArgs {
    /// File path to read
    #[arg(value_name = "PATH")]
    pub path: String,
}

#[derive(Args, Debug)]
pub struct DebugFileListArgs {
    /// File path to list
    #[arg(value_name = "PATH")]
    pub path: String,
}

#[derive(Args, Debug)]
pub struct DebugFileSearchArgs {
    /// Search query
    #[arg(value_name = "QUERY")]
    pub query: String,
}

#[derive(Args, Debug)]
pub struct DebugFileTreeArgs {
    /// Directory to tree
    #[arg(value_name = "DIR", default_value = ".")]
    pub dir: String,
}

#[derive(Subcommand, Debug)]
pub enum DebugSnapshotSubcommand {
    /// Track current snapshot state
    Track,
    /// Show patch for a snapshot hash
    Patch(DebugSnapshotHashArgs),
    /// Show diff for a snapshot hash
    Diff(DebugSnapshotHashArgs),
    /// List available snapshots
    List,
    /// Show a specific snapshot
    Show(DebugSnapshotHashArgs),
}

#[derive(Args, Debug)]
pub struct DebugSnapshotHashArgs {
    /// Snapshot hash
    #[arg(value_name = "HASH")]
    pub hash: String,
}

// ─── MCP ─────────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum McpSubcommand {
    /// List MCP servers and their status
    #[command(alias = "ls")]
    List,
    /// Add an MCP server
    Add,
    /// Authenticate with an OAuth-enabled MCP server
    Auth(McpAuthArgs),
    /// Remove OAuth credentials for an MCP server
    Logout(McpLogoutArgs),
    /// Debug OAuth connection for an MCP server
    Debug(McpDebugArgs),
}

#[derive(Args, Debug)]
pub struct McpAuthArgs {
    /// Name of the MCP server
    pub name: Option<String>,

    #[command(subcommand)]
    pub subcommand: Option<McpAuthSubcommand>,
}

#[derive(Subcommand, Debug)]
pub enum McpAuthSubcommand {
    /// List OAuth-capable MCP servers and their auth status
    #[command(alias = "ls")]
    List,
}

#[derive(Args, Debug)]
pub struct McpLogoutArgs {
    /// Name of the MCP server
    pub name: Option<String>,
}

#[derive(Args, Debug)]
pub struct McpDebugArgs {
    /// Name of the MCP server
    #[arg(value_name = "NAME")]
    pub name: String,
}

// ─── GitHub ──────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum GithubSubcommand {
    /// Install the GitHub agent
    Install,
    /// Run the GitHub agent
    Run(GithubRunArgs),
}

#[derive(Args, Debug)]
pub struct GithubRunArgs {
    /// GitHub mock event to run the agent for
    #[arg(long)]
    pub event: Option<String>,

    /// GitHub personal access token
    #[arg(long)]
    pub token: Option<String>,
}

// ─── Export ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Session ID to export
    pub session_id: Option<String>,

    /// Redact sensitive transcript and file data
    #[arg(long)]
    pub sanitize: bool,
}

// ─── Import ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Path to JSON file or share URL
    #[arg(value_name = "FILE")]
    pub file: String,
}

// ─── PR ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PrArgs {
    /// PR number to checkout
    #[arg(value_name = "NUMBER")]
    pub number: u64,
}

// ─── Session ─────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum SessionSubcommand {
    /// List sessions
    List(SessionListArgs),
    /// Delete a session
    Delete(SessionDeleteArgs),
}

#[derive(Args, Debug)]
pub struct SessionListArgs {
    /// Limit to N most recent sessions
    #[arg(short = 'n', long)]
    pub max_count: Option<usize>,

    /// Output format (table or json)
    #[arg(long, value_enum, default_value = "table")]
    pub format: OutputFormat,
}

#[derive(Args, Debug)]
pub struct SessionDeleteArgs {
    /// Session ID to delete
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Table,
    Json,
}

// ─── Plugin ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PluginArgs {
    /// Plugin module to install
    #[arg(value_name = "MODULE")]
    pub module: String,

    /// Install in global config
    #[arg(short = 'g', long)]
    pub global: bool,

    /// Replace existing plugin version
    #[arg(short = 'f', long)]
    pub force: bool,
}

// ─── DB ──────────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum DbSubcommand {
    /// Run a SQL query or open interactive sqlite3 shell
    Query(DbQueryArgs),
    /// Print the database path
    Path,
    /// Migrate JSON data to SQLite
    Migrate,
}

#[derive(Args, Debug)]
pub struct DbQueryArgs {
    /// SQL query to execute
    pub query: Option<String>,

    /// Output format (json or tsv)
    #[arg(long, value_enum, default_value = "tsv")]
    pub format: DbFormat,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum DbFormat {
    Json,
    Tsv,
}

// ─── ACP ─────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct AcpArgs {
    /// Working directory
    #[arg(long)]
    pub cwd: Option<String>,

    #[command(flatten)]
    pub network: NetworkArgs,
}

// ─── Attach ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct AttachArgs {
    /// Server URL to attach to
    pub url: Option<String>,

    /// Working directory to start TUI in
    #[arg(long)]
    pub dir: Option<String>,

    /// Continue the last session
    #[arg(short = 'c', long)]
    pub r#continue: bool,

    /// Session ID to continue
    #[arg(short = 's', long)]
    pub session: Option<String>,

    /// Fork the session when continuing (use with --continue or --session)
    #[arg(long)]
    pub fork: bool,

    /// Basic auth password (defaults to OPENCODE_SERVER_PASSWORD)
    #[arg(short = 'p', long)]
    pub password: Option<String>,

    /// Basic auth username (defaults to OPENCODE_SERVER_USERNAME or 'opencode')
    #[arg(short = 'u', long)]
    pub username: Option<String>,
}

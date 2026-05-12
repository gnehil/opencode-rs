# OpenCode-RS

A Rust rewrite of the [OpenCode](https://github.com/sst/opencode) CLI tool - an AI-powered terminal assistant for developers.

## Overview

OpenCode-RS provides identical core functionality to the original TypeScript OpenCode, rewritten in Rust for better performance and reliability.

## Features

### Providers (27 supported)

| Provider | Streaming | Notes |
|----------|-----------|-------|
| Anthropic | ✓ SSE | Claude models |
| OpenAI | ✓ SSE | GPT models |
| Google | ✓ SSE | Gemini models |
| Azure | ✓ SSE | Azure OpenAI |
| xAI | ✓ SSE | Grok models |
| Groq | ✓ SSE | Llama, Mixtral |
| AWS Bedrock | ✓ | AWS hosted models |
| Google Vertex | ✓ | Vertex AI |
| Ollama | ✓ | Local models |
| Mistral | ✓ SSE | Mistral AI |
| OpenRouter | ✓ SSE | Multi-provider gateway |
| DeepSeek | ✓ SSE | DeepSeek models |
| Cohere | ✓ | Cohere models |
| Perplexity | ✓ SSE | Perplexity AI |
| Together AI | ✓ SSE | Together.ai |
| LMStudio | ✓ | Local models |
| Fireworks | ✓ SSE | Fireworks AI |
| DeepInfra | ✓ SSE | DeepInfra |
| Cerebras | ✓ SSE | Cerebras |
| Venice | ✓ SSE | Venice AI |
| Vercel | ✓ SSE | Vercel AI |
| Alibaba | ✓ | Alibaba Qwen |
| GitLab | ✓ | GitLab models |
| GitHub Copilot | ✓ | GitHub Copilot |
| Custom | ✓ | Custom endpoints |

### Tools (27 implemented)

**File Operations**
- `read` - Read file contents with pagination
- `write` - Write/create files
- `edit` - Edit files with exact string replacement
- `glob` - Find files by glob patterns
- `grep` - Search file contents with regex
- `patch` - Apply unified diff patches

**Code Analysis**
- `ast_grep_search` - AST-aware code search
- `ast_grep_replace` - AST-aware code replacement
- `lsp_*` - LSP integration (diagnostics, definitions, references, rename)
- `codesearch` - GitHub code search

**Execution**
- `bash` - Execute shell commands
- `interactive_bash` - TMUX-based interactive shell

**Session Management**
- `todo` - Todo list management
- `plan` - Planning tool
- `task` - Spawn background agents
- `question` - Ask user questions

**Web & Search**
- `webfetch` - Fetch web content
- `websearch` - Web search
- `repo_search` - Search GitHub repositories

**Background Operations**
- `background_output` - Get background task output
- `background_cancel` - Cancel background tasks

### Core Modules

#### ACP (Agent Communication Protocol)
- Full JSON-RPC 2.0 implementation
- Session management (create, load, fork, resume)
- Real prompt execution with provider integration
- Event streaming via SSE

#### PTY (Pseudo Terminal)
- Async-safe process spawning with `spawn_blocking`
- AtomicBool-based race condition handling
- Buffer management with cursor tracking
- Kill/remove coordination with wait thread

#### OAuth
- PKCE S256 code challenge
- axum HTTP callback server on port 19876
- Token refresh support
- Persistent token storage

#### File System
- **Watcher**: Real-time file monitoring via `notify`
- **Ignore**: Gitignore pattern matching
- **Protected**: Sensitive file protection (.env, credentials, keys)

#### Skill System
- SKILL.md discovery from project and global directories
- YAML frontmatter parsing
- Built-in `customize-opencode` skill

#### Worktree
- Git worktree management (create, list, remove, prune)
- Workspace isolation for sessions

### HTTP API (54 routes)

Built with `axum`:

```
/api/session/*      - Session CRUD operations
/api/message/*       - Message management
/api/permission/*    - Permission requests
/api/mcp/*           - MCP server management
/api/config/*        - Configuration
/api/file/*          - File operations
/api/agent/*         - Agent operations
/api/workspace/*     - Workspace management
```

### TUI

Built with `ratatui`:
- Chat interface with message rendering
- Sidebar with session list
- Help panel
- Status bar
- Toast notifications

## Installation

### Prerequisites

- Rust 1.70+ (with `rustup`)
- SQLite3

### Build

```bash
# Clone repository
git clone https://github.com/gnehil/opencode-rs.git
cd opencode-rs

# Build release
cargo build --release

# Install locally
cargo install --path crates/opencode
```

### Feature Flags

```bash
# Build with all providers
cargo build --release --features all-providers

# Build with specific providers
cargo build --release --features anthropic,openai,google

# Build with plugin support
cargo build --release --features plugins

# Build without self-update
cargo build --release --no-default-features --features anthropic,openai
```

## Usage

### CLI Commands

```bash
# Start TUI
opencode tui

# Run with prompt
opencode run "Fix the type error in auth.ts"

# Continue existing session
opencode run --session <session-id> "Continue fixing"

# List sessions
opencode session list

# Delete session
opencode session delete <session-id>

# List available models
opencode models --provider anthropic

# List providers
opencode providers list

# Start ACP server
opencode acp --cwd /path/to/project
```

### Environment Variables

```bash
# Provider API keys
ANTHROPIC_API_KEY=your-key
OPENAI_API_KEY=your-key
GOOGLE_API_KEY=your-key
XAI_API_KEY=your-key
GROQ_API_KEY=your-key

# Azure OpenAI
AZURE_OPENAI_API_KEY=your-key
AZURE_OPENAI_ENDPOINT=https://your-resource.openai.azure.com
AZURE_OPENAI_DEPLOYMENT=gpt-4o

# AWS Bedrock
AWS_ACCESS_KEY_ID=your-key
AWS_SECRET_ACCESS_KEY=your-secret

# Ollama (local)
OLLAMA_HOST=http://localhost:11434
```

### Configuration

Create `opencode.json` in your project root:

```json
{
  "agents": {
    "build": {
      "model": "anthropic/claude-3-5-sonnet-20241022",
      "system": "You are a helpful coding assistant."
    }
  },
  "tools": {
    "bash": {
      "timeout": 60000
    }
  },
  "mcp": {
    "servers": {
      "filesystem": {
        "type": "local",
        "command": "mcp-server-filesystem",
        "args": ["."]
      }
    }
  },
  "permissions": {
    "rules": [
      { "tool": "bash", "rule": "allow", "pattern": "git *" },
      { "tool": "write", "rule": "ask", "pattern": "*.rs" }
    ]
  },
  "skills": {
    "paths": ["./skills"]
  }
}
```

## Project Structure

```
opencode-rs/
├── crates/
│   ├── opencode/           # Main CLI application
│   │   └── src/
│   │       ├── acp/        # Agent Communication Protocol
│   │       ├── agent/      # Agent implementations
│   │       ├── bus/        # Event bus
│   │       ├── cli/        # CLI commands
│   │       ├── config/     # Configuration
│   │       ├── file/       # File operations
│   │       ├── git/        # Git utilities
│   │       ├── id/         # ID types
│   │       ├── mcp/        # MCP client & OAuth
│   │       ├── message/    # Message types
│   │       ├── permission/ # Permission system
│   │       ├── plugin/     # Plugin system
│   │       ├── provider/   # 27 LLM providers
│   │       ├── pty/        # Pseudo terminal
│   │       ├── server/     # HTTP API
│   │       ├── session/    # Session management
│   │       ├── skill/      # Skill system
│   │       ├── storage/    # SQLite storage
│   │       ├── tool/       # 27 tools
│   │       ├── tui/        # Terminal UI
│   │       ├── util/       # Utilities
│   │       └── worktree/   # Git worktree
│   ├── opencode-sdk/      # SDK library
│   └── opencode-plugin/   # Plugin SDK
├── Cargo.toml
└── Cargo.lock
```

## Comparison with TypeScript OpenCode

| Metric | Rust | TypeScript |
|--------|------|------------|
| Lines of Code | ~30,000 | ~83,000 |
| Source Files | 175 | ~200 |
| Startup Time | ~10ms | ~500ms |
| Memory Usage | ~20MB | ~100MB |

### Coverage

- **Core functionality**: 85-90%
- **Providers**: 100% (all have streaming)
- **Tools**: 96% (27/28)
- **Modules**: All critical modules implemented

### Not Implemented (Optional Cloud Features)

- `account` - User accounts (cloud)
- `control-plane` - Cloud sync
- `snapshot/sync` - Cloud snapshots
- `v2 API` - Newer API version

These are optional features not required for local CLI usage.

## Development

### Run Tests

```bash
cargo test
```

### Check Compilation

```bash
cargo check
cargo clippy
```

### Format Code

```bash
cargo fmt
```

## Dependencies

Key dependencies:
- `tokio` - Async runtime
- `axum` - HTTP framework
- `ratatui` - Terminal UI
- `sqlx` - SQLite database
- `reqwest` - HTTP client
- `serde` - Serialization
- `notify` - File watching
- `portable-pty` - PTY handling
- `async_stream` - Streaming macros

## License

MIT License

## Credits

- Original [OpenCode](https://github.com/sst/opencode) by SST
- Rust rewrite by the OpenCode-RS contributors
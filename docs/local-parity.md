# Local opencode parity matrix

Scope: local CLI/runtime/API parity with the TypeScript opencode checkout at
`/Users/gnehil/projects/opencode`. Cloud account, control-plane, cloud snapshot
sync, and v2 API are intentionally out of scope.

Legend:
- `done`: implemented and covered by current Rust tests or compile checks
- `partial`: usable, but behavior or UX is not yet equivalent
- `missing`: TS local feature exists and Rust does not yet expose it
- `skip`: intentionally excluded cloud/non-local surface

## CLI

| Area | TS reference | Rust reference | Status | Next work |
| --- | --- | --- | --- | --- |
| Top-level commands | `packages/opencode/src/cli/cmd/*.ts` | `crates/opencode/src/cli/args.rs` | partial | Keep option/alias parity as implementation catches up |
| `run` non-interactive | `cli/cmd/run.ts` | `cli/mod.rs`, `session/processor.rs` | partial | Align JSON event stream and command/shell prompt route behavior |
| `run --interactive` | `cli/cmd/run/runtime*.ts`, `footer*.tsx` | `tui/*`, `cli/local.rs` | partial | Split footer, permission/question prompt, scrollback, subagent frames |
| `tui` | `cli/cmd/tui/*` | `tui/*` | partial | Worker/internal transport, session validation, model/agent pickers |
| `attach` | `cli/cmd/tui/attach.ts` | `cli/local.rs` | partial | Launch real remote TUI instead of only validating/selecting session |
| `debug` | `cli/cmd/debug/*` | `cli/local.rs` | partial | Scrap/debug snapshots beyond git-backed fallback |
| `providers` | `cli/cmd/providers.ts` | `cli/provider_auth.rs`, `auth.rs`, `cli/mod.rs` | partial | Interactive provider selection and OAuth provider UX |
| `mcp` | `cli/cmd/mcp.ts` | `cli/mcp_cli.rs`, `mcp/*` | partial | needs_auth status, reauth prompts, remote reconnect parity |
| `github`/`pr` | `cli/cmd/github.ts`, `cli/cmd/pr.ts` | `cli/local_process.rs`, `cli/local.rs` | partial | Cross-repo PR remote setup, event simulation parity |
| `account`/console cloud | `cli/cmd/account.ts` | `cli/local.rs` console stubs | skip | Cloud scope |

## Server API

| Area | TS reference | Rust reference | Status | Next work |
| --- | --- | --- | --- | --- |
| Session list/create/get/update/delete | `server/.../groups/session.ts` | `server/handlers/session_handlers.rs` | partial | Request/response shape and workspace routing parity |
| Session route compatibility | `SessionPaths` | `server/routes.rs` | partial | Keep adding canonical `/session/...` aliases before `/api/session` legacy paths |
| Session messages | `SessionPaths.messages/message` | `server/handlers/message_handlers.rs` | partial | Broaden `MessageV2` shape tests and SDK compatibility checks |
| Session prompt | `prompt`, `prompt_async`, `command`, `shell` | `message_handlers.rs`, `command/*`, `session/history.rs`, `session/processor.rs`, `plugin/*` | partial | Route-level SDK shape checks and broader prompt variant parity |
| Revert/unrevert | `SessionPaths.revert/unrevert` | `session/service.rs`, `session_handlers.rs` | partial | Full restore semantics after revert, not only clearing the marker |
| Todo/diff/init | `SessionPaths.todo/diff/init` | `session_handlers.rs`, `session/service.rs` | partial | Replace git working-tree diff fallback with exact snapshot/message diff parity |
| Share/unshare | `SessionPaths.share` | none | skip | Cloud share scope |
| File/find API | `groups/file.ts` | `server/handlers/file_handlers.rs` | partial | Replace `/find/symbol` empty fallback with LSP-backed search |
| SSE event API | `event.ts` | `server/handlers/event_handlers.rs` | partial | Align remaining Rust-only event type names/properties with TS bus schemas |
| TUI control | `groups/tui.ts`, `groups/control.ts` | `tui_handlers.rs`, `tui/control.rs` | partial | Wire queue producers from the interactive TUI runtime |
| Pty | `groups/pty.ts` | `pty/*`, `server/handlers/pty_handlers.rs` | partial | Add browser-origin/auth fallback nuance and WebSocket E2E coverage |
| Sync | `groups/sync.ts` | `workspace_handlers.rs` placeholders | skip | Cloud sync scope |

## Runtime Modules

| Area | TS reference | Rust reference | Status | Next work |
| --- | --- | --- | --- | --- |
| Providers/model auth | `provider/*`, `auth/index.ts` | `provider/*`, `auth.rs`, `cli/provider_auth.rs` | partial | Per-provider auth schema and live smoke tests |
| Tools | `tool/*` | `tool/*` | partial | Validate behavior, output shape, permission integration per tool |
| Permission ask | `permission/index.ts` | `permission/broker.rs`, `tool/context.rs` | done | Broaden HTTP/UI reply integration |
| MCP runtime | `mcp/*` | `mcp/*`, `cli/mcp_cli.rs` | partial | OAuth state/status, unauthorized reconnect parity |
| Plugin runtime | `cli/cmd/tui/plugin/*`, plugin hooks | `plugin/*`, `session/processor.rs`, `message_handlers.rs`, `cli/mod.rs` | partial | External JS/TS plugin loading and TUI plugin slots |
| LSP | `lsp/*` | `lsp/*`, `tool/lsp.rs` | partial | Long-lived pool behavior and diagnostics shape |

## Current priority queue

1. Run JSON command/session behavior against the server routes.
2. External JS/TS plugin loading/runtime compatibility.
3. Remote TUI attach/control queue and TUI prompt execution.
4. MCP needs_auth/reconnect parity.
5. Provider/model/auth dynamic loading and stored credential use beyond API keys.

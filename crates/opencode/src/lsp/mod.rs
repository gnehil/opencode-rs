//! LSP client infrastructure.
//!
//! We talk to language servers (rust-analyzer, typescript-language-server,
//! etc.) over their JSON-RPC stdio protocol. Submodules:
//!
//!   - `framing`: pure Content-Length / JSON framing.
//!   - `registry`: language → server-spawn-command mapping.
//!   - `client`:   async client (spawn + handshake + request/notify).
//!
//! Higher-level callers (`tool/lsp.rs`) compose these into operations
//! like "give me the diagnostics for this file".

pub mod client;
pub mod diagnostics;
pub mod framing;
pub mod ops;
pub mod pool;
pub mod registry;

pub use client::{LspClient, LspError, ServerNotification};
pub use registry::{for_path as server_for_path, ServerSpec, BUILTIN as BUILTIN_SERVERS};

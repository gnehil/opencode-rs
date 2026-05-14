//! Subprocess bridge for external JS/TS plugins.
//!
//! opencode plugins are JavaScript/TypeScript modules that expect a real
//! node/bun environment. Rather than embedding a JS engine, the bridge runs
//! a small JS host (`host.mjs`) in a node/bun subprocess and exchanges
//! newline-delimited JSON-RPC with it (see [`protocol`]).

pub mod protocol;

pub use protocol::{
    HostEvent, HostRequest, LoadedPlugin, PluginInputData, PluginLoadError, PluginToLoad,
    HOST_SCRIPT,
};

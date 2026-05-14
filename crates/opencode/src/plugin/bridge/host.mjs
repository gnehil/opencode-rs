// opencode external plugin bridge host.
//
// Runs under node or bun. Speaks newline-delimited JSON-RPC over stdio with
// the Rust side: stdin carries requests (init / trigger / notify / shutdown),
// stdout carries responses (ready / trigger_result / trigger_error / log).
// stderr is left for the runtime's own noise and is not part of the protocol.

import { createInterface } from "node:readline"
import { createRequire } from "node:module"
import { fileURLToPath, pathToFileURL } from "node:url"

// Loaded plugins, in registration order. Each entry: { spec, hooks }.
const plugins = []

function send(msg) {
  process.stdout.write(JSON.stringify(msg) + "\n")
}

function log(level, message) {
  send({ type: "log", level, message })
}

function errorText(err) {
  if (err && err.stack) return String(err.stack)
  if (err && err.message) return String(err.message)
  return String(err)
}

// Build the opencode SDK `client` for a plugin by resolving `@opencode-ai/sdk`
// from the plugin's own location. Real opencode plugins depend on the SDK, so
// resolving relative to the plugin entry finds the version it was built
// against. Returns `undefined` when the SDK is not installed near the plugin;
// plugins that only use lifecycle hooks do not need it.
async function buildClient(data, entry) {
  try {
    const base = entry.startsWith("file://") ? fileURLToPath(entry) : entry
    const require = createRequire(base)
    const sdkPath = require.resolve("@opencode-ai/sdk")
    const sdk = await import(pathToFileURL(sdkPath).href)
    if (typeof sdk.createOpencodeClient !== "function") return undefined
    return sdk.createOpencodeClient({
      baseUrl: data.server_url,
      directory: data.directory,
    })
  } catch (err) {
    log("warn", `plugin ${entry}: opencode SDK client unavailable: ${errorText(err)}`)
    return undefined
  }
}

async function pluginInput(data, entry) {
  let serverUrl = data.server_url
  try {
    serverUrl = new URL(data.server_url)
  } catch {}
  return {
    client: await buildClient(data, entry),
    project: data.project,
    directory: data.directory,
    worktree: data.worktree,
    experimental_workspace: { register() {} },
    serverUrl,
    $: typeof Bun === "undefined" ? undefined : Bun.$,
  }
}

// Extract the plugin function from an imported module, matching the
// TypeScript loader: a PluginModule exposes `server`, otherwise accept a
// bare function export or `default`, or any function found among the exports.
function extractPluginFn(mod) {
  if (typeof mod === "function") return mod
  if (mod && typeof mod === "object") {
    if (typeof mod.server === "function") return mod.server
    if (typeof mod.default === "function") return mod.default
    if (mod.default && typeof mod.default.server === "function") return mod.default.server
    for (const value of Object.values(mod)) {
      if (typeof value === "function") return value
      if (value && typeof value === "object" && typeof value.server === "function") {
        return value.server
      }
    }
  }
  return undefined
}

async function init(msg) {
  const loaded = []
  const errors = []
  for (const plugin of msg.plugins) {
    try {
      // The SDK client is resolved per plugin so each gets a client built
      // against the `@opencode-ai/sdk` shipped alongside it.
      const input = await pluginInput(msg.input, plugin.entry)
      const mod = await import(plugin.entry)
      const fn = extractPluginFn(mod)
      if (!fn) throw new Error("plugin export is not a function")
      const hooks = (await fn(input, plugin.options ?? undefined)) ?? {}
      plugins.push({ spec: plugin.spec, hooks })
      loaded.push({ spec: plugin.spec, hooks: Object.keys(hooks) })
    } catch (err) {
      errors.push({ spec: plugin.spec, error: errorText(err) })
    }
  }
  send({ type: "ready", plugins: loaded, errors })
}

// A trigger-style hook: called with (input, output); the plugin mutates
// `output` in place. Hooks run sequentially in plugin registration order so
// the result is deterministic.
async function trigger(msg) {
  const output = msg.output
  try {
    for (const { hooks } of plugins) {
      const fn = hooks[msg.hook]
      if (typeof fn !== "function") continue
      await fn(msg.input, output)
    }
    send({ type: "trigger_result", id: msg.id, output })
  } catch (err) {
    send({ type: "trigger_error", id: msg.id, error: errorText(err) })
  }
}

// A one-way notification hook (`event`, `config`): called with input only.
// Failures are logged but never surfaced as protocol errors.
async function notify(msg) {
  for (const { hooks } of plugins) {
    const fn = hooks[msg.hook]
    if (typeof fn !== "function") continue
    try {
      await fn(msg.input)
    } catch (err) {
      log("error", `plugin ${msg.hook} hook failed: ${errorText(err)}`)
    }
  }
}

async function handle(line) {
  const text = line.trim()
  if (!text) return
  let msg
  try {
    msg = JSON.parse(text)
  } catch {
    log("error", "invalid json on host channel")
    return
  }
  switch (msg.type) {
    case "init":
      await init(msg)
      break
    case "trigger":
      await trigger(msg)
      break
    case "notify":
      await notify(msg)
      break
    case "shutdown":
      process.exit(0)
      break
    default:
      log("warn", `unknown request type: ${msg.type}`)
  }
}

const rl = createInterface({ input: process.stdin })

// Process requests strictly in order: chaining onto a single promise keeps
// hook registration and execution deterministic even though readline emits
// `line` events without waiting for the async handler.
let queue = Promise.resolve()
rl.on("line", (line) => {
  queue = queue.then(() => handle(line)).catch((err) => log("error", errorText(err)))
})
rl.on("close", () => process.exit(0))

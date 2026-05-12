# AGENTS.md

这是给 AI 代理（如 Claude、GPT 等）的项目指南。

## 项目概述

OpenCode-RS 是 OpenCode CLI 工具的 Rust 重写版本，提供与 TypeScript 版本相同的核心功能。

## 代码结构

### 核心模块

```
crates/opencode/src/
├── acp/          # Agent Communication Protocol
│   ├── agent.rs  # ACP 代理实现，处理 JSON-RPC 请求
│   ├── server.rs # JSON-RPC stdio 服务器
│   ├── session.rs # 会话管理
│   └── types.rs  # 协议类型定义（~750行）
│
├── provider/     # LLM 提供商（27个）
│   ├── anthropic.rs # Anthropic Claude
│   ├── openai.rs    # OpenAI GPT
│   ├── google.rs    # Google Gemini
│   ├── xai.rs       # xAI Grok
│   └── ...          # 其他提供商
│
├── tool/         # 工具实现（27个）
│   ├── bash.rs      # Shell 命令执行
│   ├── read.rs      # 文件读取
│   ├── write.rs     # 文件写入
│   ├── edit.rs      # 文件编辑
│   └── ...
│
├── pty/          # 伪终端模块
│   └── mod.rs    # PTY 会话管理
│                  # - spawn_blocking 处理阻塞操作
│                  # - AtomicBool 竞态协调
│
├── mcp/          # MCP 客户端
│   ├── oauth.rs  # OAuth 认证（PKCE S256）
│   ├── client.rs # MCP 协议客户端
│   └── manager.rs # MCP 服务器管理
│
├── session/      # 会话管理
│   ├── processor.rs # 提示处理器
│   └── service.rs   # 会话存储
│
├── server/       # HTTP API（54路由）
│   ├── routes.rs   # 路由定义
│   └── handlers/   # 各类处理器
│
├── tui/          # 终端 UI（ratatui）
│   ├── app.rs      # 主应用
│   └── components/ # UI 组件
│
├── file/         # 文件系统
│   ├── watcher.rs  # 文件监控
│   ├── ignore.rs   # Gitignore 匹配
│   └── protected.rs # 保护文件检测
│
├── skill/        # 技能系统
│   └ skill_service.rs # SKILL.md 发现
│   └── customize-opencode.md # 内置技能
│
└── worktree/     # Git 工作树
    └── worktree_service.rs # 工作树管理
```

## 关键设计决策

### PTY 竞态处理

使用 `AtomicBool` 协调 kill 和 wait 操作：

```rust
pub struct PtySession {
    killed: Arc<AtomicBool>,  // 是否被 kill
    exited: Arc<AtomicBool>,   // 是否已退出
}

// kill() 设置 killed=true，wait 线程检查后跳过状态更新
```

### 流式传输实现

所有 Provider 使用 `async_stream::try_stream!` 宏：

```rust
fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
    let stream = async_stream::try_stream! {
        // SSE 解析逻辑
        while let Some(chunk) = stream_reader.next().await.transpose()? {
            // 解析 data: 前缀
            // 返回 StreamEvent::text_delta()
        }
    };
    Ok(Box::pin(stream))
}
```

### ACP 提示执行

真实的提示执行流程：

```rust
pub async fn handle_prompt(&self, params: Value) -> Result<Value> {
    // 1. 解析请求
    // 2. 创建用户消息
    // 3. 调用 provider.complete()
    // 4. 创建助手消息
    // 5. 返回 PromptResponse
}
```

## 验证状态

Oracle Round 11 验证结果：
- ✅ 所有流式传输已实现（26/26 Provider）
- ✅ ACP handle_prompt 执行真实提示
- ✅ 所有工具实现（27/28）
- ✅ PTY 竞态修复验证
- ✅ OAuth 验证
- ✅ 无 TODO/FIXME/stub

## 未实现功能（可选）

以下为云端功能，非 CLI 核心：
- `account` - 用户账户
- `control-plane` - 云端同步
- `snapshot/sync` - 云端快照
- `v2 API` - 新版 API

## 调试建议

### 查看模块结构

```bash
find crates/opencode/src -type f -name "*.rs" | wc -l
# 结果：175 文件
```

### 检查特定模块

```bash
# PTY 模块
cat crates/opencode/src/pty/mod.rs

# 流式传输示例（xAI）
cat crates/opencode/src/provider/xai.rs | grep -A30 "fn stream"
```

### 验证无 stub

```bash
grep -r "placeholder" crates/opencode/src/
grep -r "TODO" crates/opencode/src/
grep -r "stub" crates/opencode/src/
# 应无结果
```

## 构建与测试

```bash
cargo build --release
cargo test
cargo clippy
```

## 与 TypeScript 版本对比

| 模块 | Rust 状态 | TypeScript |
|------|-----------|------------|
| acp | ✅ 完成 | 原版 |
| provider | ✅ 27个 | 原版 |
| tool | ✅ 27个 | 原版 |
| pty | ✅ 完成 | 原版 |
| mcp | ✅ 完成 | 原版 |
| tui | ✅ ratatui | Ink |
| server | ✅ axum | Hono |
| storage | ✅ SQLite | Drizzle |

## 注意事项

1. **Provider 流式传输**：所有 Provider 使用 SSE，非 stub
2. **PTY 阻塞处理**：使用 `spawn_blocking` 避免 tokio 阻塞
3. **OAuth base64**：使用 base64 0.22 新 API（`URL_SAFE_NO_PAD.encode()`）
4. **EventBus**：使用 `broadcast` channel 实现事件订阅
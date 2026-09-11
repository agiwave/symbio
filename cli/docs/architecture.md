# 架构

`symbio-cli` 与 `tauri/` 调用的是**同一套上下文键**（`PATH` / `PAYLOAD` / `SESSION_ID` /
`WORKDIR` / …）和**同一份协议**，差别只在「传输层」。Tauri 走 `route_v2` IPC；本 CLI 直接拿到
`Arc<dyn Plugin>` 根节点在**进程内**调用 `root.route(ctx)`。因此本 CLI 的引入**零侵入**
`symbio/` 与协议。

```
                 ┌────────────────────┐
   tauri/  ──────▶│  route_v2 (IPC)    │──┐
                 └────────────────────┘  │      ┌───────────────────────────┐
                                         ├─────▶│  home → worker(composite) │
                 ┌────────────────────┐  │      │   ├─ session              │
   cli/    ──────▶│ root.route(ctx)    │──┘      │   ├─ model                │
                 │  （进程内直连）      │         │   ├─ event_bus  …         │
                 └────────────────────┘         └───────────────────────────┘
```

## 1. 进程内客户端（`client.rs`）

`SymbioClient::start()` 做的事：

1. **注入系统目录**：`std::env::set_var("SYMBIO_HOMEDIR", homedir)` —— 这是
   `HomedirRegistry` 的最高优先级来源（高于 `~/.symbio_bootstrap` 与 `~/.symbio`），
   必须在任何 `HomedirRegistry::get()` 之前设置（插件树构造期就会读它）。
2. `create_root_plugin().await` 构造整个进程内插件树。
3. 订阅 `event_bus/subscribe`（`SubscribeRequest { kinds: None }`），一条连接收全部事件。
4. 起一个 `tokio::spawn` 转发任务，把 `PluginFrame` 解包成 `BusEvent` 塞进 `mpsc::UnboundedReceiver`。

### 为什么选 `event_bus` 而不是 session 私有通道

后端下行有三条通道：① session 私有长连接（需先 `session/open`）；② 全局 `EventBus`
（`event_bus/subscribe` 一条连接收全部事件）；③ 前端主动拉取。

CLI 选 ②：它与「当前打开哪个会话」**解耦** —— 订阅一次即可覆盖之后所有会话，切会话不必
重建连接。这恰好也是 Tauri 前端会话列表实时更新的同一机制。代价是转发任务里要按
`session_id` 过滤本会话的事件。

### 上行：发送一条消息

`ask()` 构造 `session_chat::Request`（User/Text 消息 + `provider_id` / `mode`），经
`route("session/chat/send", …)` 发送，等响应 `status == "accepted"` 后进入事件循环。

`route<T>()` 是通用封装：包 `Arc::new(SimpleRequest::new(None, None))`，设 `PATH` /
`WORKDIR` / `SESSION_ID` / `payload`（`InvokeRequestExt::set_payload`），再 `.route(ctx)`。
走进程内强类型通道，**无需为这些请求实现 `Serialize`**。

### 会话元数据落库

`ensure_session()` 写 `session/update`，把 `workdir` / `mode` / `risk_level` / `provider_id`
（可选）/ `agent_id`（可选）写进会话元数据。这些是后端 `resolve_session_params` 的回退来源：
会话一旦绑定，后续每次发送都不必重复携带。`/workdir` 等 REPL 内改动后需重新调用一次使其落库。

## 2. 下游事件循环与完成判定

循环 `self.events.recv()`（带 `TURN_TIMEOUT = 900s` 兜底），把 `StreamEvent` 解出来：

| 事件 | 处理 |
| --- | --- |
| `Update { message }` | 交给渲染器增量合并 |
| `Delete { message_id }` | 渲染器按 id 删除快照 |
| `Status { status }` | 渲染器更新状态；`status == "idle"` ⇒ 退出循环 |
| `Error { error }` | 记录业务错误（**不**立即中断，继续等 `idle` 收敛） |
| `Abort` | 结束循环 |
| `Connected` / `Disconnected` / `SessionResumed` | 忽略 |

完成判定与前端 `sessionBusWatcher` 一致：`Status{idle}` 或 `Abort`。业务错误只记录，仍等
`idle` 收敛，以免把「还有后续帧」误判成结束。`drain_stale()` 在每轮 `ask` 前清空上一轮残留事件，
避免旧补丁被重复渲染。

## 3. 终端渲染器（`render.rs`）

| 通道 | 内容 |
| --- | --- |
| **stdout** | 模型正文 + 交互提示符（可直接管道给下游程序） |
| **stderr** | 进度、工具调用、错误、插件日志（可整体 `2>/dev/null` 静音） |

按 `ChatMessage::apply_patch` 做增量合并（Text/Reasoning append，Turn/ToolCall 全量替换）；
渲染器按消息 id 维护快照。`on_update` 只把**模型 Text 增量**写 stdout，Reasoning 仅在
`--verbose` 时落 stderr，ToolCall 在 stderr 一次性 announce。刻意抑制 User/Tool/System 角色
回声，保持 stdout 纯净。`wrote_text` 标记是否有正文输出（无正文则非交互模式判为失败）。

### 为什么不调用 `symbio::init::initialize()`

那个函数会安装写 **stdout** 的 tracing subscriber（默认过滤 `info,symbio=debug`），把大量插件
日志混进模型正文。不初始化时，插件日志宏退回 `eprintln!`，正好落在 stderr。stdout 因此保持纯净，
可直接 `| jq` 或喂给下游程序。

## 4. 模式选择逻辑（`main.rs`）

判定顺序（显式消息 > 强制 REPL > 管道输入 > 交互 REPL）：

```rust
if args.message.is_some() {                 // ① -m / 位置参数
    return run_once(client, args.message.clone(), &args).await;
}
if args.repl {                             // ② --repl / -i（强制 REPL）
    return run_repl(client, &args).await;
}
if !std::io::stdin().is_terminal() {       // ③ 管道 / 重定向（stdin 非终端）
    return run_once(client, None, &args).await;
}
run_repl(client, &args).await              // ④ 终端默认进 REPL
```

`-i/--repl` 的存在是因为：默认「stdin 非终端 ⇒ 走管道」对 `echo x | cli` 正确，但会挡住两类场景
——① 终端不支持 TTY 时仍想用 REPL；② 用脚本喂多轮输入来自动化验证会话。显式开关两全。

## 5. REPL 内置命令

`/help`、`/new`（新建会话）、`/session <ID>`（切换/续接）、`/provider <ID>`（`default` =
回退系统默认）、`/workdir <路径>`、`/exit`（`/quit`）。命令在 `handle_command()` 中处理，`/new`
用 `chrono_like_stamp()`（基于 `SystemTime` 的简易十六进制戳，避免为这一个用途引入时间库）生成会话 id。

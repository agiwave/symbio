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
3. **两条下行连接**（各归其域，见下节）：
   - `session/stream` —— **消息实时面**。一条流、显式操作（`NodeOp`）、流内单调 `seq`、
     按帧里的 `session_id` 归属；后端**广播**给全部订阅者。
   - `event_bus/subscribe`（`SubscribeRequest { kinds: None }`）+ `vdfs/watch(<根>/session/<sid>)`
     —— **会话运行态**。前者是收件地址、后者是开闸；后端只向登记过路径的订阅者投递变更
     （`ChangeSubscriptions`），因此只做订阅是一条永远不响的频道。
4. 两条连接各起一个 `tokio::spawn` 转发任务，把 `PluginFrame` 解包成 `Frame`（`Transcript` /
   `Resync` / `Node`）后塞进**同一个** `mpsc::UnboundedReceiver`——调用侧只面对一个接收端。

### 方向选择：为什么是「两条」而不是「一条」

分治依据是**语义**，不是实现巧合：

| 域 | 通道 | 特征 |
| --- | --- | --- |
| 消息实时面 | `session/stream` | 高频突发；与「当前打开哪个会话」无关；每条消息归属一个会话 |
| 会话运行态 | `event_bus` + `vdfs/watch` | 低频；状态是**节点属性**；与侧栏会话清单共用同一份订阅 |

曾经消息也寄生在 VDFS 变更上（`kind = "vdfs"`），那条路有两个结构性缺陷：VDFS 变更
**没有流内序号**（丢一帧不可检测，表现为「工具一直进行中、刷新即愈」），且载荷是全量而
渲染器要增量，中间必须有人猜「追加还是替换」。现在每帧自带完整事实（`upsert` 全量快照 /
`append` 裸增量）、自带 `seq`，折算层整个消失。

会话域曾经还另有一条 `kind = "session"` 的事件频道（`Status` / `Update` / `Abort` 帧），
已随「状态即节点属性」整体废除（见 `symbio/src/plugins/session/docs/node-state-streaming.md`）。

`event_bus` 在本 CLI 里**只是传输层**。长连接而不是每会话一条私有通道：它与「当前打开哪个
会话」解耦，切会话只需换一个 `vdfs/watch` 地址，不必重建连接。代价是消费侧按**地址前缀**
过滤（不是按 `session_id`——VDFS 帧的业务身份全在 `VdfsChange::path` 里，`session_id` 是死字段）。

### 上行：发送一条消息

`ask()` 构造 `session_chat::Request`（User/Text 消息 + `provider_id` / `mode`），经
`route("session/chat/send", …)` 发送，等响应 `status == "accepted"` 后进入变更循环。

`route<T>()` 是通用封装：包 `Arc::new(SimpleRequest::new(None, None))`，设 `PATH` /
`WORKDIR` / `SESSION_ID` / `payload`（`InvokeRequestExt::set_payload`），再 `.route(ctx)`。
走进程内强类型通道，**无需为这些请求实现 `Serialize`**。

### 会话元数据落库

`ensure_session()` 写 `session/update`，把 `workdir` / `mode` / `risk_level` / `provider_id`
（可选）/ `agent_id`（可选）写进会话元数据。这些是后端 `resolve_session_params` 的回退来源：
会话一旦绑定，后续每次发送都不必重复携带。`/workdir` 等 REPL 内改动后需重新调用一次使其落库。

## 2. 下游帧循环与完成判定

循环 `self.events.recv()`（带 `TURN_TIMEOUT = 900s` 兜底），每帧按变元分派：

| 帧 | 来源 | 处理 |
| --- | --- | --- |
| `Frame::Transcript` | `session/stream` | 过滤掉非当前会话；`seq <= last` 丢弃（重复帧），跳号则告警留痕；随后按 `NodeOp` 直接落地：`upsert` ⇒ `on_upsert`、`append` ⇒ `on_append`、`remove` ⇒ `on_remove`、`reset` ⇒ 快照作废 |
| `Frame::Resync` | `session/stream` | 后端的背压标记（通道曾满）。CLI 无历史可重读，告警留痕 |
| `Frame::Node` | `event_bus` + `vdfs/watch` | **会话叶子**（`path == 会话地址`）承载运行态：`status == working` ⇒ 提示「处理中」；离开 `working` ⇒ **本轮结束**，退出循环。比会话叶子更深的 VDFS 变更已不再是实时面，忽略 |

完成判定是**会话节点自己的 `status`**（不再有 `Status{idle}` 帧可等）。结局从同一次变更的
`node.attributes` 读：`outcome == aborted` ⇒ 报中止，`status == failed` / `error` 非空 ⇒ 该错误
作为本轮的返回值（退出码非 0）。

**为什么结束判据只能看会话节点**：一轮请求里模型会多次定格根 Turn 节点（每个工具轮次一次），
所以「Turn 到达终态」只说明这一轮*模型输出*结束，不说明整轮请求结束——转写流本身无法回答
「还有没有下一轮」。

两个细节：

- **转写流为什么要做 `seq` 判定**：后端每帧单调 +1，跳号 = 已知有损。渲染器只在单轮内做增量
  合并，重连窗口内重发的 `append` 会把增量叠两次，因此 `seq <= last` 必须丢弃。
- **节点变更也包含标题 / 元数据写入**，所以运行态提示只在**迁移**上报一次
  （`last_status`）；`drain_stale()` 在每轮 `ask` 前清空上一轮残留帧，避免旧帧被重复渲染。

## 3. 终端渲染器（`render.rs`）

| 通道 | 内容 |
| --- | --- |
| **stdout** | 模型正文 + 交互提示符（可直接管道给下游程序） |
| **stderr** | 进度、工具调用、错误、插件日志（可整体 `2>/dev/null` 静音） |

渲染器直接消费 `NodeOp`，不再有补丁合并：`on_append` 把裸增量累积进本地快照并**直接输出**
（流式热路径，O(delta)）；`on_upsert` 用全量快照整条替换，与快照做差分算出「还没打印过的
那一段」（收尾帧通常为空，后端若直接给全量则整段输出，不丢字）；`on_remove` 丢快照。

只把**模型 Text** 写 stdout，Reasoning 仅在 `--verbose` 时落 stderr，ToolCall 在 stderr 一次性
announce。刻意抑制 User/Tool/System 角色回声，保持 stdout 纯净。`wrote_text` 标记是否有正文输出
（无正文则非交互模式判为失败）。

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

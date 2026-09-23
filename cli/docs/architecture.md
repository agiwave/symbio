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
3. **一条下行订阅**（见下节）：`event_bus/subscribe` —— `vdfs` 频道（消息 +
   会话运行态 + 资源信号），归属由信封的 `path` 给出（`<根>/session/<sid>/…`）。
4. 该订阅起一个 `tokio::spawn` 转发任务，把 `PluginFrame` 解包成 `Frame`
   （`Change` / `Resync`）后塞进 `mpsc::UnboundedReceiver`——调用侧只面对一个接收端。

### 方向选择：为什么是「一条」

| 域 | 通道 | 特征 |
| --- | --- | --- |
| 消息实时面 + 会话运行态 + 资源信号 | `event_bus` 的 `vdfs` 频道 | 一条订阅、单一 FIFO、按路径归属 |

**曾经是两条**：① `session/stream` 收消息、② `event_bus/subscribe` + `vdfs/watch` 收运行态。
两条的到达顺序没有机制保证，于是「会话报不忙」推不出「本轮消息都已终态」——前端因此挂了一张
宽限期复查的兜底网（`reconcileTranscript`）。批次 E 把运行态并进同一条流后，兜底网随之删除
——不是被更强的网替代，而是它要补的那个缺口不再存在。

**ADR-025 已落地**（2026-09-23，S27 收口）：实时面**迁回 VDFS 变更**，`session/stream` 与
`symbio_core::transcript_stream` 一并退役，本 CLI 订阅 `vdfs` 频道。**批次 E 的合并理由
（「需要顺序保证」）与 S23–S25 的拆分理由（「没有流内序号」）是同一个错误**：把「数据的属性」
当成了「传输的属性」——顺序由**单一订阅连接**给（单一 FIFO），不由帧里的序号给。
信封没有操作枚举（S27）：`{path, data?}`，语义全在 `data` 的字段上。



循环 `self.events.recv()`（带 `TURN_TIMEOUT = 900s` 兜底），每帧按变元分派：

| 帧 | 来源 | 处理 |
| --- | --- | --- |
| `Frame::Change`（落点 = 消息目录 `<sid>/消息`） | `vdfs` 频道 | `data` 就是那条 `ChatMessage`（身份在 `data.id`），直接落地（`on_message`）：`delta` ⇒ 追加、`content` ⇒ 整条替换、`status = removed` ⇒ 丢快照 |
| `Frame::Change`（落点 = 会话叶子 `<sid>`） | `vdfs` 频道 | **会话运行态**：`data` 带全量节点视图（零回读），缺失 ⇒ 回读 `stat`；`status == working` ⇒ 提示「处理中」；离开 `working` ⇒ **本轮结束**，退出循环 |
| `Frame::Change`（其余路径） | `vdfs` 频道 | 记忆 / 工作目录 / 插件等资源信号，与本轮渲染无关，丢弃 |
| `Frame::Resync` | `vdfs` 频道 | 后端的背压标记（通道曾满）。CLI 无历史可重读，告警留痕 |

> 「过滤非当前会话」按**地址段**分派（`<根>/session/<sid>` 剥前缀，恰一段 = 会话叶子、
> 两段且末段是消息目录 = 消息）——对象身份（`ChatMessage.id` / `VdfsNode.name`）在
> `data` 里，不在路径上（S27：`path` = 变更文件所在的**目录**）。

（不再有 `Status{idle}` 帧可等）。结局从同一次变更的
`node.attributes` 读：`outcome == aborted` ⇒ 报中止，`status == failed` / `error` 非空 ⇒ 该错误
作为本轮的返回值（退出码非 0）。

**为什么结束判据只能看会话节点**：一轮请求里模型会多次定格根 Turn 节点（每个工具轮次一次），
所以「Turn 到达终态」只说明这一轮*模型输出*结束，不说明整轮请求结束——转写帧本身无法回答
「还有没有下一轮」。

两个细节：

- **顺序由通道给，不再由序号给**（S27）：订阅连接是单一 FIFO，后端按发布序投递，
  不存在「重发叠字」的窗口；后端判明「可能漏了变更」时明示补送 resync 标记
  （[`Frame::Resync`]），CLI 无历史可重读，告警留痕。
- **节点变更也包含标题 / 元数据写入**，所以运行态提示只在**迁移**上报一次
  （`last_status`）；`drain_stale()` 在每轮 `ask` 前清空上一轮残留帧，避免旧帧被重复渲染。

## 3. 终端渲染器（`render.rs`）

| 通道 | 内容 |
| --- | --- |
| **stdout** | 模型正文 + 交互提示符（可直接管道给下游程序） |
| **stderr** | 进度、工具调用、错误、插件日志（可整体 `2>/dev/null` 静音） |

渲染器直接消费消息帧，不再有补丁合并：帧带 `delta` 时把裸增量累积进本地快照并**直接输出**
（流式热路径，O(delta)）；帧带 `content` 时用全量正文整条替换快照，与已输出内容做差分算出
「还没打印过的那一段」（改写发生时留痕而非重印整段）；`status = removed` 丢快照。

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

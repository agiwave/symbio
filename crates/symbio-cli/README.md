---
kind: guide
id: crates/symbio-cli/README
title: symbio-cli
status: active
created: 2026-09-11
---

# symbio-cli

与 [`../tauri`](../tauri) **并列**的第二种 Symbio 前端形态：纯 Rust 的命令行客户端，
支持**交互（REPL）**、**非交互（单次 / 管道）**与**心跳守护（`--heartbeat`）**三种形态。

## 它和 tauri 前端的唯一差别是传输层

```
 ┌────────────────────┐
 tauri/ ──────▶│ route_v2 (IPC) │──┐
 └────────────────────┘ │ ┌───────────────────────────┐
 ├─────▶│ home → worker(composite) │
 ┌────────────────────┐ │ │ ├─ session │
 cli/ ──────▶│ root.route(ctx) │──┘ │ ├─ model │
 │ （进程内直连） │ │ ├─ event_bus … │
 └────────────────────┘ └───────────────────────────┘
```

两者调用的是**同一套上下文键**（`PATH` / `PAYLOAD` / `SESSION_ID` / `WORKDIR` / …）
与**同一份协议**，因此本 CLI 的引入**没有改动 `symbio/` 的任何一行代码，也没有动协议**。

- 上行：`PluginSimpleRequest` 装上下文 → `root.route(ctx)`（`Arc<dyn Plugin>` 进程内直连）
- 下行：**两条连接，各归其域**——消息实时面订阅 **`session/stream`**（消息帧：
 `session_id` + 流内单调 `seq` + 一条 `ChatMessage`，按 `content` / `delta` / `status`
 直接落地到终端视图）；会话运行态
 订阅 **event_bus**（`kind = "vdfs"` 的变更帧，承载会话节点的 `status` / `outcome`）。
 两条都与「当前打开哪个会话」解耦 —— 订阅一次，之后切会话不必重建连接。

## 📚 文档

详细的架构、构建、用法文档在 [`crates/symbio-cli/docs/`](docs/README.md "crates/symbio-cli/docs/README")：

- [架构](docs/112-architecture.md "DE-112-architecture") — 进程内客户端、`event_bus` 通道选择、渲染器、模式选择、消息增量合并
- [构建](docs/003-building.md "GU-003-building") — 为什么不能裸 `cargo build`、C 工具链注入三要素、target 缓存修复
- [用法](docs/004-usage.md "GU-004-usage") — 四种模式示例、完整参数、REPL 命令、输出通道、已知边界

## 快速开始

```bash
# 以下都在 `cli/` 下执行；产物落在**仓库根**的 target/（`cli` 是根 workspace 的成员，
# 所有成员共用一个 target/）——构建脚本成功后会打印这一行，别照记忆抄路径
node scripts/build-cli.mjs

# 非交互：单条消息
./../target/debug/symbio-cli.exe --homedir ~/.symbio -m "用一句话解释什么是分形插件架构"

# 非交互：管道（脚本友好；stdout 只含模型文本）
echo "你好" | ./../target/debug/symbio-cli.exe --homedir ~/.symbio

# 交互：多轮 REPL
./../target/debug/symbio-cli.exe --homedir ~/.symbio

# 交互：stdin 不是终端时用 --repl 强制进入 REPL（也便于脚本化验证会话）
printf '第一轮\n/provider\n/exit\n' | ./../target/debug/symbio-cli.exe --repl

# 心跳守护：驻留并为本目录所有启用心跳的会话触发空闲心跳
./../target/debug/symbio-cli.exe --homedir ~/.symbio --heartbeat
```

产物：`../target/debug/symbio-cli.exe`（相对 `cli/`；仓库根看就是 `target/debug/…`）。

> **以构建脚本打印的绝对路径为准。** 上面是**规范位置**（根 workspace 单 target）。
> 若本机 `cli/.cargo/config.toml` 改过 `target-dir`（该文件是机器本地配置、不入库；
> 本仓的常见改法是重定向到 `../symbio/target` 以复用 C 依赖缓存），真实产物就不在这里——
> `node scripts/build-cli.mjs` 构建成功后会打印它实际解析到的绝对路径，照那一行敲。
> 本页**不写**具体机器的落点：那会让每个读者都去用一台机器上的布局。

## 参数

> **参数清单不在本页**：权威清单是 [RE-006 §5.2](../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表")（从 `crates/symbio-cli/src/args.rs` 的 `HELP` 提取，改代码即变）。本页曾抄一份**逐字相同**的表，与那份形成两处并存——同一句事实存两份，改一次要动两处。

模式判定顺序：`--heartbeat` → `-m`/位置参数 → `--repl` → stdin 非终端则走管道非交互 → 否则 REPL。

### REPL 内置命令

`/help`、`/new`、`/session <ID>`、`/provider <ID>`、`/workdir <路径>`、`/exit`。

## 输出通道约定

| 通道 | 内容 |
| --- | --- |
| **stdout** | 模型正文 + 交互提示符（可直接管道给下游程序） |
| **stderr** | 进度、工具调用、错误、插件日志（可整体 `2>/dev/null` 静音） |

为此本 CLI **刻意不调用** `symbio::init::initialize`：它会安装写 stdout 的 tracing
subscriber（默认过滤级别 `info,symbio=debug`），把插件日志混进正文。不初始化时，
插件日志宏退回 `eprintln!`，正好落在 stderr。

## 系统目录（homedir）

默认取 `<当前目录>/.symbio`；`--homedir` 参数在客户端启动时注入 **`SYMBIO_HOMEDIR` 环境变量**
（先于插件树构建）。`HomedirRegistry` 优先级链：**环境变量 > `~/.symbio_bootstrap` > 默认
`<当前目录>/.symbio`**（环境变量最高，因此 `--homedir` 总是生效）。

若该目录下没有任何模型条目（`plugins/model/<id>/provider.json`），CLI 会提示并退回内置
默认配置（通常表现为"没有可用 Provider"）。

## 构建（要点）

**统一走 `node scripts/build-cli.mjs`**（它负责把 C 工具链喂给 cc-rs，并保证环境字节级一致）。
关键环节与坑见 [构建](docs/003-building.md "GU-003-building")，三句话概括：

1. **C 编译器必须显式注入**（`CC`/`CXX`/`INCLUDE`/`LIB`，不碰 `PATH`）；`aws-lc-sys` 0.44 自带
 prebuilt NASM 对象，**既不需要 cmake 也不需要 nasm**。
2. **`INCLUDE`/`LIB` 分隔符必须统一成 `/`**，否则 Git Bash MSYS 改写路径导致 aws-lc-sys 反复全量重编。
3. **`build.target-dir` 必须绝对路径**，否则 cargo 不归一化、所有 C 依赖被判定过期重编挂死。

## 已知边界

- **不做工具审批交互**。默认 `--mode auto`：遇到需要审批的工具，后端返回友好错误后
 继续，不会挂起等待。人在环的审批卡片属于 tauri 前端的职责。
- **不做消息树持久渲染**。终端只做流式增量输出；历史回看用 `--session <ID>` 续接会话
 （上下文由后端从存储加载），而不是在本进程重放整棵树。
- 插件树是**全量**启动的，因此 `gateway` 插件会尝试监听 `127.0.0.1:9231`、
 `telegram` 插件会读它自己的配置。与正在运行的 Tauri 实例并存时，gateway 端口
 冲突只会在 stderr 记一条错误，不影响会话。

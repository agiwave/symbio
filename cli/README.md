# symbio-cli

与 [`../tauri`](../tauri) **并列**的第二种 Symbio 前端形态：纯 Rust 的命令行客户端，
支持**交互（REPL）**、**非交互（单次 / 管道）**与**心跳守护（`--heartbeat`）**三种形态。

## 它和 tauri 前端的唯一差别是传输层

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

两者调用的是**同一套上下文键**（`PATH` / `PAYLOAD` / `SESSION_ID` / `WORKDIR` / …）
与**同一份协议**，因此本 CLI 的引入**没有改动 `symbio/` 的任何一行代码，也没有动协议**。

- 上行：`PluginSimpleRequest` 装上下文 → `root.route(ctx)`（`Arc<dyn Plugin>` 进程内直连）
- 下行：**两条连接，各归其域**——消息实时面订阅 **`session/stream`**（消息帧：
  `session_id` + 流内单调 `seq` + 一条 `ChatMessage`，按 `content` / `delta` / `status`
  直接落地到终端视图）；会话运行态
  订阅 **event_bus**（`kind = "vdfs"` 的变更帧，承载会话节点的 `status` / `outcome`）。
  两条都与「当前打开哪个会话」解耦 —— 订阅一次，之后切会话不必重建连接。

## 📚 文档

详细的架构、构建、用法文档在 [`cli/docs/`](./docs/README.md)：

- [架构](./docs/architecture.md) — 进程内客户端、`event_bus` 通道选择、渲染器、模式选择、消息增量合并
- [构建](./docs/building.md) — 为什么不能裸 `cargo build`、C 工具链注入三要素、target 缓存修复
- [用法](./docs/usage.md) — 四种模式示例、完整参数、REPL 命令、输出通道、已知边界

## 快速开始

```bash
# 构建（脚本负责把 C 工具链喂给 cc-rs 并归一化路径分隔符）
node scripts/build-cli.mjs

# 非交互：单条消息
./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio" -m "用一句话解释什么是分形插件架构"

# 非交互：管道（脚本友好；stdout 只含模型文本）
echo "你好" | ./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio"

# 交互：多轮 REPL
./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio"

# 交互：stdin 不是终端时用 --repl 强制进入 REPL（也便于脚本化验证会话）
printf '第一轮\n/provider\n/exit\n' | ./../symbio/target/debug/symbio-cli.exe --repl

# 心跳守护：驻留并为本目录所有启用心跳的会话触发空闲心跳
./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio" --heartbeat
```

产物：`../symbio/target/debug/symbio-cli.exe`。

## 参数

| 参数 | 说明 |
| --- | --- |
| `-m, --message <文本>` | 发送单条消息后退出；也可用位置参数 `symbio-cli 你好` |
| `-s, --session <ID>` | 复用/续接既有会话（默认新建） |
| `--provider <ID>` | Model Provider，默认 `usrouter-glm5-3-flash`；传 `default` 回退到系统目录配置 |
| `--mode <MODE>` | `auto`（默认）\| `interactive` |
| `--workdir <路径>` | 会话工作目录（默认当前目录） |
| `--homedir <路径>` | **系统目录**（默认 `<当前目录>/.symbio`） |
| `--heartbeat` | 心跳守护模式：驻留并为本目录所有启用心跳的会话触发空闲心跳（与 `-m`/`--repl` 互斥） |
| `--agent <ID>` | 绑定 Agent（可选） |
| `-i, --repl` | 强制进入交互式 REPL，即使 stdin 不是终端 |
| `-q, --quiet` | 只输出模型文本 |
| `-v, --verbose` | 额外打印模型推理内容与诊断 |

模式判定顺序：`--heartbeat` → `-m`/位置参数 → `--repl` → stdin 非终端则走管道非交互 → 否则 REPL。

### REPL 内置命令

`/help`、`/new`、`/session <ID>`、`/provider <ID>`、`/workdir <路径>`、`/exit`。

## 输出通道约定

| 通道 | 内容 |
| --- | --- |
| **stdout** | 模型正文 + 交互提示符（可直接管道给下游程序） |
| **stderr** | 进度、工具调用、错误、插件日志（可整体 `2>/dev/null` 静音） |

为此本 CLI **刻意不调用** `symbio::init::initialize()`：它会安装写 stdout 的 tracing
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
关键环节与坑见 [cli/docs/building.md](./docs/building.md)，三句话概括：

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

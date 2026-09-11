# 用法

## 先构建

```bash
node scripts/build-cli.mjs
# 产物：../symbio/target/debug/symbio-cli.exe
```

详细构建约束（C 工具链注入、路径分隔符归一化、绝对 target-dir、toolchain 锁、缓存修复）见
[构建 (building.md)](./building.md)。

---

## 三种运行模式

### ① 非交互：单条消息（`-m` 或位置参数）

```bash
./../symbio/target/debug/symbio-cli.exe \
  --homedir "D:\Bing\symbio\.symbio" \
  -m "用一句话解释什么是分形插件架构"
```

### ② 非交互：管道（脚本友好）

stdin 不是终端时自动进入管道模式；stdout 只含模型文本，可直接喂给下游程序。

```bash
echo "你好" | ./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio"
# 或
printf '用一句话说明什么是插件系统。' | ./../symbio/target/debug/symbio-cli.exe
```

### ③ 交互：多轮 REPL

```bash
./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio"
```

终端不支持 TTY、或想用脚本喂多轮输入来自动化验证会话时，用 `--repl` 强制进入 REPL：

```bash
printf '第一轮\n/provider\n/exit\n' | ./../symbio/target/debug/symbio-cli.exe --repl
```

### ④ 心跳守护模式（`--heartbeat`）

```bash
./../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio" --heartbeat
```

驻留进程，为本 homedir 下**所有启用了心跳的会话**按各自配置触发空闲心跳（空闲 N 秒自动注入
心跳提示词驱动一轮工作），并把工作/空闲/错误状态渲染到 stderr。与 `-m`、`--repl` 互斥。
机制细节见 [docs/design/heartbeat-mechanism.md](../../docs/design/heartbeat-mechanism.md)。

模式判定顺序：`--heartbeat` → `-m`/位置参数 → `--repl` → stdin 非终端则走管道 → 否则 REPL。

---

## 完整参数

| 参数 | 说明 |
| --- | --- |
| `-m, --message <文本>` | 发送单条消息后退出；也可用位置参数 `symbio-cli 你好 世界` |
| `-s, --session <ID>` | 复用/续接既有会话（默认新建） |
| `--provider <ID>` | Model Provider，默认 `usrouter-glm5-3-flash`；传 `default` 回退到系统目录配置 |
| `--mode <MODE>` | `auto`（默认）\| `interactive` |
| `--workdir <路径>` | 会话工作目录（默认当前目录） |
| `--homedir <路径>` | **系统目录**（默认 `<当前目录>/.symbio`） |
| `--heartbeat` | 心跳守护模式：驻留并为本目录所有启用心跳的会话触发空闲心跳（与 `-m`/`--repl` 互斥） |
| `--agent <ID>` | 绑定 Agent（可选） |
| `-i, --repl` | 强制进入交互式 REPL，即使 stdin 不是终端 |
| `-q, --quiet` | 非交互模式下只输出模型文本 |
| `-v, --verbose` | 额外打印模型推理内容与诊断 |
| `-h, --help` / `-V, --version` | 帮助 / 版本 |

---

## REPL 内置命令

| 命令 | 作用 |
| --- | --- |
| `/help` | 命令列表 |
| `/new` | 新建会话（生成新会话 id） |
| `/session <ID>` | 切换/续接会话（无参数则显示当前） |
| `/provider <ID>` | 切换 Provider（`default` = 系统默认；无参数则显示当前） |
| `/workdir <路径>` | 切换工作目录（无参数则显示当前） |
| `/exit` / `/quit` / `/q` | 退出（Ctrl+D 亦可） |

---

## 输出通道约定

| 通道 | 内容 |
| --- | --- |
| **stdout** | 模型正文 + 交互提示符（可直接管道给下游程序） |
| **stderr** | 进度、工具调用、错误、插件日志（可整体 `2>/dev/null` 静音） |

为此本 CLI **刻意不调用** `symbio::init::initialize()`：它会安装写 stdout 的 tracing subscriber
（默认过滤 `info,symbio=debug`），把插件日志混进正文。不初始化时，插件日志宏退回 `eprintln!`，
正好落在 stderr。

---

## 系统目录（homedir）与 Provider 语义

- **homedir**：默认 `<当前目录>/.symbio`。`--homedir` 参数在 `SymbioClient::start()` 内注入
  **`SYMBIO_HOMEDIR` 环境变量**（先于插件树构建）；`HomedirRegistry` 的优先级链为
  **环境变量 > 用户主目录 bootstrap 文件 > 默认 `<当前目录>/.symbio`**。该目录下没有
  `config.yaml` 时，CLI 提示并退回内置默认配置（通常表现为「没有可用 Provider」）。
- **provider**：CLI 默认 `usrouter-glm5-3-flash`（开箱即连一个可用模型）；`--provider default` 显式
  保留「回退到系统目录 `config.yaml` 的 `default_provider_id`」能力。
- **mode**：默认 `auto`（无人值守）—— 遇到需审批的工具，后端返回友好错误后继续，不挂起等待。人在环的
  审批卡片属于 tauri 前端的职责。

---

## 已知边界

- **不做工具审批交互**：默认 `--mode auto`，遇需审批的工具不挂起。
- **不做消息树持久渲染**：终端只做流式增量输出；历史回看用 `--session <ID>` 续接会话（上下文由后端从
  存储加载），而非在本进程重放整棵树。
- **插件树全量启动**：`gateway` 会尝试监听 `127.0.0.1:9231`、`telegram` 会读自己的配置。与正在运行的
  Tauri 实例并存时，gateway 端口冲突只在 stderr 记一条错误，不影响会话。

---
kind: guide
id: GU-004-usage
title: 用法
status: active
created: 2026-09-11
---

# 用法

## 先构建

```bash
node scripts/build-cli.mjs
# 产物：../target/debug/symbio-cli.exe（仓库根的 target/，见 003-building）
```

详细构建约束（C 工具链注入、路径分隔符归一化、单一 target 目录、工具链唯一来源、缓存修复）见
[构建 (003-building.md)](003-building.md "GU-003-building")。

---

## 三种运行模式

### ① 非交互：单条消息（`-m` 或位置参数）

```bash
./../target/debug/symbio-cli.exe \
 --homedir ~/.symbio \
 -m "用一句话解释什么是分形插件架构"
```

### ② 非交互：管道（脚本友好）

stdin 不是终端时自动进入管道模式；stdout 只含模型文本，可直接喂给下游程序。

```bash
echo "你好" | ./../target/debug/symbio-cli.exe --homedir ~/.symbio
# 或
printf '用一句话说明什么是插件系统。' | ./../target/debug/symbio-cli.exe
```

### ③ 交互：多轮 REPL

```bash
./../target/debug/symbio-cli.exe --homedir ~/.symbio
```

终端不支持 TTY、或想用脚本喂多轮输入来自动化验证会话时，用 `--repl` 强制进入 REPL：

```bash
printf '第一轮\n/provider\n/exit\n' | ./../target/debug/symbio-cli.exe --repl
```

### ④ 心跳守护模式（`--heartbeat`）

```bash
./../target/debug/symbio-cli.exe --homedir ~/.symbio --heartbeat
```

驻留进程，为本 homedir 下**所有启用了心跳的会话**按各自配置触发空闲心跳（空闲 N 秒自动注入
心跳提示词驱动一轮工作），并把工作/空闲/错误状态渲染到 stderr。与 `-m`、`--repl` 互斥。
机制细节见 [会话心跳机制（heartbeat）](../../symbio/src/plugins/heartbeat/docs/113-heartbeat-mechanism.md "DE-113-heartbeat-mechanism")。

模式判定顺序：`--heartbeat` → `-m`/位置参数 → `--repl` → stdin 非终端则走管道 → 否则 REPL。

---

## 完整参数

> **参数清单不在本页**：权威清单是 [RE-006 §5.2](../../docs/004-ref/006-当前事实表.md#5-规模与宿主接缝 "RE-006-当前事实表")（从 `cli/src/args.rs` 的 `HELP` 提取，改代码即变）。本页曾抄一份**逐字相同**的表，与那份形成两处并存——同一句事实存两份，改一次要动两处。

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

为此本 CLI **刻意不调用** `symbio::init::initialize`：它会安装写 stdout 的 tracing subscriber
（默认过滤 `info,symbio=debug`），把插件日志混进正文。不初始化时，插件日志宏退回 `eprintln!`，
正好落在 stderr。

---

## 系统目录（homedir）与 Provider 语义

- **homedir**：默认 `<当前目录>/.symbio`。`--homedir` 参数在 `SymbioClient::start` 内注入
 **`SYMBIO_HOMEDIR` 环境变量**（先于插件树构建）；`HomedirRegistry` 的优先级链为
 **环境变量 > 用户主目录 bootstrap 文件 > 默认 `<当前目录>/.symbio`**。该目录下没有
 任何模型条目（`plugins/model/<id>/provider.json`）时，CLI 提示并退回内置默认配置
 （通常表现为「没有可用 Provider」）。
- **provider**：CLI 默认 `usrouter-glm5-3-flash`（开箱即连一个可用模型）；`--provider default` 显式
 保留「回退到系统目录 `plugins/model/PLUGIN.yml` 的 `default_provider_id`」能力。
- **mode**：默认 `auto`（无人值守）—— 遇到需审批的工具，后端返回友好错误后继续，不挂起等待。人在环的
 审批卡片属于 tauri 前端的职责。

---

## 已知边界

- **不做工具审批交互**：默认 `--mode auto`，遇需审批的工具不挂起。
- **不做消息树持久渲染**：终端只做流式增量输出；历史回看用 `--session <ID>` 续接会话（上下文由后端从
 存储加载），而非在本进程重放整棵树。
- **插件树全量启动**：`gateway` 会尝试监听 `127.0.0.1:9231`、`telegram` 会读自己的配置。与正在运行的
 Tauri 实例并存时，gateway 端口冲突只在 stderr 记一条错误，不影响会话。

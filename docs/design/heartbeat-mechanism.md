# 会话心跳机制（heartbeat）

> **文档类型：Design（机制设计）** — 会话空闲心跳的调度语义、设置工具与 CLI 守护模式。

## 概述

心跳 = 会话空闲超过阈值后，自动以「心跳提示词」注入一轮用户消息，驱动 agent 无人工介入地执行周期任务。由三部分组成：

1. **调度器**（`symbio/src/plugins/session/heartbeat.rs`）：每个运行中的 symbio 进程每 15s 扫描本 store 全部会话，对「已启用 + 空闲满阈值」的会话触发心跳。
2. **设置工具**（`symbio/src/plugins/session/heartbeat_tool.rs`）：agent 在对话中可调用 `heartbeat` 工具对本会话 set/get/cancel。
3. **CLI 守护模式**（`symbio-cli --heartbeat`）：无人值守驻留进程，专司心跳触发与状态渲染。

## 语义契约（不变式）

1. **防重入**：会话 `is_working` 期间绝不触发（调度循环先查工作状态再判空闲）；手动触发入口 `trigger_heartbeat_now` 同样受保护。
2. **空闲从活动结束起算**：空闲基线 = `max(内存锚点 last_activity, 磁盘 updated_at)`。正常回合内每条消息落盘都推进 `updated_at`，回合最后一次写盘 ≈ 活动结束，因此下一跳最早发生在**回合结束 + interval** 之后；内存锚点兜底覆盖「回合内零写盘」的异常路径，防止热循环。
3. **触发即重置**：触发时刻写入内存锚点，作为防热循环下限（即使回合零写盘也不会 15s 连发）。
4. **重启追赶**：内存锚点首次见到某会话时回退 `session.updated_at` —— 进程重启或新进程接入同一 store 时，按磁盘时间戳补触发已空闲的会话。这也是「每个进程都扫描全部会话」的设计根源：无需专职守护，任何存活进程都能兜底。
5. **抖动防雪崩**：每会话固定 jitter = `hash(session_id) % 30s`，避免多会话在同一 tick 齐触发。

## 调度细节

- tick 间隔 15s，单 tick 最多补触发 2 个会话。
- 实际触发节奏 = `interval + tick 粒度 + jitter`（如 30s 间隔实测 ≈ 36-50s 一跳）。
- 心跳消息：id `hb_<sid>_<毫秒>`，`meta.heartbeat = true`，以心跳提示词作为用户消息进入正常回合管线（工具调用、事件、持久化与普通消息完全一致）。
- 配置持久化于 `session.json` 的 `metadata.heartbeat`；工具写回会刷新 `updated_at`（写配置即视为活动）。

## heartbeat 工具 API

| action | 行为 |
| --- | --- |
| `set` | 启用/更新：`interval_seconds`（≥ 10）、`prompt`（非空）、`include_history`（布尔）；部分更新，未提及字段保留 |
| `get` | 读取当前配置（含 `enabled`） |
| `cancel` | 停用：`enabled=false`，配置保留，可随时 `set` 重启 |

## CLI 守护模式

```bash
symbio-cli --heartbeat [--homedir <路径>]
```

与 `-m`/`--repl` 互斥；进程驻留，订阅事件总线，把心跳会话的工作/空闲/错误状态渲染到 stderr。模式判定顺序：`--heartbeat` → `-m`/位置参数 → `--repl` → 管道 → REPL。

## homedir 传导

`--homedir` 在 `SymbioClient::start()` 内注入 `SYMBIO_HOMEDIR` 环境变量（先于插件树构建）；`HomedirRegistry` 优先级：**环境变量 > 用户主目录 bootstrap 文件 > 默认 `<当前目录>/.symbio`**。环境变量最高，因此 `--homedir` 总是生效，不会被 bootstrap 文件覆盖。

## 实测节奏

- **set → 落盘 → 触发**：30s 间隔实测 ≈45s 节奏（interval + 15s tick），`hb_` 消息与 `metadata.heartbeat` 落盘正确。
- **重启追赶**：一次性（one-shot）进程退出后，心跳由同 store 的其他存活进程接管；守护进程重启后立即补触发空闲会话。
- **跨进程 cancel**：一次性进程 cancel 后，另一进程的调度器下个 tick 即停（调度器每 tick 重读 store）。
- **触发间隔**：30s 间隔实测最小间隔 > interval（空闲严格从活动结束起算，见语义契约 2）。

## 边界与已知约束

- 多进程共享同一 store 时存在极小的同 tick 竞态窗口（A 进程刚触发、心跳消息尚未落盘时 B 进程同 tick 扫描）；守护模式部署建议 store 隔离。
- 日志中的 `~/.symbio/...` 为静态模板文本，不代表实际 homedir 解析结果。

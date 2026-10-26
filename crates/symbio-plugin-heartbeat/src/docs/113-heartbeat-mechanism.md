---
kind: design
id: DE-113-heartbeat-mechanism
title: 会话心跳机制（heartbeat）
status: active
created: 2026-09-11
impl: done
---

# 会话心跳机制（heartbeat）

> **文档类型：Design（机制设计）** — 会话空闲心跳的调度语义、设置工具与 CLI 守护模式。

## 概述

心跳 = 会话空闲超过阈值后，自动以「心跳提示词」注入一轮用户消息，驱动 agent 无人工介入地执行周期任务。由四部分组成（切出为独立插件的判据见 [D-077](../../README.md "crates/symbio-plugin-heartbeat/README")）：

1. **调度器**（`crates/symbio-plugin-heartbeat/src/scheduler.rs`）：每 15s 经地址面读一次会话清单（`vdfs/list(session)`），对「已启用 + 空闲满阈值」的会话向 `vdfs/write(session/<id>/inbox)` 写一条带触发声明的消息。
2. **设置工具**（`crates/symbio-plugin-heartbeat/src/tool.rs`）：agent 在对话中可调用 `heartbeat` 工具对本会话 set/get/cancel，读写走 `stat` / `write(session/<id>)`。
3. **选项行**（`crates/symbio-plugin-heartbeat/src/option.rs`）：会话选项栏那一格的声明，保存走前端已在用的 `option` 绑定写通道。
4. **CLI 守护模式**（`symbio-cli --heartbeat`）：无人值守驻留进程，专司渲染心跳会话的工作/空闲状态——**触发本身不在 CLI 里**，它就是这个插件的那条常驻循环。

数据形状（`metadata.heartbeat` 那四格）住 [`setting.rs`](../setting.rs)，读写它的两半共用这一个定义；会话存储侧只按键浅合并，不认识其内容。

## 语义契约（不变式）

1. **防重入**：会话 `status = working` 期间绝不触发（调度循环先查工作状态再判空闲）；触发事实落格还会让开「抢占待结算」的窗口（见契约 6）。
2. **空闲从活动结束起算**：空闲基线 = `max(锚点 last_activity, 磁盘 updated_at)`。正常回合内每条消息落盘都推进 `updated_at`，回合最后一次写盘 ≈ 活动结束，因此下一跳最早发生在**回合结束 + interval** 之后；锚点兜底覆盖「回合内零写盘」的异常路径，防止热循环。锚点现在住本插件（`HeartbeatPlugin::anchors`），宿主不再替它记活动。
3. **触发即重置**：**成功**触发的那一刻写入锚点，作为防热循环下限（即使回合零写盘也不会 15s 连发）。失败不写——失败都是暂时的（会话正被写 / 无事实源），压着原锚点等下个 tick 重试，按整段 interval 推迟等于把一次本可成功的触发白白睡掉。
4. **重启追赶**：锚点首次见到某会话时回退 `session.updated_at` —— 进程重启或新进程接入同一 store 时，按磁盘时间戳补触发已空闲的会话。这也是「每个进程都扫描全部会话」的设计根源：无需专职守护，任何存活进程都能兜底。`updated_at` 缺失则**不触发**（「不知道它活动过」不能读成「它一直空闲」）。
5. **抖动防雪崩**：每会话固定 jitter = `hash(session_id) % 30s`（确定性，不是随机——同一会话每次求得的抖动要稳定），避免多会话在同一 tick 齐触发。
6. **先落事实，再开轮**：调度器只答「什么时候该开一轮」。这一次开轮**是系统自己发起的**这件事，先落成事实源里的四格（触发 / 自检 / 欲 / 闸门放行才长任务），落成了才入队；运行参数（`mode` / `include_history`）由服务端按会话自己的数据推导。两者都住在会话那一侧：`plugins/session/transcript/autonomous.rs`（`request_autonomous_round`），入口是收件箱写臂识别 `meta.trigger`（`plugins/session/plugin/nodes.rs::trigger_declaration`），归属与逐口裁决见 [D-076](../../README.md "crates/symbio-plugin-heartbeat/README")。**落格失败 ⇒ `Conflict` 且连队列都不碰**：一条永远跑不起来的队首会把后面的用户消息一起堵死。

## 调度细节

- tick 间隔 15s，单 tick 最多补触发 2 个会话。
- 实际触发节奏 = `interval + tick 粒度 + jitter`（如 30s 间隔实测 ≈ 36-50s 一跳）。
- 心跳消息：id `hb_<sid>_<毫秒>`，`meta.heartbeat = true`（给前端）+ `meta.trigger`（给写臂的触发声明），以心跳提示词作为用户消息进入正常回合管线（工具调用、事件、持久化与普通消息完全一致）。
- 配置持久化于 `session.json` 的 `metadata.heartbeat`；工具写回会刷新 `updated_at`（写配置即视为活动）。
- 每 tick 的成本 = 一次 `list(session)`（含 schema 广播）+ 一次容器子树广播，量级与前端点一次会话列表相同；**不为省它缓存清单**（那会造出第二份 `metadata.heartbeat` 副本），裁决与省法都记在 [D-077](../../README.md "crates/symbio-plugin-heartbeat/README") 后果段。

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

`--homedir` 在 `SymbioClient::start` 内注入 `SYMBIO_HOMEDIR` 环境变量（先于插件树构建）；`HomedirRegistry` 优先级：**环境变量 > 用户主目录 bootstrap 文件 > 默认 `<当前目录>/.symbio`**。环境变量最高，因此 `--homedir` 总是生效，不会被 bootstrap 文件覆盖。

## 实测节奏

- **set → 落盘 → 触发**：30s 间隔实测 ≈45s 节奏（interval + 15s tick），`hb_` 消息与 `metadata.heartbeat` 落盘正确。
- **重启追赶**：一次性（one-shot）进程退出后，心跳由同 store 的其他存活进程接管；守护进程重启后立即补触发空闲会话。
- **跨进程 cancel**：一次性进程 cancel 后，另一进程的调度器下个 tick 即停（调度器每 tick 重读 store）。
- **触发间隔**：30s 间隔实测最小间隔 > interval（空闲严格从活动结束起算，见语义契约 2）。

## 边界与已知约束

- 多进程共享同一 store 时存在极小的同 tick 竞态窗口（A 进程刚触发、心跳消息尚未落盘时 B 进程同 tick 扫描）；守护模式部署建议 store 隔离。
- 日志中的 `~/.symbio/...` 为静态模板文本，不代表实际 homedir 解析结果。
- **可停用**：本插件在必需清单里（`ASSEMBLY_SUB_AGENT_PLUGINS`），因此**可停用不可删除**。停用的可执行形态是注册表的 `set_enabled(false)` ⇒ `stop(Disabled)`：常驻循环在停机窗口内退出（超时要 `abort`），能力表里的 `heartbeat` 两行（工具 + 选项）随下一轮收集消失，会话那一侧一行不少。判据两处各钉一半：本片 `plugin.test.rs`，容器侧 `plugins/composite/composite.test.rs`（经工厂 id + 真实 `mount_all`，不 import 兄弟模块）。
- **停用后已存的配置不动**：`metadata.heartbeat` 是会话的数据，不是本插件的数据——它的 owner 是那块 metadata 的语义，谁在场都能读；卸载会话才带走它。

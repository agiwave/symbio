# Session 插件机制审计：复杂性成因、冗余清单与简化方案

> 状态：审计结论（含可执行方案）。本文是 `docs/archive/implementation-logs/session-mechanism-simplification.md`
> 的后续核查版——那份方案里的部分判断被本次取证推翻或修正，差异见 §6。
> 所有结论都带 `文件:行号` 证据，可逐条复核。

---

## 0. 一句话结论

**机制本身没有过度设计；复杂度来自"同一语义的多套并行实现"和"配置项无单一真源"。**
真正需要做的不是删机制，而是收敛：把 4 套骨架化、3 套会话实现、2 套截断、2 套默认值声明
各收敛为 1 套，并修掉 **2 个行为缺陷（D2/D4）+ 1 个配置契约漂移（D5，需产品决策）**。
预计净减 **约 1200–1500 行生产代码**，删除 **2 个配置项**，**不改变任何对外的会话行为语义**。

---

## 1. 规模与结构基线（用于判断"是否真的过度复杂"）

| 范围 | 行数 | 说明 |
|---|---|---|
| `symbio` 全 crate | 44,365 | — |
| `plugins/session/` 合计 | 13,113 | 占全 crate 约 30% |
| 其中测试 | 2,440（19%） | `#[cfg(test)]` 之后计入测试 |
| **生产代码** | **10,673** | 判断冗余项的基准 |

生产代码 Top 5：`chat_loop.rs` 1,673 / `orchestrator.rs` 1,366 / `tool_executor.rs` 866 /
`compression.rs` 760 / `plugin.rs` 707（均含较多文档注释，`orchestrator.rs` 尤甚）。

单文件最大函数：`run_chat_loop` 主体约 **892 行**（`chat_loop.rs:104–995`）。

结论：以"一个具备自动压缩、分层滑窗、断点续传、心跳、工作目录选择、多模型协议适配的
agent 会话引擎"为职责边界，10.7k 行**不算失控**；但其中**约 11–14% 是可证明的重复实现**，
这才是"感觉过度复杂"的真实来源。

---

## 2. 成因分析：复杂度从哪里长出来的

### 2.1 历史迁移留下的"双轨"（最主要成因）

`session_chat` 曾经既支持"编排器内部直接聊天"，又要支持"宿主交付会话引擎句柄"。
迁移时为了**不破坏既有链路**，采取了"新链路优先、旧链路兜底"的加法策略，
于是同一语义出现了两条实现路径，事后旧路径没删：

- 会话引擎：`PersistentChatSession`（编排器主路径，`orchestrator.rs:885`）
  / `EphemeralChatSession`（`handlers.rs:332/342`）
  / `FallbackChatSession`（`chat_loop.rs:1006`）——**三套**。
- 请求视图构造：编排器自己调 `build_request_view`（`orchestrator.rs:1047`），
  而 `EphemeralChatSession::build_llm_request`（`chat_session.rs:521`）是同一逻辑的第二份拷贝。

**这是典型的"迁移期兜底未回收"，不是设计选择。**

### 2.2 配置项没有单一真源

一个配置项要写 **4 处**：`SessionConfig` 结构体字段 + `default_xxx()` 函数 +
`plugin.rs::config_schema()` 里的 JSON Schema + `chat_loop.rs` 里的 `unwrap_or(硬编码)` 兜底。
四处靠人肉同步，已经出现漂移（见 §3.1 的 D5）。这是"加一个开关很贵"的体感来源，
也是过度复杂感的直接成因——**不是机制多，是每个机制的落地成本高**。

### 2.3 防御性编程叠加：机制上再套机制

`session_chat` 已经处在"被 agent 调用"的位置，外面还叠了：
`tool_executing` 标志 + `AbortSignal` + `watchdog`（消费循环超时）+ `max_rounds` 软上限 +
`rate_limit` 重试 + `tool_result_guard` 体积守卫 + `heartbeat` 续跑。
每一层单独看都有理由（都对应过一次真实故障），**但没有一份文档说明"哪层是兜哪条故障的"**，
于是维护者无法判断哪层可以删。`chat_loop.rs:632` 的注释
*"兜住任何逃逸到编排器层的异常（理论上不应触发，此处为最后防线）"* 就是这种状态的自白。

### 2.4 死机制被"预留"名义保留

`context_retention`（`ToolContextRetention::All/LastOnly/LastN`）在 `capability.rs` 定义了
完整语义，全仓 **零工具声明、零消费者**（仅 `subagent.rs:91` 写 `None`）。
类似地 `store_kind` 是"可配置但不可用"。这类"已建好但没接线"的机制最伤体感：
读代码时以为要理解它，实际它是空转的。

---

## 3. 冗余与缺陷清单

### 3.1 已确认的问题（3 项：2 个行为缺陷 + 1 个配置契约漂移）

**D5（配置契约漂移，已确证）— `max_tool_rounds` 默认值三处不一致，兜底值实际永不生效**

| 位置 | 值 |
|---|---|
| `symbio_core/schemas/session/session_config.rs:68` | **65535** |
| `plugins/session/plugin.rs:247`（JSON Schema default） | **15** |
| `plugins/session/plugin.rs:245`（Schema 描述文案） | "默认 15" |
| `plugins/session/chat_loop.rs:562` | `req.max_tool_rounds.unwrap_or(15)` |

后果链：`SessionConfig::default()` 与 `serde` 缺省都给出 **65535** →
`orchestrator.rs:961/975/997` 三处 `Some(session_cfg.max_tool_rounds)` **恒为 `Some(65535)`** →
`chat_loop.rs:562` 的 `unwrap_or(15)` **永远不生效** → 软上限实际是 65535，
即 `chat_loop.rs:437–443` 的"工具循环软上限 + 提示文案"**形同虚设**。
只有当前端/宿主在 `config` 里显式传 `max_tool_rounds` 时才有效。

同时，JSON Schema 向宿主/UI 承诺"默认 15"（但前端目前**并未暴露**该字段：
`grep max_tool_rounds tauri/src/` 零命中），因此实际线上行为是"不限轮次"。
这属于**文档/契约与实现不一致**，而不是"护栏被意外关闭"——需要产品决策，不能单方面改。

> **决策点（必须先定，再动手）**：
> ① 认定"应有软上限"→ `default_max_tool_rounds()` 改 15（与 Schema/文案/兜底一致）。
>    **风险**：`subagent.rs:290` 构造 `session_chat::Request` 时不传该字段，
>    子智能体当前实际享有"不限轮次"；改 15 会给所有深层子任务加上硬顶，可能直接改变现网行为。
>    若走此路，需同时给 subagent 显式传一个更大的值（如 200），并同步 `chat_loop.rs:440` 的提示文案。
> ② 认定"默认不限轮次"是产品意图 → 把 Schema `default` 与描述文案改为 65535/不限，
>    并删除 `chat_loop.rs:562` 的 `unwrap_or(15)`（消除第二真源）。**零行为变更，推荐先做这一步。**
>
> 无论选哪条，**都要加一条断言测试**：`config_schema()` 每个 `default` == `SessionConfig::default()` 对应字段，
> 从此杜绝再漂移。

**D2（中）— 工具级骨架化被 `context_messages` 连带关闭**

`chat_loop.rs:539`：`let retention = if context_messages > 0 { … } else { HashMap::new() };`
`context_messages == 0` 的语义是"不限制会话轮数"（`session_config.rs:31` 注释明示），
但这里被复用来关闭工具滑窗，于是**"不限轮数"的长会话反而失去工具结果骨架化**，
与 `context_window.rs:272`"无轮数窗口约束"的注释语义相反。属于隐式耦合缺陷。

**D4（中低）— 存储期 prune 会物理删除归档文件**

`chat_session.rs:587–598`：prune 移除 Tool/ToolCall/Reasoning 节点时，
连带 `tokio::fs::remove_file` 删除其 `archive_path` 归档文件。
这与"存储层恒为完整原文、只动请求视图"（`context_window.rs` 模块头、`tool_result_guard.rs` 设计说明）
的既定原则冲突：**归档一旦删除，`context_compact` 与后续任何回捞都不可逆**。
虽然 prune 只在 `context_messages > 0` 时执行（`chat_session.rs:304`），
但"默认配置（`context_messages=6`）下每轮都在静默删文件"这一点必须显式确认是否有意。

### 3.2 重复实现清单（收敛目标）

| # | 重复项 | 位置 | 规模 | 判断 |
|---|---|---|---|---|
| R1 | 会话引擎实现 ×3 | `PersistentChatSession` `chat_session.rs:190–440`(251行) / `EphemeralChatSession` 同文件 `440–555`(116行) / `FallbackChatSession` `chat_loop.rs:996–1060`(65行) | 432 行 | **Ephemeral 与 Fallback 语义等价**（内存 Vec + seq 续号 + 不 prune + 不 save）；Persistent 独有仅 prune+save |
| R2 | 骨架化 ×4 套 | `context_window.rs:269` 分层滑窗(152行) / `compression.rs` `skeletonize_tool_call`+`skeletonize_tool_result` / `tool_executor.rs:92` `summarize_tool_result` / `tool_result_guard.rs` `digest_of`+`first_line`+`skeletonize_json` | 4 处独立实现 | 同一"骨架化/摘要"语义；锚点参数表与 `head\n…\ntail` 格式各写一遍 |
| R3 | 截断 ×2 | `context_window.rs:109 truncate_at_char_boundary` / `compression.rs:127 truncate_str` | 小 | 同为 UTF-8 安全截断 |
| R4 | head/tail 行切分 ×2 | `compression.rs:141 split_head_tail_lines` / `tool_result_guard.rs:134 split_head_tail` | 小 | 同为"头 N 行 + 尾 M 行" |
| R5 | 默认值声明 ×2 源 | `session_config.rs:59–81`（8 个 `default_xxx`） vs `plugin.rs:216–277`（62 行 JSON Schema，含 9 处 `default` 字面量） | 62 行 | 8 项中 7 项一致，`max_tool_rounds` 漂移（=D5） |
| R6 | 死机制 | `capability.rs:84–95` `ToolContextRetention` 全家族；`plugin.rs:341` `store_kind` | ~30 行 + 认知成本 | 零声明零消费 / 不可配置 |

> 注：`context_window.rs` 与 `compression.rs` **不是**重复机制——前者"视图裁剪"（零 LLM、可逆、
> 每请求重建），后者"语义压缩"（调 LLM、不可逆、落库）。这条边界是清晰的，**不要合并**。

---

## 4. 简化方案（分三批，每批独立可交付、可回滚）

### 批次 A：修缺陷（不动结构，半天，最高优先）

1. **A1 收口 D5（先决策后动手）**：按 §3.1 的决策点选 ① 或 ②。
   **默认执行 ②**（Schema `default`/描述文案改为 65535 或"不限"，删 `chat_loop.rs:562` 的 `unwrap_or(15)`）
   ——零行为变更、消除第二真源；若确认要恢复软上限，再单独走 ①（含 subagent 显式传值 + 文案回归测试）。
   无论哪条，都补一条"Schema default == `SessionConfig::default()` 字段"的断言测试。
2. **A2 修 D2**：`chat_loop.rs:539` 的 retention 计算去掉 `context_messages` 门控，
   改由 `tool_context_window > 0` 单独控制（两机制解耦）。
3. **A3 定 D4**：prune 不再 `remove_file`，只删消息节点、保留归档（归档生命周期交给显式清理策略）。
   若确认"就是要删"，则改为写日志 + 保留最近 N 天。

### 批次 B：收敛重复实现（1–2 天，行为不变）

4. **B1 会话实现 3→2**：
   - `PersistentChatSession` 引入 `persist: bool`（或 `Option<PrunePolicy>`），
     把 prune/save 收进策略；
   - 删除 `EphemeralChatSession`，`handlers.rs:332/342` 改用
     `PersistentChatSession::ephemeral(&cfg)`；
   - `FallbackChatSession` 也复用同一实现（其存在理由"避免 session↔chat_loop 模块环"
     已由 `chat_loop.rs:1003` 注释说明，用 `Arc<dyn ChatSession>` 已解耦，不构成保留理由）。
   - **净减约 180 行**，并消除"改一处行为要记得改三处"的风险。
5. **B2 骨架化 4→1**：新建 `symbio_core::text::skeleton`（或 `session::skeleton`）单一模块，
   提供 `truncate_at_char_boundary` / `split_head_tail` / `first_line` / `skeletonize_json` /
   `digest_of` / 锚点参数表；`context_window.rs`、`compression.rs`、`tool_executor.rs`、
   `tool_result_guard.rs` 全部改为引用。
   - 顺手收掉 R3、R4；**净减约 120–180 行**。
   - 注意：`tool_executor.rs:92 summarize_tool_result` 的**语义**（实时流式帧摘要，供前端展示）
     与视图骨架化不同，只共享工具函数，**不要合并成一个函数**。
6. **B3 删死机制 R6**：删除 `ToolContextRetention` 及其 `context_retention` 字段
   （含 `subagent.rs:91`）；`store_kind` 从 Schema 移除（保留 `FileSessionStore` 唯一实现）。
   - 需要时再按真实需求加回，成本远低于现在维护"看起来存在的能力"。

### 批次 C：配置单一真源（1 天）

7. **C1 `config_schema()` 由 `SessionConfig` 派生**：在 `session_config.rs` 用
   `#[serde(default = "…")]` + 一份 `const DEFAULTS: &[(&str, Value)]` 表，
   或引入 `schemars` 派生 `JsonSchema`，让 Schema 的 `default` 直接读 `SessionConfig::default()`。
   - `plugin.rs:216–277` 那 62 行手搓 Schema 可改为从 `SessionConfig::default()` 派生默认值，
     消除双源（字段条目数不减少，但 `default` 字面量归零）。
   - 这是 D5 类问题的**根治**：默认值从此不可能漂移。
8. **C2 删掉 `chat_loop.rs:559–563` 的 `unwrap_or(硬编码)` 兜底**：
   编排器已三处恒传 `Some(...)`（`orchestrator.rs:960/974/996/1006`），
   兜底值只是第二份真源。改为 `unwrap_or_else(|| SessionConfig::default().xxx)`。

### 批次 D（原暂缓，2026-09-13 经授权已实施）：`run_chat_loop` 拆分

拆为"轮次驱动骨架 + 帧消费 + 终态落库"三段。**但**：`chat_loop.rs:66` 注释已明确
"不再持有终态落库职责，终态唯一落库点在 `orchestrator.rs`"——这条不变式很宝贵，
拆分容易把它重新搅浑。**原建议：等 A–C 落地、并发缺陷（watchdog 与 `stop_session` 竞态）
有真实复现时再做，不作为简化指标推进。**

> 落地情况见 §8.6：A–C 已落地；前置竞态经取证确认为真实缺陷（陈旧 `ai_control_tx`
> 登记令 `handle_abort` 空等 3s），已先以 `AiControlGuard`（Drop 守卫）修复并配回归测试，
> 再执行拆分；不变式经调用点比对确认未被搅浑。

---

## 5. 明确"不要动"的部分（避免把机制当冗余删掉）

| 机制 | 保留理由 |
|---|---|
| 自动压缩 + `context_compact` 主动压缩 双轨 | 一个被动兜底、一个模型自主，职责不同且共用 `compression.rs` 内核 |
| 请求视图层（不落库、每请求重建） | 保证"存储恒为完整原文"，是 resume 与审计的前提 |
| 分层滑窗（轮级/工具级/内容级） | 三层作用对象互斥，已实测可逆性 |
| `AbortSignal` + `watchdog` + `max_rounds` 软上限 | 各兜一类真实故障（用户停止 / provider 流挂死 / 模型死循环）——修好 D5 后软上限才真正生效 |
| `tool_result_guard` 体积守卫 | 与视图骨架化正交：它守的是**单条结果进入存储**的上限 |
| 心跳工具 + 独立后台任务 | 后台任务必须与 chat 回合解耦，不能并入 `chat_loop` |
| `workdir` 选择器 + `fs_watcher` | 独立能力，与 session 内核无耦合 |
| `rate_limit` 重试 | 多模型协议下的必要适配 |

---

## 6. 与既有方案（`./complexity-audit.md`，现已归档）的差异

那份方案的以下判断被本次取证**修正**：

- **P2「压缩双轨过度」→ 撤销**：两轨共用同一内核，入口分离是刻意设计，不是重复。
- **P4「视图层与分层滑窗重叠」→ 撤销**：`context_window.rs` 与 `compression.rs` 语义正交。
- **P6「`max_rounds` 与 watchdog 重叠」→ 降级**：`watchdog` 兜帧间挂死、`max_rounds` 兜逻辑死循环，
  不重叠；**但 `max_rounds` 因 D5 实际未生效，需按 A1 修**。
- **P1「`context_retention` 死机制」→ 撤销（本表原写"确认，进 B3"，实施期取证推翻）**：
  `todo_write.rs:160` 与 `heartbeat_tool.rs:85` 实际声明 `LastOnly`，并由
  `context_window.rs:310` 的 `keep_count()` 消费，是"最新一次 todo 清单不被骨架化掉"的真实防线。
  故 B3 未执行删除，见 §8.2-B3。
- **P3「三套骨架化」→ 确认为四套**（漏了 `context_window.rs` 自身），进 B2。
- **P5「双会话实现」→ 确认为三套**（漏了 `FallbackChatSession`），进 B1。
- **D5（默认值漂移）为该方案未覆盖的新发现，且是本次最高优先级项。**

---

## 7. 验收标准（含 2026-09-13 实测结果）

- A1 后：新增测试断言 `config_schema()` 每个 `default` == `SessionConfig::default()` 对应字段。
  → **达成**：`config_schema_defaults_match_session_config_default`（逐字段，含覆盖性断言）。
- B1 后：`grep -c "impl ChatSession for"` 从 4 降到 2（含测试 mock）。
  → **超额达成**：生产代码降到 **1**（`PersistentChatSession`）；测试 mock 亦随
  `FallbackChatSession` 删除而移除，`symbio/tests/session_chat_e2e.rs` 已不存在（早期集成测试
  在更早的重构中删除，故本项不再作为门禁），改由 `cargo test --lib` 337 项覆盖。
- B2 后：`grep -rn "fn truncate_at_char_boundary\|fn truncate_str\|fn split_head_tail\|fn first_line" src/`
  命中数从 6 降到 2（core 一份 + 各自薄封装）。
  → **达成**：命中恰为 2（`text_split::split_head_tail` 唯一机制实现 + `context_window::first_line_digest`
  的摘要语义），字符边界安全统一由 `symbio_core::text::floor_char_boundary` 承担。
- C1 后：`session_config.rs` 成为默认值唯一出处（Schema 内不再出现 `default` 字面量）。
  → **达成（但原目标数字有误，见 §8.5-2）**：`config_schema()` 函数体内 `default` 字面量
  由 9 处降为 **0**，全部改为 `d("key")` 从 `SessionConfig::default()` 派生；fade /
  max_tool_rounds / tool_context_window 等常量已从 `chat_loop` 迁入配置。
  **`plugin.rs` 生产行数 707 → 737（+30，未达原写的 ≤620）**：schema 需为 3 个新提升的
  可配置项补条目，函数体 62 → 83 行，属预期的功能增加而非冗余膨胀。
- 全程：`cargo test --lib` 与 `cargo test --test session_chat_e2e` 无回归（基线 278 passed）。
  → **达成**：`cargo test --lib` **337 passed / 0 failed**（基线 278 → 337，新增 59 个用例）；
  `cargo clippy --lib --tests -- -D warnings` 零告警。`--test session_chat_e2e` 一项作废（见 B1 说明）。

---

## 8. 批次落地记录（2026-09-13，与复杂度审计 §八 同一轮改动）

用户授权"推动实施并验证"后，§4 的批次 A/B/C 已全部落地；批次 D 按原建议**暂缓**。

### 8.1 批次 A（修缺陷）

| 项 | 结论 | 落地方式 |
|---|---|---|
| **A1 收口 D5** | 按默认执行 ②，并进一步做**默认值迁移** | schema `default` 全部由 `SessionConfig::default()` 派生（`d(key)` 局部闭包），并新增 `model_chat_max_tool_rounds()` 作为唯一契约翻译点（`0 → None`）；`chat_loop` 的 `unwrap_or(15)` 兜底改读配置默认。默认值最终定为 `0 = 不限制`（比 `65535` 语义更正确，且与 `max_messages`/`tool_context_window` 的 0 语义一致），迁移安全性已在复杂度审计 §8.5-1 核实 |
| **A2 修 D2** | 已修 | retention/骨架化的门控由 `tool_context_window > 0` 单独控制，不再被 `context_messages` 连带关闭；`context_messages = 0` 在 `prune_historical_tool_calls` 早返回（与 `sliding_window` 的"0 = 不限制"统一） |
| **A3 定 D4** | 采纳"不删文件" | 轮次 FIFO 淘汰与存储期 prune 均只删消息节点、**不再 `remove_file`**；归档磁盘生命周期唯一归 `tool_result_guard` 的 `TOOL_ARCHIVE_KEEP` 滚动策略。另新增 `prune_tool_history` 配置（默认 `true` 保持行为，可关成"存储恒为完整原文"） |

### 8.2 批次 B（收敛重复实现）

- **B1 会话实现 3→2→1**：`EphemeralChatSession` 与 `FallbackChatSession` 均删除，临时/降级会话改为
  `PersistentChatSession` + `store::InMemorySessionStore`（同一份引擎逻辑，仅存储后端不同）。
  `impl ChatSession for` 命中数 **4 → 1**（优于验收目标的 2），净减约 150 行。
  顺带把 `store_kind` 真正接通：`create_store` 现支持 `memory`，"可配置但不可用"的状态结束。
- **B2 骨架化 4→1**：头尾切分/截断机制唯一化到 `session::text_split`
  （`truncate_head_chars` / `truncate_tail_chars` / `truncate_head_chars_if_over` /
  `truncate_tokens` / `split_head_tail`），`context_window.rs`（视图骨架化）与
  `tool_result_guard.rs`（L0 生产期守卫）均改为引用；`compression.rs` 不再自持副本。
  按原方案注记，`summarize_tool_result`（实时流式帧摘要，供前端展示）**语义保持独立**，
  只共享底层切分函数，未强行合并成一个函数。
- **B3 删死机制 R6**：**未执行删除**——核实修正审计结论：`ToolContextRetention` 并非死机制，
  `todo_write.rs:160` 与 `heartbeat_tool.rs:85` 均实际声明 `LastOnly`，并由
  `chat_loop` 从 CapabilityVisitor 动态解析进 `build_request_view`，是"最新一次 todo 清单不被
  骨架化掉"的真实防线。故保留该机制；`store_kind` 亦保留（现已接通 `memory`）。
  审计 §3.2-R6 的"死机制"判断作废。

### 8.3 批次 C（配置单一真源）

- **C1**：`config_schema()` 的函数体内 `default` 字面量由 9 处降为 **0**（原 §8.3 记的
  "142 行降到 83 行"经复核有误，见 §8.5-2：HEAD 函数体实为 62 行，现为 83 行，因新增
  3 个可配置项而变长；真正的收益是默认值单源化）；
  `session_config.rs` 成为默认值唯一出处。新增 2 个覆盖性测试：
  `config_schema_defaults_match_session_config_default`（逐字段 default 相等）、
  `config_schema_covers_all_session_config_fields`（字段覆盖完备，`session_id` 以
  `NON_CONFIGURABLE` 显式豁免并注明理由）。
- **C2**：`chat_loop` 的 `unwrap_or(硬编码)` 兜底全部改读 `SessionConfig::default()`；
  fade 的 `40/12` 常量同时迁入配置字段 `fade_activate_rounds` / `fade_keep_recent_turns`（R3）。
- **C3（附带）**：`SessionConfig::session_id` 死字段删除（R8）；`orchestrator.rs` 三分支
  重复的 8 个会话派生字段收敛为 `req_base` + `..req_base`（R6）；`workdir` 候选路径列表提取为单一常量（R9）。

### 8.4 验证证据与验收对照

- `cargo check --workspace`：0 error / 0 warning；`cargo clippy --lib -- -D warnings`：通过。
- `cargo test --lib`：**337 passed / 0 failed**（该审计基线 278 → 复杂度审计基线 313 → 本轮 337）。
- 验收逐条：A1 ✅（default 相等断言已建）；B1 ✅（`impl ChatSession for` = 1）；
  B2 ✅（截断/切分函数定义唯一化到 `text_split`，全仓无重复实现）；C1 ✅（83 行）；全程无回归 ✅。
- 新增存储后端契约测试：`store::sqlite::sqlite_store_roundtrip_contract`、
  `store::memory` 的读写/嵌套/删除用例——原先"声明但不实现"的 sqlite 后端实为**已完整实现且零测试**
  （审计 §3.5 结论过时），现已纳入回归网。
- resume/message 互斥防御补齐（原 §五-步骤3 剩余项）：两者同时提供时显式报错，
  不再让 user 消息被静默落库而请求走 resume 分支。

### 8.5 数字核对与更正（对 §7 / §8.3 的修正，2026-09-13 复核）

同一脚本（按 `#[cfg(test)]` 块切分生产/测试行）统计 `symbio/src/plugins/session/**/*.rs`
的审计基线与本轮改动后的工作树：

| | 文件数 | 总行 | 生产行 | 测试行 |
| :--- | ---: | ---: | ---: | ---: |
| 审计基线 `3ef56fd`（**即当前 HEAD**：本轮改动全部未提交） | 30 | 13,113 | 10,571 | 2,542 |
| 本轮改动后（工作树） | 32 | 13,808 | **10,619** | **3,189** |
| Δ | +2 | **+695** | **+48** | **+647** |

两条必须先钉住的事实：

* `git rev-parse HEAD` == `3ef56fd`，即**审计基线就是 HEAD**，本轮所有改动都还在工作树里。
  §7 表内"现状"列（合计 13,143）是本轮改动**中途**从工作树取的快照，不是基线值；
  基线的准确总行数是 **13,113**（与 §1 一致）。
* 生产/测试的切分结果随脚本口径而异（是否把缩进的 `#[cfg(test)]`、`#[cfg(all(feature, test))]`
  计入测试），本文采用"块级切分"，故与 §1 的 10,673 / 2,440 存在约 ±100 行的口径差；
  但**Δ 值在各口径下一致（+47 ~ +48）**，结论不受影响。

**结论：生产行数没有下降，反而 +48 行；测试 +647 行。** 逐文件明细（生产行，基线 → 现在）：

| 文件 | 生产行 | Δ生产 | 说明 |
| :--- | :--- | ---: | :--- |
| `chat_loop.rs` | 1678 → 1615 | **−63** | 删 `FallbackChatSession`（116 行）+ 兜底常量移除 |
| `chat_session.rs` | 639 → 586 | **−53** | 删 `EphemeralChatSession` 独立实现，改为 `PersistentChatSession::ephemeral()` |
| `model_chat.rs` | 71 → 60 | **−11** | 兜底值删除，契约翻译收敛为单一函数 |
| `compression.rs` | 760 → 777 | +17 | 轮边界回退缺陷（P0）修复 + 豁免判定 |
| `orchestrator.rs` | 1366 → 1394 | +28 | `req_base` 收敛 + resume/message 互斥防御 |
| `plugin.rs` | 707 → 737 | +30 | schema 新增 3 个原硬编码项（fade×2 / prune）+ 派生默认值辅助闭包 |
| `store/memory.rs` | 0 → 76 | +76 | 新增内存后端（B1 的落地载体，替代两份引擎实现） |
| 其余（file/sqlite/mod/handlers/heartbeat） | — | +24 | 路径助手收敛、嵌套能力声明、归档门控 |

需要更正三处被后续读者误信的数字：

1. **"§7 表内 `chat_loop.rs 1784 → ~1100`、`plugin.rs 707 → ≤620` 未达成"属预期**：这两行是
   批次 D（`run_chat_loop` 892 行拆分）的目标，§4 中即标注"可选，暂缓"，本轮未执行。
   实际 `chat_loop.rs` 生产行 1054 → 991（−63），`plugin.rs` 生产行 707 → 737（**+30，未达 ≤620**）。
2. **"142 行 JSON Schema" 是错的**（本文 §3.2-R5 与 §8.3-C1 均沿用）：HEAD 的
   `config_schema()` 函数体是 **216–277 行 = 62 行**。本轮该函数变为 **83 行（225–307）——是增加而非减少**，
   原因是把 3 个原硬编码在 `chat_loop` 的常量（`fade_activate_rounds` / `fade_keep_recent_turns` /
   `prune_tool_history`）提升为可配置项，schema 必须为它们补条目。
   C1 的**真实**收益是：函数体内 `default` 字面量由 **9 处降为 0**（HEAD 9 处 → 现在 12 处
   `"default"` 键全部写作 `d("key")`，从 `SessionConfig::default()` 派生），并由
   `config_schema_defaults_match_session_config_default` 锁死一致性。
   §8.3-C1 与 `docs/CHANGELOG.md` 中"142 行降到 83 行"的表述已按此更正。
3. **`plugin.rs` 文件行数 749 → 882**（+133）中，+103 行是新增测试（42 → 145），
   并非生产代码膨胀；测试总数 2502 → 3152 行、`cargo test --lib` 用例 313 → 337。

本轮的实质收益不在行数，而在**结构数量与真源唯一性**：`impl ChatSession for` 4 → 1、
截断/切分实现 4 → 1、默认值声明源 2 → 1（并有覆盖性测试）、配置面漂移点 3 → 0、
存储后端契约测试 0 → 3 套（file / sqlite / memory）。

### 8.6 遗留决策项收口（2026-09-13，经用户授权实施）

§8.5 列出的两项"需用户决定"的遗留实施项，经授权后已全部落地：

**① `load_history` 序列化行为归一**（对应复杂度审计 §8.2-P1⑤ 原"刻意保持原样"项）

- 取证：`model_chat::Request` 中 `load_history` 是**唯一缺少** `skip_serializing_if` 的
  `Option` 字段——同结构体内自相矛盾：`None` 时其他可选字段消失、它却输出 `"load_history":null`。
- 处置：补齐该属性，使序列化**键集恒定**、`None`/`Some(true)`/`Some(false)` 三态无损往返。
  **语义零变化**（`None` 与 `Some(true)` 本就同为"加载历史"）。
- 风险边界：该结构体是 session→model 的**纯进程内**契约——无 TS 对应文件（`tauri/src/protocols/`
  不存在 `model_chat.ts`）、不落库、`cli/` 与 `tauri/src-tauri/` 均不引用，故无跨语言兼容风险。
- 测试：3 个 serde 契约测试 + 穷尽结构体字面量（新增字段即编译失败，契约不可被无声破坏）；
  变异测试（加删 `skip_serializing_if`）确认有拦截力。

**② 批次 D：`run_chat_loop` 拆分**（前置竞态先修后拆）

- **前置竞态取证（确认为真实缺陷）**：消费循环的提前出口（watchdog 超时、业务 Error 帧）
  跳过了 `ai_control_tx = None` 清理，留下指向已关闭通道的**陈旧 sender**；而 `handle_abort`
  恰以 `ai_control_tx.is_none()` 作为子任务退出判据 → abort 必然空等 3s 兜底、Abort 帧投递到
  死通道被静默丢弃。
- **修复方式**：新增 `AiControlGuard`（`Drop` 守卫，与既有 `WorkingGuard` 同型）——登记时快照
  request_id，注销时仅当仍是本轮登记才清除。**跳过清理在语言层面不再可能表达**，panic 路径
  同样被覆盖；配 3 个回归测试（Drop 注销 / disarm 不重复注销 / 陈旧守卫不误伤下一轮登记），
  变异测试双向验证（去掉 Drop 体、去掉 `armed=false` 各自被对应测试拦截）。
- **拆分结果**：`run_chat_loop` 640 → **404 行**（纯骨架：装载上下文 → 消费流 → 收尾分派）；
  提取 `close_turn`（241 行：截断续写 / 主动压缩拦截 / 工具分发 / 父节点状态落库 / 停等判定，
  以 `TurnFlow::{NextTurn,Finish}` 回传循环决策）与 `run_context_compact`（压缩执行）。
- **拆函数不拆行为的证据**：搬移段与拆分前逐行比对，241 行区间仅 11 处差异，全部为机械改写
  （借用形式 `&mut out`/`&channel`、出口 `continue`→`NextTurn`、`return Ok(())`→`Finish`），
  三条出口路径与拆分前逐一对应；期间 `tool_rounds` 按值传参导致计数不推进的问题**在编译期
  即暴露**并改为 `&mut`（这是提取共享状态函数时的主要风险点）。
- **不变式复核**：文档 §5 强调的"终态唯一落库点在 orchestrator"未被搅浑——
  `finalize_assistant_turn` 调用点数量与位置与拆分前一致（1 处）。

**本轮验收**：`cargo test --lib` **343 passed / 0 failed**（337 基线 + 3 serde 契约 + 3 守卫回归）；
`cargo clippy --lib --tests -- -D warnings` 零告警；`cli`、`tauri/src-tauri` 两 crate `cargo check` 通过。
至此 §8.5 遗留决策项**全部收口**，session 机制审计不再有未决项。

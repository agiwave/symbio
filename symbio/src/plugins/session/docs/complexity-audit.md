# Session 插件复杂度审计：成因、冗余清单与简化方案

> 日期：2026-09-12（实施记录见 §八）｜ 审计基线 commit：`3ef56fd`
> ⚠️ **文中 `文件:行号` 是审计当时的快照**：S2/S3（2026-09-16）已把 `chat_loop.rs`、
> `plugin.rs` 拆成多文件（见 `./module-layout.md`），且 `config_schema` 已改为
> `config_definition`——行号与部分符号名已漂移，**一律按符号名检索**。
> 范围：`symbio/src/plugins/session/`（26 文件 / 11,829 行，其中生产代码 9,597 行）
> 结论先行：session 的复杂度里约 **4 成是上下文工程的必要代价**，另外 **6 成是"同一决策多处声明 + 视图层/存储层职责串线 + 无消费者的配置面"** 造成的可消除冗余。已定位 **4 个真实行为缺陷**（其中 2 个可直接复现）。

---

## 一、规模基线

| 文件 | 总行 | 生产 | 测试 | 说明 |
|---|---:|---:|---:|---|
| chat_loop.rs | 1784 | 1566 | 218 | 单函数 `run_chat_loop_task` 约 700 行，8 个退出点各自重复 finalize |
| orchestrator.rs | 1294 | 1294 | **0** | 零测试的中央编排器 |
| compression.rs | 1336 | 698 | 638 | L1 自动压缩 + L2 主动压缩 + 兜底 |
| tool_executor.rs | 924 | 819 | 105 | 工具分发/审批/失败回传 |
| context_window.rs | 825 | 387 | 438 | 请求视图骨架化（测试多于实现） |
| plugin.rs | 690 | 651 | 39 | 含 120 行手写 JSON Schema |
| chat_session.rs | 573 | 571 | **2** | 存储门面 + 双会话实现 + 存储期裁剪 |
| 其余 19 文件 | 3477 | — | — | options/workdir/resume/handlers/store/… |

---

## 二、复杂度的来源：必要 vs 可消除

### 2.1 必要内核（约 40%，应保留）

1. **多 Provider 协议适配**（OpenAI/Anthropic/Gemini 的 id 字段与 reasoning 落位差异）——已下沉 `symbio_core::turn` 的 `TurnLayout`，是本轮最好的架构收敛。
2. **压缩必须防"输入超限死锁"**——L0 守卫 / L1 自动 / L2 主动 / 本地机械兜底，四层对应四种真实故障模式，且都有注释说明失败场景。
3. **工具链配对完整性**（ToolCall↔Tool 不能断联）与 **Turn 子树完整性**（切分点不能落在 Turn 首个 ToolCall）——provider 硬约束。
4. **失败必须成为可恢复的一等状态**（Failed + error + retry，而非静默降级）。

这四项是"对话引擎"与"HTTP 转发器"的分界线，不该被简化掉。

### 2.2 可消除的复杂度（约 60%）

**成因 A：同一个决策在 2–4 处独立声明，彼此不同步**

| 决策 | 声明点 | 值 |
|---|---|---|
| 工具窗口默认值 | `plugin.rs:257`(schema) / `session_config.rs`(fn) / `chat_loop.rs:563`(`unwrap_or`) / `Ephemeral::new` | 15 / 15 / **15** / 15 |
| 轮次窗口默认值 | `chat_loop.rs:471` / `chat_session.rs:266` / `Ephemeral::new` | 6 / 6 / 6 |
| 淡化行数阈值 | `chat_session.rs:429`(trait 兜底) / `session_config.rs` | 200 / 200 |
| 淡化保护数 | `chat_session.rs:65`(trait 默认) / `:436`(unwrap_or) / `session_config.rs` | 3 / 3 / 3 |
| 上下文窗口兜底 | `orchestrator.rs:107` / `chat_loop.rs:466` | 65536 / 65536 |
| **max_tool_rounds** | `plugin.rs:247`(schema) vs `session_config.rs:69`(结构体) | **15 vs 65535** ⚠️ |

前五行"暂时同步"，靠人工维护；最后一行已经漂移。

**成因 B：视图层与存储层职责串线**

架构原则写在 `chat_loop.rs:552`（"存储保持完整历史，视图逐轮裁剪"）和 `chat_session.rs:283`，但 `append_messages` 实际做了三件越界的事：
1. `prune_historical_tool_calls`：**物理删除** ToolCall+子树并落盘；
2. `max_messages` FIFO：**物理 drain** 并删存档文件；
3. 两者都发生在"写入时刻"，而不是"读取视图时刻"。

后果是**双实现互为死代码**：`prune_historical_tool_calls`（存储期裁剪）与 `context_window::apply_layered_sliding_window`（视图期骨架化）解决同一问题，前者已把老工具链删掉，后者永远看不到它们 → 视图层的"工具级保留策略 + 锚点保留"这套精细逻辑实际作用面被上游削平。

**成因 C：配置面 > 消费面**

`SessionConfig` 10 个字段，其中 `session_id` **全仓零读取**（死字段）；`max_tool_rounds` 与 `tool_context_window` 虽被 orchestrator 读取，但对应语义在下游被架空（见 3.2 / 3.3）。

**成因 D：会话抽象被实现两遍，第二份无人使用**

`FileChatSession` / `EphemeralChatSession` 逐方法复刻 `ChatSession`（含 `backfill_timestamps`、seq 分配、孤儿存档清理、FIFO 淘汰、5 个 trait 方法）。全仓 `EphemeralChatSession` 的**唯一使用者是它自己的单测**（`chat_session.rs:635`）。

**成因 E：中央编排器持锁跨网络 I/O**

`orchestrator.rs:902` 在 `state.inner.write()` 临界区内 `config.read().await` 并克隆，随后构造 3 个 `json!` 分支（resume / ping / 普通），三个分支的 6 个字段完全重复。

---

## 三、行为缺陷（按严重度）

### 3.1 【高】`max_messages` 被静默钳到 ≥500，配置项形同虚设

```
chat_session.rs:314   let max_turns = self.config.read().await.max_messages.max(500);
chat_session.rs:457   max_messages: config.max_messages.max(500),
```
而设置面板 `plugin.rs:222` 声明 `min=10, max=1000, default=100`。
→ 用户设 10–499 的任何值都**不生效**（等价 500）；UI 上的"最大消息数"是一个说谎的开关。
→ 修复：要么把 `.max(500)` 去掉（尊重配置），要么把下限改成配置项并在 schema 里如实标注 `minimum: 500`。

### 3.2 【高】`max_tool_rounds` 双默认值漂移，可能触发硬截停

- `session_config.rs:68` → **65535**
- `plugin.rs:244` schema → **15**
- `chat_loop.rs:332` 注释明确记录产品决策：**"用户明确要求不要设置 max_tool_rounds 硬性上限"**
- `orchestrator.rs:961/975/997` 无条件 `Some(session_cfg.max_tool_rounds)`，使 `chat_loop` 的"None = 无限轮次"分支在 session 路径上永不成立。

→ 一旦有代码/配置走 `config_schema()` 的默认值，长会话将在第 15 轮被软上限打断并回"已达到本轮工具调用上限"，与既定产品决策直接冲突。
→ 修复：两处默认值必须同源；session 路径应传 `None`（或 `0` 表示无限），把"无限"作为显式语义而不是 65535 这个魔法数。（实施说明见 §八：默认值已同源、`0 = 不限制` 已成为显式语义；把默认值本身从 65535 迁到 `0` 属破坏性变更，需单独授权，见 §8.5-1。）

### 3.3 【中】`tool_context_window = 0` 的语义与文档相反

```
chat_loop.rs:563   let window = req.tool_context_window.unwrap_or(15);
compression.rs:557 if window > 0 && !retention.is_empty() { ...apply_layered_sliding_window... }
```
`plugin.rs:254-261` 承诺"0 = 不启用工具级骨架化"。但由于 `retention` 只收录**非 All** 的策略声明（全仓仅 `todo_write`、`heartbeat_tool` 声明 `LastOnly`），`!retention.is_empty()` 在装载本地工具的会话里恒为真；此时 `window=0` → `active_threshold = len - 0 = len` → **所有** ToolCall 判为 stale → 全量骨架化，即"关闭"变成"最激进"。
→ 修复：门控条件改为 `window > 0` 单独决定；或 `usize::MAX` 表示关闭；且 `unwrap_or(15)` 的兜底值应来自 `SessionConfig::default()`。

### 3.4 【中】`context_messages = 0` 与 schema 承诺相反

`plugin.rs:234` 写"0 表示不限制"，但 `sliding_window` 的 `turns=0` 会返回空视图（`chat_session.rs:264` 兜底只在 config 读取失败时生效）。→ 0 应显式短路为"不裁剪"。

### 3.5 【低】`store_kind` 的 Sqlite 分支静默降级

`plugin.rs:251` 声明支持 `"sqlite"`，但 `chat_session.rs:213` 对非 File 一律返回 `NotImplemented`，调用方回退文件存储且无日志告知用户。→ 要么实现，要么从 schema 移除该选项。

### 3.6 【低】`session_id` 是 `SessionConfig` 的死字段

全仓无 `config.session_id` 读取点。会话身份由 `FileChatSession.session_id` 承载，配置里再放一份只会造成"改了不生效"的误解。

---

## 四、冗余清单（可直接执行）

| # | 冗余项 | 规模 | 处置 | 风险 |
|---|---|---:|---|---|
| R1 | `EphemeralChatSession` 全实现 | 116 行 | 删除；测试改用 `FileChatSession` + 临时目录，或把 `ChatSession` 的内存后端做成 `MemorySessionStore`（复用现成 trait，~40 行） | 低 |
| R2 | `prune_historical_tool_calls` 与视图层骨架化 | 148 行 | 二选一。建议**保留视图层**（符合"存储完整"原则），删存储期裁剪 | 中：需回归 resume/压缩 |
| R3 | `SessionConfig` 的 `compress_line_threshold` / `compress_keep_recent` 与 fade 常量 | ~30 行 | 二者当前都走 config 路径，但 `chat_loop` 里另有 `FADE_ACTIVATE_ROUNDS` / `FADE_KEEP_RECENT_TURNS` 常量——统一进 `SessionConfig` 单一来源 | 低 |
| R4 | 6 组"同一默认值多处声明" | ~25 行 | 引入 `impl SessionConfig { pub fn tool_window_or_default(...) }`，或直接让所有兜底引用 `SessionConfig::default()` 字段 | 低 |
| R5 | `config_schema()` 手写 JSON 与 `SessionConfig` serde 双份 | 120 行 | 改由 `schemars`/宏生成，或加**一致性单测**：遍历 struct 字段 ⇄ schema properties 一一对应、默认值相等 | 低（先加测试即可止血） |
| R6 | `orchestrator.rs` 3 个 `json!` 分支重复 6 字段 | ~30 行 | 提取 `build_chat_input(cfg, kind)`，分支只决定 `single_message`/`resume`/`system_prompt` | 低 |
| R7 | `chat_loop.rs` 8 个退出点重复 finalize | ~120 行 | 抽 `finalize_turn(outcome)`；8 处改为一行调用 | 中：需覆盖 abort/软上限/超时/panic 分支测试 |
| R8 | `session_id.is_empty() \|\| == "default"` 特判 | 1 处 | 在入口归一化一次，下游不再判空 | 低 |
| R9 | `workdir.rs` 候选路径列表出现 2 次 | ~15 行 | 提取 `const CANDIDATES: &[&str]` | 低 |

**合计可移除 ≈ 450–500 行生产代码**，并消除 6 组一致性隐患。

---

## 五、简化方案（分四步，每步可独立提交）

### Step 0 · 止血（0.5 天，零行为变更）
- 加**配置面一致性测试**：`SessionConfig` 字段 ⇄ `config_schema()` properties 双向对齐；每个字段的 `default` 与 `default_*()` 函数值相等。（这条测试今天就会因 `max_tool_rounds` 15≠65535 而失败——先让它红。）
- 修 3.1（`max(500)`）、3.2（默认值同源）、3.3（window 门控）、3.4（0 语义）。
- 给 `orchestrator.rs`（1294 行 0 测试）补最小用例：会话句柄缓存、三字段解析链、Request 构造。

### Step 1 · 删无消费者的抽象（1 天）
- 删 `EphemeralChatSession`（R1）与 `SessionConfig.session_id`（3.6）。
- `store_kind` 的 sqlite 选项从 schema 移除，或返回明确错误（3.5）。
- 收益：−120 行，`ChatSession` trait 回到单一实现。

### Step 2 · 划清存储层 / 视图层（2–3 天，需回归）
- 确立唯一原则：**存储只追加 + 只按 `max_messages` 做 FIFO 淘汰；一切"给模型看什么"的裁剪都在 `build_request_view`。**
- 移除 `prune_historical_tool_calls`（R2），其职责由 `apply_layered_sliding_window` 承接（它本就更强：带锚点保留、工具级策略、Turn 完整性）。
- 存档文件生命周期改由 `replace_messages` + FIFO 淘汰两处管理（已有孤儿清理逻辑，去掉第三处）。
- 收益：−150 行 + 消除"上游删干净导致下游逻辑空转"的隐性失效。

### Step 3 · 机制化（1–2 天）
- **配置单源**：`SessionConfig` 作为唯一真值，schema 由宏生成（R5）；所有 `unwrap_or(魔法数)` 改为引用 `SessionConfig::default()`（R4）。
- **退出路径机制化**：`finalize_turn(outcome)` 收口 8 个退出点（R7）；`StopSignal` 已存在，把 abort/软上限/压缩中止统一映射到 `TurnOutcome` 枚举。
- **Request 构造机制化**：R6 提取函数。
- 收益：`chat_loop.rs` 从 1784 行降到约 1450 行，`orchestrator.rs` 降到约 1200 行，且新增轮次/压缩策略只需改一处。

### 预期终态
- 生产代码 9,597 → 约 **8,900 行**（−7%），但**决策声明点从 18 处降到 6 处**，配置项 100% 有消费者且值一致。
- 复杂度分布从"编排层承担"转为"内核 + 数据驱动"：轮次策略、压缩层级、保留策略都变成可读的表/枚举，而不是散落在 4 个文件里的 `if`。

---

## 六、不要动的地方（明确保留）

1. `symbio_core::turn` 的 `TurnLayout` / `TurnOutput::into_wire_messages` —— 协议差异的唯一出口，session 侧不得再出现 provider 名判断。
2. 压缩四层（L0/L1/L2/本地兜底）—— 每层对应一种真实故障，合并会造成"超限即死锁"回归。
3. `context_window.rs` 的 Turn 完整性守卫（`find_turn_head` / 切分点不早于 Turn 首个 ToolCall）—— provider 400 的直接防线。
4. `tool_result_guard.rs`（L0）—— 在**生产时刻**截断是唯一能防止单条工具输出打爆请求的做法。
5. 失败回传 + `TurnOutcome` + 重试收窄 —— 已文档化，是产品级要求。

---

## 附：证据索引（文件:行）

- `chat_session.rs:314` / `:457` — `max_messages.max(500)`
- `plugin.rs:222` — `max_messages` schema 仅有 `default: 100`（无 `min`/`max` 约束），而存储层另有 `.max(500)` 隐式下限：面板可设 10，实际行为是 500
- `session_config.rs:68` vs `plugin.rs:247` — `max_tool_rounds` 65535 vs 15
- `chat_loop.rs:332` — "不要设置硬性上限"的产品决策注释
- `chat_loop.rs:563` / `compression.rs:557` — `window=0` 语义反转
- `chat_loop.rs:471` / `chat_session.rs:266` — `context_messages` 兜底 6
- `chat_session.rs:563` — `prune_historical_tool_calls`（存储期物理删除）
- `chat_session.rs:283` — "存储保持完整原文"的注释（与上一行矛盾）
- `chat_session.rs:440-555` — `EphemeralChatSession`（唯一使用者 `:635` 自身测试）
- `orchestrator.rs:902` / `:955-1005` — 持写锁读 config + 3 分支重复字段
- `session_config.rs:35-36` — `session_id` 死字段
- `plugin.rs:251` / `chat_session.rs:213` — `store_kind=sqlite` 声明但不实现

---

## 八、实施记录（2026-09-12，基线 `3ef56fd` 之上的未提交改动）

用户授权"推动实施并验证"后，按 §五 的分级方案落地了 **P0 全部 + P1 的 ①②③④⑤**。
P2（EphemeralChatSession 归一、FallbackChatSession 语义对齐、resume 互斥、文档同步）未动，属需单独决策的范围。

### 8.1 已修复的行为缺陷

| 项 | 改动 | 位置 |
|---|---|---|
| **P0-1** `max_tool_rounds` 契约漂移 | 产品意图按 README 定为"默认 65535 = 实质无上限"；新增 `SessionConfig::model_chat_max_tool_rounds()` 作为**唯一契约翻译点**（`0 → None`，`>0 → Some`），编排层不再无条件 `Some(...)` | `session_config.rs`、`orchestrator.rs` |
| 同上（chat_loop 侧） | `req.max_tool_rounds.filter(|n| *n > 0)`，使 `Some(0)` 与 `None` 同义，消除"0 轮即熔断" | `chat_loop.rs:336` |
| 同上（schema 侧） | schema 的 `max_tool_rounds.default` 不再硬编码 15，改由 `json!(defaults["max_tool_rounds"])` 派生 | `plugin.rs::config_schema` |
| **P0-2** `context_messages=0` 越界 panic | `prune_historical_tool_calls` 开头加 `if keep_turns == 0 { return; }`，语义与 `sliding_window`、`tool_context_window` 的"0 = 不限制"统一 | `chat_session.rs:585` |
| **P0-3** `max_messages` 被 `.max(500)` 钳制 | 删除两处硬编码下限（Persistent 与 Ephemeral 的 `EphemeralChatSession::new`），Persistent 侧补 `max_turns > 0` 守卫；0 = 不限制的语义由既有 `sliding_window` 的 `max_turns == 0` 早返回承担，不再另造规则 | `chat_session.rs`（HEAD 原 314 / 457 两处 `.max(500)` 均删除） |

### 8.2 已消除的冗余

- **P1-③ 配置面默认值单一真源**：`config_schema` 原先逐字段硬编码 `default` 字面量（`100 / true / false / 6 / 15 / 200 / 3 / 15 / "file"`），与 `SessionConfig` 的 serde default 各写一份；现改为先 `serde_json::to_value(SessionConfig::default())`，每个字段的 `"default"` 一律取自该结果（局部闭包 `d(key)`），schema 只保留 UI 元信息（type/title/description/enum）。注：原 schema 本就未声明 `min`/`max`，本次不新增边界，避免再造一套假约束。`session_id` 属身份/路由字段（由调用方注入），不进设置面板，在覆盖性测试中以 `NON_CONFIGURABLE` 常量显式豁免并注明理由。
- **P1-④ 请求构造去重**：`orchestrator.rs` 三个分支各自重复书写 8 个会话派生字段（`stream / max_tool_rounds / tool_context_window / auto_compress / enable_compact_tool / provider_id / load_history / resume`），收敛为单一 `req_base` + 各分支 `..req_base` 仅覆盖真正不同的字段（resume 分支只覆盖 `load_history: Some(true)`）。重复字段行 24 → 0，文件净 -6 行；真正的收益是"改一处不再漏两处"。
- **P1-⑤ 死字段清理**：删除 `model_chat::ThinkingConfig` 与 `Request.thinking`（session 各构造点恒为 `None`、无任何消费者；provider 侧 thinking 走 `cfg.reasoning`，与本字段无关）。因原字段带 `skip_serializing_if = "Option::is_none"`，删除后**跨语言线上格式零变化**；`load_history` 的序列化行为本轮刻意保持原样（超出当时授权范围；该项已于 2026-09-13 经授权收口，见 `./mechanism-audit.md` §8.6-①）。

### 8.3 存储/视图矛盾的处理（P1-②）

采纳"保留为显式例外 + 可关闭"，而非直接移除：新增 `SessionConfig::prune_tool_history`（serde 默认 `true`，保持历史行为），`append_messages` 中据此决定是否调用 `prune_historical_tool_calls`；函数与调用点的注释改写为"架构原则的唯一例外 + 存在理由（控制磁盘与节点树体积）+ 关闭后的语义"。设为 `false` 时存储严格保留全文，工具链裁剪完全由 `build_request_view` 承担。

### 8.4 验证证据

- `cargo check --workspace`：0 error / 0 warning；`cargo clippy --lib -- -D warnings`：通过。
- `cargo test --lib`：**325 passed / 0 failed**（基线 313 → 325，新增 12 个用例，无忽略项）。
- 新增用例（按文件）：
  - `chat_session.test.rs`（8 个，均 `#[tokio::test]`）：`prune_zero_keep_turns_does_not_panic`、`prune_empty_messages_zero_keep_turns_is_noop`、`prune_keeps_recent_turns_and_drops_older_tool_chain`、`append_messages_with_zero_context_messages_does_not_fail`、`append_messages_prunes_tool_chain_by_default`、`append_messages_keeps_tool_chain_when_prune_disabled`、`max_messages_below_legacy_store_floor_is_honored`、`max_messages_zero_means_unlimited`。
  - `plugin.rs`（2 个）：`config_schema_defaults_match_session_config_default`、`config_schema_covers_all_session_config_fields`。
  - `session_config.rs`（2 个）：`max_tool_rounds_zero_maps_to_unlimited`、`default_max_tool_rounds_is_effectively_unlimited`。
- 原有 `test_replace_messages_*`（4 个，含 2 个路径穿越拒绝用例）保持通过，未因本轮改动退化。
- 前端影响面核查：`tauri/src` 无任何按字面量读取 session `config/schema` 默认值的代码（schema 为动态渲染），故默认值 15 → 65535 的收敛不需前端同步改动。
- **变异测试**（确认新测试有拦截力，非恒真断言）：
  1. 把 schema 的 `max_tool_rounds` default 改回硬编码 `15` → `config_schema_defaults_match_session_config_default` **FAILED**，报错直指"max_tool_rounds: schema default 15 != SessionConfig::default() 65535"（即历史缺陷本身）；
  2. 删掉 `prune_historical_tool_calls` 的 `keep_turns == 0` 守卫 → `prune_zero_keep_turns_does_not_panic` **FAILED**（`index out of bounds: the len is 0 but the index is 0`）；
  3. 重新引入 `max_messages` 的 `.max(500)` 下限 → `max_messages_below_legacy_store_floor_is_honored` **FAILED**。
  三处变异均已还原并复验通过，最终 diff 仅含预期的 7 个源文件 + 1 个文档。

### 8.5 遗留决策项（收尾结论，2026-09-13）

原列为"需用户单独确认"的 6 项已全部收口（用户授权"推动实施并验证"后按下述方式处置）：

1. **`max_tool_rounds` 默认值** → **已执行**：由魔法数 `65535` 改为 `0`（`0 = 不限制`，与
   `max_messages` / `tool_context_window` 的 0 语义统一）。迁移安全性即本项原先记载的核实结论；
   契约翻译唯一入口 `model_chat_max_tool_rounds()`（`0 → None`），`chat_loop` 不再自持兜底值。
2. **`prune_tool_history` 默认值** → **保持 `true`**（默认不改变既有磁盘/节点树行为），但把
   "存储期物理删除归档文件"这一条摘掉（见机制审计 §8.1-A3）：prune 现只删消息节点，归档文件的磁盘
   生命周期唯一归 `tool_result_guard` 的 `TOOL_ARCHIVE_KEEP` 滚动策略。需要"存储恒为完整原文"
   的会话显式设 `prune_tool_history = false` 即可，不必移除该路径。
3. **`session_id` 死字段** → **已从 `SessionConfig` 删除**（比原方案"仅在 schema 层豁免"更彻底）。
   旧 `session_config.json` 中残留的该键被 serde 静默忽略（未开 `deny_unknown_fields`），零迁移成本。
4. **`store_kind = sqlite`** → **结论修正**：取证时该后端实为**已完整实现、只缺测试**（本文 §3.5
   的判断过时）。处置为"补契约测试 + 真正接通 `memory` 后端"，配置面保留，详见机制审计 §8.2-B1、§8.4。
5. **多套会话实现归一 + resume/message 互斥** → **已执行**：`EphemeralChatSession` 与
   `FallbackChatSession` 均删除，临时/降级会话改为 `PersistentChatSession` + `InMemorySessionStore`
   （`impl ChatSession for` 由 4 降到 1）；`orchestrator.rs` 中 resume 与 message 同时提供时显式报错，
   不再让 user 消息被静默落库而请求走 resume 分支。
6. **两份审计文档的取舍** → **本文件归档，`./mechanism-audit.md` 为现行权威版本**。
   机制审计 §6 已逐条修正本文的判断（P2/P4 撤销、P6 降级、R6"死机制"作废等），批次 A/B/C 的
   落地记录亦写在机制审计 §8；本文保留作取证过程与规模基线的历史记录。

> 对外可见的行为/配置变更事实统一记于 `docs/CHANGELOG.md` 的 2026-09-13 条目；
> 现行机制描述见 `../README.md`。

# Session 插件模块分工评审

> 状态：**评审 + 执行方案**（S1/S2/S3 + 测试扁平化已落地；S4 待执行）
> 触发：`chat_loop.rs` 达 2400 行，已到"不该再往单文件里加东西"的程度
> 范围：`symbio/src/plugins/session/`（27 个 .rs，合计 16647 行；**S1 后**生产 12419 + 测试 4313）
> 相关：`./core-loop.md`（核心循环收口，同批次完成）

---

## 0. 一句话结论

分工的**骨架是清楚的**（引擎 / 存储 / 策略 / 执行 / 场景各有其文件），问题出在两点：

1. **测试与实现混放**——全插件 **3713 行（22%）** 是内联 `#[cfg(test)] mod tests`，
   而项目里**已经有两处正确示范**（`chat_session.test.rs`、`store/mod.rs` + `store/tests.rs`）。
   仅此一项就能把 `chat_loop.rs` 2400 → 1993、`plugin.rs` 2191 → 1479、
   `compression.rs` 1641 → 806、`context_window.rs` 875 → 411。
2. **三个"什么都往里放"的文件**——`chat_loop.rs` / `plugin.rs` / `orchestrator.rs`
   各自承担了 3~6 类不相干的职责，靠"文件"这一层已经无法表达边界。

---

## 1. 现状测绘

### 1.1 按生产代码量排序（已扣除测试）

| 文件 | 生产 | 测试 | 职责数 | 判定 |
|---|---:|---:|---:|---|
| `chat_loop.rs` | **1993** | 407 | **6** | 🔴 拆 |
| `orchestrator.rs` | **1510** | 180 | 3 | 🟠 拆（含 382/298 行巨函数） |
| `plugin.rs` | **1479** | 712 | **4** | 🔴 拆 |
| `compression.rs` | 806 | 835 | 1 | 🟢 保留（23 个纯函数，内聚） |
| `tool_executor.rs` | 866 | 121 | 2 | 🟢 保留 |
| `workdir.rs` | 498 | 193 | 1 | 🟢 保留 |
| `store/mod.rs` | 676 | — | 2 | 🟢 保留（测试已外置） |
| `options.rs` | 463 | 134 | 1 | 🟢 保留 |
| `chat_session.rs` | 585 | — | 1 | 🟢 保留（测试已外置） |
| `resume.rs` | 470 | 0 | 1 | 🟢 保留 |
| `context_window.rs` | 411 | 464 | 1 | 🟢 保留 |
| `types.rs` | 284 | 89 | 1 | 🟢 |
| `tokenizer.rs` | 230 | 51 | 1 | 🟢 |
| `tool_result_guard.rs` | 222 | 128 | 1 | 🟢 |
| `heartbeat.rs` | 255 | 18 | 1 | 🟢 |
| `heartbeat_tool.rs` | 191 | 37 | 1 | 🟢 |
| `handlers.rs` | 317 | 0 | 1 | 🟢 |
| `text_split.rs` | 129 | 75 | 1 | 🟢 |
| `prompt.rs` | 116 | 41 | 1 | 🟢 |
| `model_chat.rs` | 61 | 87 | 1 | 🟢 |
| `paths.rs` | 53 | 67 | 1 | 🟢 |
| `rate_limit.rs` | 78 | 35 | 1 | 🟢 |
| `fs_watcher.rs` | 78 | 32 | 1 | 🟢 |
| `active.rs` | 110 | 0 | 1 | 🟢 |
| `chat_pipeline.rs` | 105 | 0 | 1 | 🟢 |
| `mod.rs` | 35 | 4 | 1 | 🟢 |

**判据**：单文件生产代码 > 800 行，或职责数 ≥ 3 → 拆。

### 1.2 超长函数（生产代码）

| 文件:行 | 行数 | 函数 | 问题 |
|---|---:|---|---|
| `orchestrator.rs:363` | 382 | `run_chat_loop_task` | 消费循环 + 帧合并 + 终态收尾挤在一起 |
| `orchestrator.rs:887` | 298 | `handle_chat_send_oneoff` | 装配层：参数解析 + 落库 + 命名 + ctx 装配 |
| `chat_loop.rs:1053` | 286 | `close_turn` | 工具分发 + 落库 + 下一步判定（收口后仍是最大块） |
| `chat_loop.rs:432` | 280 | `run_chat_loop` | 前步骤 4 段 + loop 6 步（已收口，可接受） |
| `chat_loop.rs:1544` | 230 | `compress_with_snapshot_core` | 压缩流水线内核（单一实现，不宜再拆） |
| `orchestrator.rs:1312` | 185 | `persist_failure` | 失败收尾作用域收窄 |
| `chat_loop.rs:779` | 103 | `prepare_turn_inputs` | 五步收口（可接受） |

> 注：`plugin.rs` 最长函数仅 100 行——它的病**不是巨函数，是"项太多、职责杂"**
> （`impl Plugin` + `impl VdfsProvider` + 25 个 VDFS 节点构造辅助函数同处一文件）。

---

## 2. 问题清单

### P1 · 测试与实现混放（全插件级）

- 20 个文件带内联 `#[cfg(test)] mod tests`，合计 **3713 行**。
- `compression.rs`（835 > 806）与 `context_window.rs`（464 > 411）**测试比生产代码还多**，
  读实现时要在 800+ 行测试里找生产函数。
- **已有正确示范**：`chat_session.rs` 尾部 `#[cfg(test)] mod tests;` + `chat_session.test.rs`；
  `store/mod.rs` 同理 + `store/tests.rs`（`mod.rs` 形态的模块，测试与 `mod.rs` 同级）。
  **约定已存在，只是没铺开。**

### P2 · `chat_loop.rs` 六类职责同处一文件

| # | 职责 | 内容 |
|---|---|---|
| 1 | 状态与契约类型 | `SessionContext` / `TurnRequest` / `TurnState` / `TurnExit` / `Gate` / `TurnResult` / `TurnFlow` |
| 2 | 编排器与生命周期 | `ChatOrchestrator` / `StopSignal`（含 `Drop` 兜底） |
| 3 | 主循环骨架 | `run_chat_loop` / `gate_turn` / `finish_turn` |
| 4 | 输入准备 | `resolve_system_prompt` / `TurnInputs` / `prepare_turn_inputs` / `apply_compaction` |
| 5 | 单轮结算 | `settle_reasoning` / `close_turn` / `feedback_estimate` |
| 6 | **压缩流水线** | `auto_compress_process` / `compress_with_snapshot_core` / `run_context_compact` / `send_compression_request` / `run_compression_llm` / `save_transcript_archive` / `summary_text`（≈560 行） |
| 7 | 通道小工具 | `persist_messages` / `broadcast_message_update` / `emit_streaming_start` / `open_chat_session` / `fire_*_hook` |

> 职责 6 尤其错位：压缩的**策略**在 `compression.rs`（阈值 / 切分 / 视图重建），
> 压缩的**流水线**（LLM 摘要请求 / 快照校验 / transcript 转存）却在 `chat_loop.rs`。
> 同一概念被切成两处，读者必须跨文件才能拼出完整压缩链路。

### P3 · `plugin.rs` 四类职责同处一文件

| # | 职责 | 内容 |
|---|---|---|
| 1 | `Plugin` trait 实现 | `route` 分发 / `traverse`（能力收集）/ 生命周期 |
| 2 | `VdfsProvider` 实现 | 会话节点树：`list` / `stat` / `read` / `write` / `delete` / `watch` |
| 3 | VDFS 节点构造辅助 | `session_node` / `parse_session_path` / `VdfsSessionPath` / `internal_dirs` / `message_node` / `message_label` / `ordered` / `overlay_live` / `transcript_window` / `cursor_id` / …（**25 个函数**） |
| 4 | 会话实体杂务 | `cleanup_crashed_sessions` / `now_ms` / `title_from_new_path` / `config_definition` |

### P4 · `orchestrator.rs` 三个巨函数

`run_chat_loop_task`(382) / `handle_chat_send_oneoff`(298) / `persist_failure`(185)。
其中 `run_chat_loop_task` 把「消费循环 + 帧合并 + 终态落库」三件事写在一个函数体里。

---

## 3. 目标划分

### 3.1 `chat_loop.rs` → `chat_loop/`

```text
chat_loop.rs            主循环骨架：run_chat_loop / gate_turn / finish_turn / TurnFlow   ~300
chat_loop/state.rs      状态与契约：SessionContext / TurnRequest / TurnState /
                        TurnExit / Gate / TurnResult / ChatOrchestrator / StopSignal     ~430
chat_loop/inputs.rs     输入准备：resolve_system_prompt / TurnInputs /
                        prepare_turn_inputs / apply_compaction                           ~250
chat_loop/turn.rs       单轮结算：settle_reasoning / close_turn / feedback_estimate      ~380
chat_loop/compress.rs   压缩流水线：8 个函数（与 compression.rs 策略层配对）              ~560
chat_loop/io.rs         通道与钩子：persist_messages / broadcast_message_update /
                        emit_streaming_start / open_chat_session / fire_*_hook          ~150
chat_loop.test.rs       测试（+ `inputs.test.rs` / `state.test.rs`）                    ~410
```

### 3.2 `plugin.rs` → `plugin/`

原计划二分（下表曾是二分口径），实测 `plugin.rs` 去掉保留段后仍有 1030 行，
二分等于把"长文件"从 `plugin.rs` 搬成 `plugin/vdfs.rs`，问题没解决。
**实际按三分执行**：

```text
plugin.rs                SessionPlugin 定义 + impl SessionPlugin + impl Plugin +
                         配置定义 / now_ms + 模块声明与共享面 re-export            ~523
plugin/nodes.rs          VDFS 节点构造 / 路径模型 / 消息投影（纯函数，不持有 self）  ~501
plugin/vdfs_provider.rs  impl VdfsProvider + 其私有辅助（impl SessionPlugin 第二块） ~512
plugin.test.rs           测试（S1 已外置；S3 后按实现文件再拆为 3 份）                 137
```

命名注意：子模块**不能叫 `vdfs`**——`plugin.rs` 已 `use crate::symbio_core::vdfs;`，
同名会 `E0255`（且子模块内 `vdfs::X` 会解析到自己）。故取 `vdfs_provider`。

### 3.3 `orchestrator.rs` → `orchestrator/`

原计划（`consume.rs` + `guards.rs`）在实测后调整为**四分**：`impl SessionPlugin`
独占 1197 行，若只拆出 guards，`consume.rs` 会拿到 773 行（其中
`run_chat_loop_task` 一个函数就 382 行）。实测边界与落位：

```text
orchestrator.rs          模块根：装配 + RAII 守卫 + resolve_required_session_id       ~320
                         （守卫 71..299 = AiControlGuard / WorkingGuard /
                           merge_message_patch，合计 229 行，留在根文件）
orchestrator/consume.rs  消费循环：broadcast_error_with_idle / fail_before_loop /
                         run_chat_loop_task(382) / handle_abort                        ~515
orchestrator/send.rs     发送入口：resolve_session_params / handle_chat_send_oneoff(298) /
                         handle_chat_abort_oneoff / ensure_auto_title                   ~425
orchestrator/report.rs   上报与失败落库：broadcast_frame / broadcast_status /
                         persist_failure(185) / subtree_of                              ~270
orchestrator.test.rs     测试（S1 已外置）                                             182
```

拆 `impl SessionPlugin`（1197 行）意味着**拆成 4 个 `impl` 块**分散到 4 个文件
——Rust 允许，且**方法声明顺序无语义**，故各文件内保持原相对顺序即可。
⚠️ `handle_chat_send_oneoff` / `handle_chat_abort_oneoff` / `broadcast_frame`
被 `plugin.rs` / `heartbeat.rs` 跨模块调用，必须 `pub(crate)`。

### 3.4 保持不动

`compression.rs`（806 生产 / 内聚纯函数）、`context_window.rs`、`tool_executor.rs`、
`workdir.rs`、`store/`、`chat_session.rs`、`resume.rs`、`types.rs` 等——
**只外置测试，不改生产划分**。

---

## 4. 执行顺序与验收

| 步 | 内容 | 风险 | 验收 |
|---|---|---|---|
| **S1** ✅ | 测试外置（**21** 个文件 → `<module>/tests.rs`；含 `chat_loop/` 的 3 个测试模块） | 低（纯搬移，`use super::*` 语义不变） | 每步 `cargo check --tests`；末次 `cargo test --lib` 用例数不变 |
| **S1b** ✅ | 测试文件**扁平化**（`<module>/tests.rs` → `<module>.test.rs`，去掉只为放测试而建的目录，见 §4.4） | 低（纯改名 + `#[path]`） | 同上 |
| **S1c** ✅ | 测试**归位**到被测试的实现文件旁（高内聚，见 §4.5） | 低（纯搬移） | 同上 |
| **S2** ✅ | 拆 `chat_loop.rs`（§3.1） | 中（跨模块可见性） | 生产代码总量不变；`cargo test --lib` 用例数不变 |
| **S3** ✅ | 拆 `plugin.rs`（§3.2，实为**三分**） | 中 | 同上 |
| **S4** | 拆 `orchestrator.rs`（§3.3） | 中高（消费循环是事故敏感区） | 同上 + 消费循环帧合并 / `persist_failure` 作用域逐字不变 |

### 4.1 S1 实施结果（2026-09-16）

| 文件 | 前 | 后（生产） |
|---|---:|---:|
| `chat_loop.rs` | 2401 | 2001 |
| `plugin.rs` | 2192 | 1482 |
| `compression.rs` | 1642 | 809 |
| **全插件** | 16647 总 | **12419 生产 + 4313 测试** |

- 21 个新测试文件：18 个模块级 `<module>/tests.rs` + `chat_loop/{tests,gate_tests,stop_signal_tests}.rs`。
  > 后续 S1b 把 `<module>/tests.rs` 扁平化为 `<module>.test.rs`，S1c 又把 3 个 `chat_loop/*_tests.rs`
  > 按实现文件重新归位（`gate_tests.rs` → `chat_loop.test.rs`，`stop_signal_tests.rs` → `state.test.rs`）。
- 每个测试文件带统一文件头（`//!` 说明"与实现分文件，约定同 `store/tests.rs`"）。
- 父文件末尾统一为 `#[cfg(test)] mod tests;`（`chat_loop.rs` 为三个 `mod`）。
- 验证：`cargo check --tests` 通过；`cargo clippy --all-targets -- -D warnings` 零告警；
  `cargo test --lib` 用例数**未减少**。

### 4.2 S2 实施结果（2026-09-16）

`chat_loop.rs` **2000 → 434 行**（只留主循环骨架），拆出 5 个子模块：

| 文件 | 行数 | 内容 |
|---|---:|---|
| `chat_loop.rs` | 434 | `mod` 声明 + 共享面 re-export + `run_chat_loop` / `TurnFlow` / `gate_turn` / `finish_turn` |
| `chat_loop/state.rs` | 363 | `SessionContext` / `TurnRequest` / `TurnState` / `TurnExit` / `Gate` / `TurnResult` / `StopSignal` / `ChatOrchestrator` |
| `chat_loop/inputs.rs` | 252 | `resolve_system_prompt` / `TurnInputs` / `prepare_turn_inputs` / `apply_compaction` |
| `chat_loop/turn.rs` | 372 | `settle_reasoning` / `close_turn` / `feedback_estimate` / `MAX_CONTINUE_ROUNDS` |
| `chat_loop/compress.rs` | 544 | 压缩流水线 7 个函数（含唯一内核 `compress_with_snapshot_core`） |
| `chat_loop/io.rs` | 116 | 落库 / 广播 / 流式占位 / 开会话 / 生命周期钩子 |

**搬移纪律**：语句、注释、调用顺序**逐字保留**，只加模块头、`use` 与可见性；
函数体一行未改（`close_turn` 的 286 行原样搬入 `turn.rs`）。

**保真校验（S3/S4 请照做）**：拆完先做一次「代码行多重集比对」——把旧文件与
「新父文件 + 各子模块」都归一化（去掉 `//!` / `use` / `mod` / 空行，抹平
`pub(crate)` 等可见性前缀），再比较两侧的行多重集。**差异必须能逐条归因**，
本次归因结果：

| 差异 | 条数 | 性质 |
|---|---:|---|
| `super::tokenizer` / `super::compression` / `super::paths` → 加深一层 | 6 | 刻意（多了一层 `chat_loop/`） |
| 两处函数签名被 rustfmt 折行（`persist_messages` / `emit_streaming_start`） | 2 → 展开为 8 | 纯格式 |
| 父模块新增 re-export 块与注释 | 5 | 新增脚手架 |

除上述外**零差异**，即无任何代码行被丢失或改写。

**可见性口径**（本次新确立，供 S3/S4 沿用）：

- **跨模块契约**（`orchestrator.rs` / `resume.rs` 经 `chat_loop::X` 引用）→ `pub`
  + 父模块 `pub use`：仅 `ChatOrchestrator`、`StopSignal`。
- **模块内共享面** → 子模块 `pub(crate)` + 父模块 `pub(crate) use`。
  ⚠️ `pub use` 要求条目为 `pub`，否则报 `E0364/E0365`；`pub(crate) use` 才是正确搭配。
- **跨模块访问的字段**必须逐个标 `pub(crate)` —— Rust 的**字段可见性不随结构体**，
  结构体公开不等于字段可读（本次 `TurnRequest` / `TurnState` / `TurnResult` /
  `TurnInputs` 共 21 个字段）。
- **子模块统一 `use super::*;`**：子模块可看到父模块的私有条目，故父模块的
  `use` 清单与 re-export 就是子模块的共享导入面，无需逐文件重复导入。
- 被搬移代码里的 `super::X::` 需**加深一层**（现在多了一层 `chat_loop/`）：
  本次 5 处（`super::tokenizer` / `super::compression` / `super::paths`）。

### 4.3 S3 实施结果（2026-09-16）

`plugin.rs` **1480 → 523 行**，拆出 2 个子模块（三分见 §3.2）：

| 文件 | 行数 | 内容 |
|---|---:|---|
| `plugin.rs` | 523 | 结构体 / `impl SessionPlugin` / `impl Plugin` / `config_definition` / `now_ms` / 模块声明与 re-export |
| `plugin/nodes.rs` | 501 | 23 个纯函数：路径模型 7 + 节点构造 6 + 消息投影 10 |
| `plugin/vdfs_provider.rs` | 512 | `impl VdfsProvider`（415 行）+ `impl SessionPlugin` 私有辅助（7 个只读函数） |

**归属微调**（与 §3.2 原表的差异，按"内聚"而非"行数"落位）：

| 项 | 原计划 | 实际落位 | 理由 |
|---|---|---|---|
| `title_from_new_path` | `entities.rs` | `nodes.rs` | 它是**新建语义**（`new_types` → 路径名作标题），属 VDFS 路径模型 |
| `config_definition` | `entities.rs` | `plugin.rs` | 配置面，紧邻其唯一调用点 `ConfigFile::new(dir, "会话设置", …)` |
| `now_ms` | `entities.rs` | `plugin.rs` | 通用工具，模块根是它的自然归宿 |
| `entities.rs` | 新建 | **不建** | 三个条目彼此无关，凑成一个 70 行文件反而降低内聚 |

**保真校验**（口径同 §4.2）：旧 `plugin.rs` 994 条归一化代码行 vs. 新三文件 1011 条，
差异**只有 21 条**且逐条可归因——4 条函数签名被 rustfmt 折行（`message_of` /
`overlay_live` / `session_content` / `sub_session_of`，各展开为 4 行）+ 5 条新增的
re-export 块脚手架；注释行差异 30 条全部是新增模块头，**零注释丢失、零代码改写**。

**本次新增的可见性/路径经验**：

- 被父模块 `pub(crate) use` 重导出的条目必须是 `pub(crate)`（§4.2 已记）；
  **只被本模块子级使用**的（如 `now_ms`）保持私有即可——子模块经 `use super::*;`
  可看到父模块的**私有**条目。
- `super::X::` 加深一层：`nodes.rs` 7 处、`vdfs_provider.rs` 12 处
  （`super::workdir::` → `super::super::workdir::`）。
- 父模块 `use` 清单中**只被子模块使用**的导入**不会**触发 `unused_imports`
  ——子模块的 glob 导入算作使用（S2/S3 两次验证）。

**遗留（未做，属"零行为变更"之外）**：`plugin.rs` 的 `now_ms()` 与
`heartbeat.rs` 的 `pub(crate) fn now_ms()` 是**两份等价实现**（前者 `SystemTime`、
后者 `time::OffsetDateTime`）。合并为一处属行为微调，留待单独提交。

**全程硬约束**：

- **零行为变更**：本评审只做"搬家"，不改任何判定、阈值、文案、执行顺序。
- **用例数只增不减**：`cargo test --lib` 基线 544（2026-09-16）。
- 每步 `cargo clippy --all-targets -- -D warnings` 零告警。
- ⚠️ 格式化**必须** `cd symbio && cargo fmt --all`（rust-toolchain.toml 锁 1.93.1 / rustfmt 1.8.0）；
  裸 `rustfmt` 走 default toolchain，版本漂移会产生假差异。

### 4.4 测试文件扁平化（S1b，2026-09-16）

**问题**：S1 把测试外置成 `<module>/tests.rs`，于是每个被测模块多出一个**只放一个测试
文件**的目录——`workdir/`、`compression/`、`chat_session/`…… 全仓 32 个。目录本身不
携带任何信息，反而让"模块"与"目录"两个概念混在一起。

**改法**：`X.rs` + `X.test.rs`（同级扁平）。靠 `#[path]` 属性实现：

```rust
// X.rs 末尾
#[cfg(test)]
#[path = "X.test.rs"]
mod tests;
```

`#[path]` 相对**声明它的文件所在目录**解析，所以测试文件与实现文件同级；而**模块路径
不变**（仍是 `X::tests`），`use super::*;` 的语义逐字不变——这是一次**纯文件搬迁**，
不涉及任何可见性或导入面调整。

**结果**：32 处扁平化，移除 30 个只为放测试而建的目录。两个目录**保留**：
`chat_loop/` 与 `plugin/`——它们另有真实子模块（`compress.rs`/`inputs.rs`/… 、
`nodes.rs`/`vdfs_provider.rs`），目录本身有存在理由。

**例外（`mod.rs` 形态）**：`store/`、`plugins/agent/host/` 的模块文件是 `mod.rs`，
测试就放在 `mod.rs` **同级**的 `tests.rs`——已是正确形态，不动。

**验收**：① 逐文件**行多重集指纹**比对（改名前后必须逐字相同）；② 断言 32 个实现文件
都带上了 `#[path]`；③ `cargo check --tests` 0 错。

**顺带清理**：删除 `plugins/skill/plugin/tests.rs`——一个自初始提交起就**没被任何
`mod` 声明引用**的孤立文件，它引用的 `SkillPlugin::classify_skill_source` /
`get_skill_detail` 在当前代码里**已不存在**（"技能来源分类"功能整体下线），
故那 9 个用例从未运行、也不可能编译。删除后 `skill/plugin/` 目录随之消失。

### 4.5 测试归位到被测试的实现文件（S1c，2026-09-16）

**用户口径**：测试单独一个文件是对的，但**测试代码要和被测试代码在一起**——不能
"测试一处、被测试代码一处"。S1 把测试搬出了实现文件，却让 `plugin/tests.rs` 一个
文件同时服务 `plugin.rs` / `plugin/nodes.rs` / `plugin/vdfs_provider.rs` 三个实现，
`chat_loop` 的三个测试模块同病。**这违反高内聚**。

**改法**：测试文件跟着**它测的那个实现文件**走，一个实现文件对应一个测试文件。

| 实现文件 | 测试文件 | 用例数 |
|---|---|---:|
| `plugin.rs` | `plugin.test.rs` | 6 |
| `plugin/nodes.rs` | `plugin/nodes.test.rs` | 15 |
| `plugin/vdfs_provider.rs` | `plugin/vdfs_provider.test.rs` | 6 |
| `chat_loop.rs` | `chat_loop.test.rs`（`gate_turn` 契约） | 9 |
| `chat_loop/state.rs` | `chat_loop/state.test.rs`（`StopSignal` + `TurnRequest`） | 9 |
| `chat_loop/inputs.rs` | `chat_loop/inputs.test.rs`（`resolve_system_prompt`） | 6 |

删除 `chat_loop/gate_tests.rs` / `chat_loop/stop_signal_tests.rs`（内容已并入上表）。
**用例数零丢失**：`chat_loop` 24（9+9+6）、`plugin` 27（6+15+6）。

---

## 5. 不做的事

- **不改模块间的调用关系**：本评审不引入新抽象、不合并文件、不删除模块。
- **不动 `compression.rs` 的策略划分**（阈值 / 切分 / 视图重建已内聚）。
- **不动 `store/`**（`mod.rs` 形态，测试与 `mod.rs` 同级，已是正确形态）。
- **不动 `close_turn`(286) / `compress_with_snapshot_core`(230) 的内部结构**——
  它们是"单一实现"约束的落点，拆开会让两条压缩入口各自维护流水线（见
  `./core-loop.md` §6）。

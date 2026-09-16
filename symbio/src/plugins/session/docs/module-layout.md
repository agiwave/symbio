# Session 插件模块分工评审

> 状态：**评审 + 执行方案**（S1/S2 已落地；S3–S4 待执行）
> 触发：`chat_loop.rs` 达 2400 行，已到"不该再往单文件里加东西"的程度
> 范围：`symbio/src/plugins/session/`（27 个 .rs，合计 16647 行；**S1 后**生产 12419 + 测试 4313）
> 相关：`./core-loop.md`（核心循环收口，同批次完成）

---

## 0. 一句话结论

分工的**骨架是清楚的**（引擎 / 存储 / 策略 / 执行 / 场景各有其文件），问题出在两点：

1. **测试与实现混放**——全插件 **3713 行（22%）** 是内联 `#[cfg(test)] mod tests`，
   而项目里**已经有两处正确示范**（`chat_session/tests.rs`、`store/tests.rs`）。
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
- **已有正确示范**：`chat_session.rs` 尾部 `#[cfg(test)] mod tests;` + `chat_session/tests.rs`；
  `store/mod.rs` 同理 + `store/tests.rs`。**约定已存在，只是没铺开。**

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
chat_loop/tests.rs      测试                                                            ~410
```

### 3.2 `plugin.rs` → `plugin/`

```text
plugin.rs                SessionPlugin 定义 + impl Plugin（路由 / traverse / 生命周期）   ~600
plugin/vdfs.rs           impl VdfsProvider + 全部 VDFS 节点构造辅助（25 个函数）           ~600
plugin/tests.rs          测试                                                           ~712
```

### 3.3 `orchestrator.rs` → `orchestrator/`

```text
orchestrator.rs          装配层：handle_chat_send_oneoff / resolve_session_params /
                         ChatOrchestrator 装配                                             ~500
orchestrator/consume.rs  消费循环：run_chat_loop_task / persist_failure /
                         broadcast_status / handle_abort                                   ~570
orchestrator/guards.rs   AiControlGuard / WorkingGuard / merge_message_patch /
                         subtree_of                                                        ~300
orchestrator/tests.rs    测试                                                             ~180
```

### 3.4 保持不动

`compression.rs`（806 生产 / 内聚纯函数）、`context_window.rs`、`tool_executor.rs`、
`workdir.rs`、`store/`、`chat_session.rs`、`resume.rs`、`types.rs` 等——
**只外置测试，不改生产划分**。

---

## 4. 执行顺序与验收

| 步 | 内容 | 风险 | 验收 |
|---|---|---|---|
| **S1** ✅ | 测试外置（**21** 个文件 → `<module>/tests.rs`；含 `chat_loop/` 的 3 个测试模块） | 低（纯搬移，`use super::*` 语义不变） | 每步 `cargo check --tests`；末次 `cargo test --lib` 用例数不变 |
| **S2** ✅ | 拆 `chat_loop.rs`（§3.1） | 中（跨模块可见性） | 生产代码总量不变；`cargo test --lib` 用例数不变 |
| **S3** | 拆 `plugin.rs`（§3.2） | 中 | 同上 |
| **S4** | 拆 `orchestrator.rs`（§3.3） | 中高（消费循环是事故敏感区） | 同上 + 消费循环帧合并 / `persist_failure` 作用域逐字不变 |

### 4.1 S1 实施结果（2026-09-16）

| 文件 | 前 | 后（生产） |
|---|---:|---:|
| `chat_loop.rs` | 2401 | 2001 |
| `plugin.rs` | 2192 | 1482 |
| `compression.rs` | 1642 | 809 |
| **全插件** | 16647 总 | **12419 生产 + 4313 测试** |

- 21 个新测试文件：18 个模块级 `<module>/tests.rs` + `chat_loop/{tests,gate_tests,stop_signal_tests}.rs`。
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

**全程硬约束**：

- **零行为变更**：本评审只做"搬家"，不改任何判定、阈值、文案、执行顺序。
- **用例数只增不减**：`cargo test --lib` 基线 544（2026-09-16）。
- 每步 `cargo clippy --all-targets -- -D warnings` 零告警。
- ⚠️ 格式化**必须** `cd symbio && cargo fmt --all`（rust-toolchain.toml 锁 1.93.1 / rustfmt 1.8.0）；
  裸 `rustfmt` 走 default toolchain，版本漂移会产生假差异。

---

## 5. 不做的事

- **不改模块间的调用关系**：本评审不引入新抽象、不合并文件、不删除模块。
- **不动 `compression.rs` 的策略划分**（阈值 / 切分 / 视图重建已内聚）。
- **不动 `store/` 与 `chat_session/`**（已有正确形态，是本次的范本）。
- **不动 `close_turn`(286) / `compress_with_snapshot_core`(230) 的内部结构**——
  它们是"单一实现"约束的落点，拆开会让两条压缩入口各自维护流水线（见
  `./core-loop.md` §6）。

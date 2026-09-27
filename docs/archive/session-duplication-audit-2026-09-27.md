# `session` 插件横向重复定向审计（2026-09-27）

> 本报告是 [`architecture-optimization-review-2026-09-27.md`](architecture-optimization-review-2026-09-27.md)
> §6 的**前置定向审计**。复核报告建议「先做一次『同一语义在 session 内出现几次』的
> 定向审计，有实测数字再决定」——本报告即该审计。
>
> **结论：三处疑似横向重复经核实均为「表面相似，不可提取」**，
> 只有两处极小的去重值得做（合计约 -2 行 + 一处一致性收益）。
> 这与复核报告「本条不急于动手」的判断一致。

---

## 一、`会话目录 → VDFS 目录视图`（复核报告点名的最大嫌疑）

### 实测对照

| 文件 | trait | dispatch 分支 | 结构性转换（树→node/item） | 底层调用 |
|---|---|---|---|---|
| `session/plugin/vdfs_provider.rs`（1103 行） | `VdfsProvider` | 9 个域方法共 879 行 | 3 处 `.map(…_node)`；节点构造**全部委托** `plugin/nodes.rs` | 直读 store + `active_mgr`；**零**子 provider dispatch |
| `agent/host/vdfs.rs`（771 行） | `VdfsProvider` | 9 个操作方法共 650 行 | 0 处纯 map | **10 处**子 provider dispatch（穿透挂载点） |
| `composite/vdfs.rs`（650 行） | `VdfsProvider` | 1 个 dispatch（172 行） | 0 处纯 map | **13 处**子插件 dispatch |

### 三边重复函数对照

| 语义 | session | agent | composite |
|---|---|---|---|
| 路径解析 | `parse_session_path` | `parse_rel_path` | `split_first` + `resolve` |
| 路径拼接 | —（无） | `mount_path` | `child_path` |
| 子树 ctx 改写 | —（域内自洽） | `sub_vfs`（含 `descend_addr`） | `sub_ctx`（含 `descend_addr`） |
| 节点合成 | `nodes_of_sessions` + `nodes.rs` 7 个 fn | `agent_dir_node` / `instruction_node` | `dir_node` / `dir_node_full` |
| watch 包装 | `watch_at_path`（**不补前缀**） | `watch_at`（`map_paths(mount_path)`） | `Watch` 臂（`map_paths(child_path)`） |

### 结论：**不可提取（表面相似）**

判据是**真逻辑差异**：

1. **数据源不同**：session 从 `SessionStore` + `active_mgr` 直读并**合并在途消息**；
   agent 从 `AgentDirStore` 双层目录读；composite 从 `PluginRegistry` 现场收集。
   三者没有可共享的「树」。
2. **转发语义不同**：agent / composite 的核心是**dispatch 转发 + 形状校验**
   （`into_xxx().ok_or_else(mismatch)`，共 23 处）；session 的对应物是 **0 处**。
   可提取的「转发骨架」只覆盖 agent + composite（约 40 行），**不含 session**。
3. **节点来源不同**：session 的节点构造**已经**正确抽到自己的 `plugin/nodes.rs`；
   agent 的两个是它自己域的投影。**不存在**一个已被绕过、本该复用的公共摊平器。
4. 三者共用 trait 与枚举是**协议要求**，不是重复代码。

### 附带发现：`mount_path` ↔ `child_path` 是**真重复但正确保留**

- `agent/host/vdfs.rs:115` 的 `mount_path` 与 `composite/vdfs.rs:109` 的 `child_path`
  是**逐字节同构**（`(empty,empty)/(empty,x)/(x,empty)/(x,x)` 四臂 return）；
- agent 侧注释**自承认**「与 `composite::child_path` 同义（这里不复用，避免在插件间引入依赖）」。

**应保持现状**：合并需下沉到 core，而这是仅 10 行的函数——
按 `plugin-entry-audit` E-009（插件互引禁止）本就不该互引，
为 10 行在 core 增加一个公开面（还要过 ADR-023 依赖方判据）是净负收益。

---

## 二、transcript 域内的「帧投递」

### 每文件职责

| 文件 | 职责 |
|---|---|
| `transcript.rs`（533） | 内存图状态机：`apply` 合并帧、分配在途 seq、分骨架/细节打日志、`deliver` 合帧、**`publish` 投递（唯一出口）** |
| `transcript/inbox.rs`（272） | 收件箱队列：入队/取消/清空/出队 + 常驻消费者 |
| `transcript/deliver.rs`（220） | 纯机械：`DELIVER_WINDOW_MS`、`PendingDelta`、`DeltaLogCoalescer`、`frame_log_of`、`render_frame_line` |
| `transcript/frames.rs`（58） | 状态帧/删除帧构造与发射 |

### 同形投递逻辑清单

| 逻辑 | 位置 | 行数 | 是否同形 |
|---|---|---|---|
| `VdfsChange` 构造 + `notify` | `transcript.rs` `publish` / `emit_session_state` | 6 + 26 | 同形 |
| `VdfsChange` 构造 + `notify` | `inbox.rs` 4 处 | 4 处 | **不同**：全是条目地址，语义是「入队/取消/出队」 |
| 帧计数器自增 | `transcript.rs` 两处 `self.frame_no += 1` | 各 1 | 轻微同形，**异义** |
| 冲刷待投递 | `flush_pending`（7 处调用点） | 6 | **已收敛为单函数** ✅ |

### `frames.rs` 是否名副其实？——**是，真复用**

`llm_emit_state` / `llm_emit_removed` / `llm_state_frame` 有 **7 个文件**引用
（`chat_loop.rs` 及 `chat_loop/{io,state,turn}.rs`、`resume.rs`、`tool_executor.rs`、
`pipeline.rs`），是**跨子模块的公共门面**。58 行里约 30 行是必要文档
（ADR-023/038 的下沉理由），实质代码约 20 行。**保留原样。**

### 结论：**不可提取**

- `transcript.rs::publish` 是**唯一出口**（模块头明写 `changes.notify` 只在这里被调用）；
- `inbox.rs` 的 4 处 `notify` 是**另一个实体**（队列条目 vs 消息节点）：
  地址函数、生命周期、载荷形状全不同。并进 `publish` 会引入
  「同一函数处理两种地址族」的**假抽象**；
- 唯一**真**横向重复是 `frame_no += 1` 两处（各 1 行），但二者分属不同帧类别
  （消息帧 vs 会话状态帧）且各自紧跟不同日志格式，抽出收益为负。**不动。**

---

## 三、`context/pipeline.rs` 与 `context/window.rs` 的分工

### 边界

| 维度 | `pipeline.rs`（874） | `window.rs`（490） |
|---|---|---|
| 性质 | **执行层**（唯一有副作用） | **纯策略层**（零副作用、纯函数） |
| 核心 | `compress_snapshot_inner` + `compress_with_snapshot_core` | `apply_layered_sliding_window`（工具调用骨架化） |
| 预算用途 | 判断**要不要压缩** | 决定**摘要多长** |
| token 估算 | 调 `super::estimate_message_tokens` | 调 `tokenizer::truncate_tokens` |
| 自有常量 | 无（全在 `context.rs`） | 4 个 |

### 是否有同名预算/截断常量（横向重复信号）

| 常量 | 位置 | 值 | 重复？ |
|---|---|---|---|
| `MESSAGE_TOKEN_CAP` | `context.rs:44` | 2048 | 与 `view.rs::FADE_BUDGET` 同值，**但语义不同**（消息上限 vs 工具结果预算） |
| `SKELETON_DIGEST_TOKEN_CAP` | `window.rs:22` | 64 | 独占 |
| `JSON_DIGEST_PARSE_LIMIT` | `window.rs:108` | 64 KiB | 独占 |
| `COMPRESSION_TOKEN_THRESHOLD` | `context.rs:38` | 0.7 | 独占 |
| **硬编码 `truncate_tokens(_, 24)`** | `window.rs:132/136/294` | 24 | **3 处硬编码同一魔法数（真重复）** |

### README 分层 ↔ 代码对照

> 说明：`session/README.md` 的决策表实际只定义了 **L0 / L1 / L2 + 三个未编号层**，
> **不存在 L3–L6**（复核报告里的「L0-L6」是转述时的放大）。按真实出现的标记核对：

| README 分层 | 代码侧文件 | 吻合 |
|---|---|---|
| **L0** 工具结果守卫 | `tools/tool_result_guard.rs` | ✅ |
| 内容节点淡化（未编号） | `context/view.rs::fade_aged_content_nodes` | ✅ |
| 工具淡化/骨架化（未编号） | `context/window.rs::apply_layered_sliding_window` | ✅ |
| **L1** 自动压缩（70%） | 判定 `context.rs::should_start_compression` + 执行 `pipeline.rs::auto_compress_process` | ✅（判定/执行分居两文件，README 未写明这点） |
| **L2** 语义压缩落库 | 判定 `context.rs` + 执行 `pipeline.rs::compress_snapshot_inner` | ✅ |
| prune / FIFO（未编号） | `chat_session/write.rs` | ✅ |

**代码侧与分层一一对应**，无错位。

### 结论：**不可提取**

一为执行层（有副作用、要回滚、要发节点），一为纯策略（无副作用、可单测）。
README 的决策表**已明确**把它们分在「需智能不可逆 → 落库」vs
「纯机械可重放 → 视图层」两侧——这是**有意的架构分界**，
合并会破坏「视图层幂等重算」这一不变量。

共享的 `estimate_message_tokens` 已收在 `context.rs`（单一出处）；
`2048` 在两处**同值但异义**——按判据**不应合并**（合并后改一处会误伤另一处）。

---

## 四、可动候选（按收益排序）

| # | 做什么 | 实测依据 | 预估 | 风险 |
|---|---|---|---|---|
| 1 | `window.rs` 提取 `ENTRY_NAME_TOKEN_CAP = 24`，替换 3 处字面量 | `grep` 命中 `window.rs:132/136/294` | 净减 2 行（收益在一致性） | 极低 |
| 2 | `agent::mount_path` ↔ `composite::child_path` 合并 | 两函数逐字节同构 | ~10 行 | **中：倾向不做**（跨插件禁引 E-009；下沉 core 增公开面） |
| 3 | `transcript.rs` 两处 `frame_no += 1` | 异义、各 1 行 | 0 | **明确不合并** |
| 4 | 三 VDFS provider 的 dispatch 面 | 数据源/转发语义/节点来源三者皆异，共享函数 = 0 | 0 | **不建议**——会造出「只覆盖三分之一调用方」的假抽象 |

---

## 五、本报告刻意标注的「看起来像重复但正确保留」

以下四处在**只看行数**的工具（含 LLM 的直觉）下会被报为重复，逐一核对后**均应保持现状**：

| 项 | 表面相似 | 实际 |
|---|---|---|
| `mount_path` ↔ `child_path` | 逐字节同构 | 跨插件边界，E-009 禁互引；合并需下沉 core 增公开面 |
| `inbox::notify` ↔ `transcript::publish` | 都构造 `VdfsChange` + notify | **不同实体**（队列条目 vs 消息节点），地址族/生命周期/载荷全不同 |
| `MESSAGE_TOKEN_CAP` ↔ `FADE_BUDGET` | 同为 2048 | **同值异义**（消息上限 vs 工具结果预算），合并会误伤 |
| 两处 `frame_no += 1` | 同一自增模式 | 分属消息帧与会话状态帧，各 1 行 |

**这正是「定向审计」的价值**：四处在没有实测数字时都是「疑似重复」，
核实后**全部是正确保留**。

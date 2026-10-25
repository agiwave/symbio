---
kind: design
id: DE-104-module-layout
title: Session / Loop 插件模块分工
status: active
created: 2026-09-16
impl: done
---

# Session / Loop 插件模块分工

> 状态：**现行规范**（域划分、判据与落点）
> 范围：`symbio/src/plugins/session/`（存储与生命周期，**留守层**）与
> `symbio/src/plugins/loop/`（单环驱动器，[D-086](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 切出的独立插件）
> 判据的「为什么」与被否决的方案见 [D-039](../../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")
> （`session` 按域重组）与 [D-086](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
> （驱动器成片、跨墙通货闭族）；文件清单与行数属**结构事实**，由代码生成，
> 见 [当前事实表](../../../../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表")，本文件不抄。
> 相关：[`./015-core-loop.md`](102-core-loop.md "DE-102-core-loop")（主循环结构与收口点）、
> [`../../loop/README.md`](../../loop/README.md "symbio/src/plugins/loop/README")（驱动器插件）、
> [`../README.md`](../README.md "symbio/src/plugins/session/README")（现行机制与配置面）

---

## 0. 五个判据

| # | 判据 | 它挡住什么 |
|---|---|---|
| 1 | **一域一目录（或一文件）**：一个职责域 = 一个 `X/` 目录（`mod.rs` + 子模块）或一个 `X.rs` | 多类互不相干的职责同居一个文件——「文件」这层表达不出边界 |
| 2 | **编排层与领域层分离**：编排层只做装配、调度与收口，领域层内聚自己的机制；依赖单向（编排 → 领域） | 领域机制被编排逻辑切碎，改一个机制要穿过接线代码 |
| 3 | **副作用出口唯一**：领域对外的落库 / 广播 / 事件经该域的单一出口 | 同一件事有多个写入口，守卫看不见旁路 |
| 4 | **域内分层，测试恒外置**：生产与测试分文件，一个实现文件对应一个测试文件 | 读实现要在大段测试里翻找生产函数 |
| 5 | **跨域共享面走门面**：跨域共享符号以 `pub` + 域根 `pub use` 导出；`pub(super)` 只表达**域内部分工** | 跨域判据被 `pub(super)` 私藏，调用方被迫绕道或复制一份 |

论证（为什么是这五条、否决了哪些切法）在 D-039；本文件只回答**现在怎么分、
新代码落在哪**。两者互不复述。

---

## 1. 分层与落点

### 1.1 编排层（留守：轮生命周期与状态出口）

| 落点 | 职责 |
|---|---|
| `mod.rs` | 模块声明与公共面收敛 |
| `plugin.rs` + `plugin/{nodes,vdfs_provider,words}.rs` | 插件装配：路由分发与**退役路由的防回潮断言**、`traverse` 能力收集、VDFS 会话节点树与路径模型、会话域协议词表 |
| `orchestrator.rs` + `orchestrator/{entry,consume,runner,failure,sink,rate_limit,stop,consolidate}.rs` | 会话侧编排与**一轮的生命周期**：`TurnLauncher`（`runner.rs`）装配交付包并派发 `loop/<sid>` 的 `run_turn`，`consume.rs` 等待驱动器动作返回并按结局收尾（看门狗 / 接管 / 失败）；`Convergence`（`failure.rs` / `convergence.rs`）做状态广播与失败收口；`stop.rs` 保证 Stop 钩子恰好一次；`consolidate.rs` 是压缩快照的持令牌落格口（动作 `session/<sid>/_run/consolidate`）；`sink.rs` 与 `rate_limit.rs` 是执行期事件出口与发送限流。**主循环本体已不在本插件**（D-086 §8） |
| `convergence.rs` | 广播出口：会话运行态的唯一发射器与可恢复错误的收敛出口（3c-1 把存储句柄收成 `Convergence`，6b-③ 随留守层归位到插件根） |

### 1.2 领域层（留守：存储与生命周期）

| 域 | 落点 | 职责 |
|---|---|---|
| 会话读写 | `chat_session.rs` + `chat_session/{read,write}.rs` | 读路径：三层清理与 `User` 轮次对齐的上下文装配；写路径：保存边界（轮数对齐裁剪 / 历史工具链裁剪 / 归档配对清理） |
| 变更入口 | `commands.rs` | 会话与消息的**写语义**：新建 / 覆盖 / 删除会话，改写 / 截断 / 清空消息。VDFS（`write` / `delete` / `action`）与编排入口都只是它的调用方——「谁能改会话」的答案只在这一处（模块头说明它为什么仍在 `impl SessionPlugin` 上） |
| 转写 | `transcript.rs` + `transcript/{frames,deliver,publish,inbox,supplements,autonomous}.rs` | 消息级变更的**唯一写入点与发射器**、帧构造 / 日志合并 / 合帧窗口（`deliver.rs`）、**落库回包的唯一出口**（`publish.rs`——「谁落库、谁发帧」那条 §3.4 硬不变量，原先住在 `plugin.rs` 里，被切的两片因此反过来引用汇编层，归位理由见该文件头）、状态帧与删除帧出口、收件箱入队与消费挑选、轮边界把 n 条补充并成**一条** `User` 消息，且「取批 + 合并 + 成形」的整体同步单入口 `take_supplement_batch_sync` 也住在本域（`supplements.rs`——存储裁剪按 User 轮次计数，逐条追加会多吃掉保留预算；单入口归位见 [D-081](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 裁决三）、core 执行期转写写柄的桥 `compression_writer`（`transcript.rs`——驱动器不再把 `Arc<Mutex<Transcript>>` 持作字段，经本域构造的写柄落格，裁决一第 3 条）、自主开轮的落格窗口（`autonomous.rs`，写臂识别 `meta.trigger` 后经它开这一轮） |
| 持久化 | `store/mod.rs` | 一种磁盘布局、两种驻留方式；没有可切换的存储后端 |
| 运行态 | `active.rs` + `runtime.rs` | `active.rs`：进程内运行态注册表（待消费请求、请求 id、中止信号、变更订阅）；`runtime.rs`：上一轮结局从生命周期事实折出的**投影**（D-083 §5.2） |
| 能力与选项收集 | `capabilities.rs` | `collect_capabilities`（工具）与 `collect_options`（选项字段）两个 traverse 收集器，同处一地 |
| 心跳 | **不住本插件**：`plugins/heartbeat` | 空闲扫描与触发调度（15 秒节拍、错峰、每 tick 上限）连同它的设置工具与选项行是一个**独立插件**（落点判据 [D-077](../../heartbeat/README.md "symbio/src/plugins/heartbeat/README")）。本插件与它相接的只有两处：收件箱写臂识别 `meta.trigger`（`plugin/nodes.rs::trigger_declaration`）与自主开轮的落格窗口（`transcript/autonomous.rs`） |
| 选项 | `options/mod.rs` | 会话自有选项的字段声明（`build_option_definition`；收集器在 `capabilities.rs`） |
| 工作目录 | `workdir/{mod,fs_watcher}.rs` | 会话工作目录的 VDFS 实现（`<根>/session/<id>/workdir/<rel>`）与其下的文件系统监听 |
| 会话记忆 | `memory.rs` | 本会话自己的 `MEMORY.md`：落位、地址、两道闸门与系统提示词注册段（机制在 `providers/memory`） |
| 消息变更事实 | `message_events.rs` | 「编辑 / 截断」入格的写方（D-053）：`commands::patch_message` / `commands::truncate_messages` 经唯一写入口落事实，重建读路径不会让旧内容复活 |
| 时间上下文 | `temporal.rs` | 挂在用户消息 `prompt` 上的 `<context>` 段（唯一调用点是本插件落库路径） |
| 统计读数 | `stats.rs` | `session/stats` 的只读投影出口 |
| worker 进展 | `worker_progress.rs` | Q3（worker 进展）的**唯一归属**：`WorkerProgress` 类型、旧落库读法（`progress_of`）与 WAL 读法（`progress_from_wal`）、逐子会话编排的存储侧单入口 `for_session`——驱动器只调这一个入口，「枚举 + 加载 + 复核 + 读法」这条链不在驱动器侧自拼（[D-081](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 裁决一第 4 条） |

### 1.3 支撑单件（一文件一职责，留守）

`config.rs`（配置真源）· `paths.rs`（地址 ↔ 目录映射）·
`types.rs`（`Session` 实体与共享类型）。

> 「一文件一职责」不等于「文件越碎越好」：单消费方的小件落在**消费方所在域**
> （`rate_limit.rs` 在 `orchestrator/`、`fs_watcher.rs` 在 `workdir/`），只有被多个
> 域共享、或本身独立成域的支撑件才留在根级。

### 1.4 驱动器片（`symbio/src/plugins/loop/`，独立插件）

| 层 | 落点 | 职责 |
|---|---|---|
| 入口 | `plugin.rs` · `provider.rs` · `mod.rs` | 插件的唯一入口：`loop/<sid>` 的 `Action("run_turn")`（授权三条见 `plugin.rs` 模块文档）。**没有 `route` 臂、没有能力行、没有常驻任务**——它贡献 1 条 Mount 行，停用即 `NotFound` |
| 跨墙面 | `session_port.rs` · `turn_session.rs` | 数据面经 `TurnDispatch`（`TurnSpec` 纯值 + 闭族对象）；会话侧读写只走地址面 `session/<sid>/_run/<op>`（`SessionPort`），不 import 会话模块（E-009 / E-013） |
| 主循环 | `chat_loop.rs` + `chat_loop/{state,inputs,turn,compose,delegate,drive}.rs` · `loop_rows.rs` · `head_exec.rs` · `report_exec.rs` · `stage_events.rs` · `turn_runner.rs` · `turn_exec.rs` | 环骨架、状态与契约、输入准备、**行表**、轮首 host 的绑定表与派发点、批次边界 host 的绑定表与派发点、阶段事实、执行引擎与批次边界产物。`loop_rows.rs` 每行 = `Decl` 数据（`owes` / `until` / `unless` + 全局 `prio`）；行注册在**自己 host 的绑定表**上（D-159），派发点向 `ring::drive` 传由该表派生的行子集，结论由引擎现读；声明与视图在 `symbio_core::decl`（DE-015） |
| 上下文与请求 | `context.rs` + `context/{pipeline,view,prompt}.rs` · `message_build.rs` · `tokenizer.rs` · `model_chat.rs` | 压缩 / 注入 / 请求视图、助手轮装配、Token 估算与头尾切分、Model 推理请求协议 |
| 工具与分派 | `tools.rs` + `tools/{tool_executor,tool_result_guard}.rs` · `tool_dispatch.rs` | 模型侧工具从分发到结果处理的全链路、L0 结果守卫与滚动存档 |
| 事实与记忆 | `derived_facts.rs` · `memory_events.rs` · `skill_events.rs` · `task_events.rs` · `resume.rs` | 轮末派生事实（承诺 / 任务表 / 熔断 + 记忆三段 + 技能观测）经会话动作面落格；恢复与历史重写 |

> 两片的连接只有三条：**触发**（session → `loop/<sid>` `run_turn`）、**回程**（动作返回 `TurnReport`）、
> **会话侧读写**（loop → `session/<sid>/_run/<op>`）。任何第四条连接都要先改 D-086。

> 依赖方向是**规范不是机制**——本仓没有判定它的审计，由评审把持：编排层可以
> 引用领域层，领域层不得反向引用编排层（`plugin.rs` / `orchestrator/` 即会话侧
> 编排层；驱动器侧的 `plugin.rs` / `chat_loop.rs` 同理）。跨插件引用规则由
> [`scripts/plugin-entry-audit.mjs`](../../../../../scripts/plugin-entry-audit.mjs)
> 的 **E-009**（不引兄弟插件）与 **E-013**（loop 的 import 根 ⊆ D-083 §4 闭族）
> 执行；内核深引是 `core-export-audit` 的 **C-002**——原 E-010 已迁走，避免两个脚本
> 各判一遍（[`009-plugin-route-address.md`](../../../../../docs/003-design/006-插件路由与地址.md "DE-006-插件路由与地址")
> 只收地址类规则 E-001 ~ E-007）。

**存量豁免（登记于此，只减不增）**——会话留守层的领域层当前仍引用的编排层符号及
归位方向。评审新 PR 时对照此表：表外的领域层 → 编排层引用即违例。

| 文件 | 符号 | 归位方向 |
|---|---|---|
| `transcript.rs` | `message_path` | 归位 paths 域（寻址规则同源） |
| `transcript/inbox.rs` | `inbox_item_node`、`inbox_item_path` | inbox 域自持（条目的地址与节点视图） |
| `transcript/inbox.test.rs` | `SessionConfig` | 真源在 `config.rs`（plugin 只是 re-export），测试直引真源即消 |

`context/pipeline.rs` 与 `resume.rs` 的两行随 6b-③ 成片搬出而消（文件已不在本插件）；
`SessionPlugin` **类型本身**不入表：inherent impl 分布在领域层文件是全仓惯例
（`commands.rs` / `options` 皆如此），见 `symbio/src/plugins/mod.rs`
的架构原则。

### 1.5 跨片共享词表（住 core，不住本插件）

会话切成驱动器（`loop` 片）与留守两片时，隔着 E-009 插件墙**两头都要用**的
**词汇**归位 core（九组反向引用逐条定归属、被否决的方案见
[D-082](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 裁决三）。两片都不再持有
这些定义，只经根别名取用：

| core 落点 | 装什么 | 取用方（留守 / 驱动器） |
|---|---|---|
| `symbio_core::schemas::session::node_build` | ChatNode 的**写侧构造与建树整形**：`dialog_node`、`llm_build_*`、`PresentationTree` 与 `detach_if_flat` / `flatten_whole_tree` / `turn_root_of` | 留守：`chat_session/read` · `plugin/{nodes,vdfs_provider}` · `config`（档名解析）· `orchestrator/failure`；驱动器：`chat_loop/{io,compose}` · `message_build` · `resume` · `report_exec` · `tool_dispatch` · `tools/tool_executor` · `turn_exec` · `turn_runner` |
| `symbio_core::schemas::session::facts` | 跨端口的**事实记录类型**：`Delegation`、`TaskDeclaration` / `TaskItem` / `TaskStatus`、`ReportDraft` | 写侧驱动器 `tools/tool_executor/{delegation,task_decl}` 的 `note_*` 出参 · 落侧 `derived_facts` · 读侧 `task_events` · `tool_dispatch` · `report_exec` |
| `symbio_core::text::merge_kernel` | 巩固合并的**纯计算内核**（截断 / 分句 / 合并 + 保真度，只依赖 `std`，verify 程序以 `#[path]` 共有同一份源码） | 驱动器 `memory_events`（续一条原名短路径，调用点不动）· `skill_events` |
| `symbio_core::request_principal` | 「本请求以谁的身份跑轮」的**唯一取值点**（core 既有键 `AGENT_ID` 的纯派生函数，不住任何片） | 驱动器 `chat_loop` / `chat_loop/state` · `chat_loop/inputs` · `context/pipeline`；留守 `stats` |

判据与 `types::Session` 一族同一条：**词汇的归属随数据 owner，跨插件者上移**。
**机制留在原处**——`note_*` 的门与声明位（`records_delegation_promise` /
`owns_task_table`）、`RoundHookFn` 的回调形态、`llm_build_assistant_messages` 的
一轮装配，都不在词表里。

---

## 2. 测试布局

两种合法形态，由 [`test-layout-audit.mjs`](../../../../../scripts/test-layout-audit.mjs)
判定（接入 `gate.mjs`，棘轮只降不升）：

```text
X.rs + X.test.rs 实现文件与测试同级，宿主末尾：
 #[cfg(test)] #[path = "X.test.rs"] mod tests;

X/mod.rs + X/tests.rs 模块文件是 mod.rs 的（如 store/、workdir/）：
 测试放同级 tests.rs，宿主末尾 #[cfg(test)] mod tests;
```

- **一个实现文件对应一个测试文件**：测试跟着它测的实现走，不许一个测试文件
 同时测几个实现，也不许几个测试文件测同一个实现。
- **测试跟着可观察行为走，不跟着文件走**：域内子模块若只是**纯机械**（类型与
 纯函数，没有独立于域根的对外行为，如 `transcript/deliver.rs`），其行为由**域
 边界测试文件**覆盖、不另立测试文件——另立就得把域根的测试夹具复制一份，
 两份夹具才是真的漂移点。判据是「这个子模块对外表现出过**独立的**行为吗」。
- `#[path]` / 同级 `tests.rs` **不改变模块路径**（仍是 `X::tests`），故
 `use super::*;` 的语义与内联 `mod tests { … }` 完全一致——拆分与搬移是纯文件操作。
- 新增测试**不写进生产文件**：棘轮看的是生产文件里的测试函数数，加一个就红。

---

## 3. 可见性与共享面

- **域根即门面**：跨域要拿的东西在域根（`X.rs` / `X/mod.rs`）以 `pub use` 汇总，
 调用方只认域根，不深入子模块（先例：`tool_dispatch.rs` 取 `tools` 域根的
 `process_tool_calls_async` / `is_unanswered`，`options` 对 `capabilities::collect_options` 的引用）。
- **`pub use` 要求条目为 `pub`**：跨子模块的共享面用 `pub(crate) use` 搭配
 `pub(crate)` 条目，写反了会报 `E0364 / E0365`。
- **字段可见性不随结构体**：跨模块访问的字段逐个标注，结构体公开不等于字段可读。
- **子模块统一 `use super::*;`**：父模块的 `use` 清单与门面就是子模块的共享导入面，
 无需逐文件重复导入（只被子模块使用的导入也不触发 `unused_imports`）。
- **`super::` 随目录层级加深**：搬进子目录的代码，`super::X` 指向本域根而非插件根；
 模块路径不变的搬移（`X.rs` → `X/mod.rs`）**不加深**——判据是模块路径，不是文件系统。
- **`pub(super)` 只表达域内部分工**：跨域共享的判据不得用它私藏（判据 5）。

---

## 4. 纯搬移如何自证没丢东西

域重组的每一步都是**零行为变更的纯搬移**，自证口径四条（每阶段提交前全绿才算数）：

1. **代码行多重集比对**：旧路径与新路径两侧归一化后逐行比对，差异**逐条归因**
 （路径加深、折行、门面调用、可见性标注、脚手架注释），不接受「说不清的增删」；
2. **必须在 `cargo fmt --all` 之后复测**：路径变长会触发 rustfmt 折行，
 「逐字未改」的结论在格式化后要重验一遍；
3. **用例数不减**：`cargo test --lib` 基线只升不降；
4. **门禁全绿**：`node scripts/commit.mjs` 自跑 fmt / check / clippy / test / 审计，
 任一红即中止提交。

---

## 5. 边界（不做的事）

- **不动机制语义**：分工只回答「代码落在哪」，不改判定、阈值、文案、执行顺序。
- **不动单点结构**：`close_turn` 与压缩内核的内部结构是「单一实现」约束的落点，
 拆开会让两条入口各自维护一套流水线——见 [`./015-core-loop.md`](102-core-loop.md "DE-102-core-loop")。
- **不顺手收窄 / 放宽可见性**：搬移阶段只做归位；可见性调整是独立决策，
 要有跨域共享面或域内分工的判据支撑（§3）。
- **不抄结构快照**：文件数、行数、代码规模一律以
 [当前事实表](../../../../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表") 为准；本文件若与它冲突，以它为准。

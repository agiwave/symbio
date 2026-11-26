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
> 范围：`crates/symbio-plugin-session/src/`（存储与生命周期，**留守层**）与
> `crates/symbio-plugin-loop/src/`（单环驱动器，独立插件）
> 划分的现行边界见 [DE-018-模块架构](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")；
> 文件清单与行数属**结构事实**，由代码生成，
> 见 [当前事实表](../../../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表")，本文件不抄。
> 相关：[`../../loop/README.md`](../../../symbio-plugin-loop/README.md "crates/symbio-plugin-loop/README")（环的形状与收口点，主循环的现行 owner）、
> [`../README.md`](../../README.md "crates/symbio-plugin-session/README")（现行机制与配置面）

---

## 0. 五个判据

| # | 判据 | 它挡住什么 |
|---|---|---|
| 1 | **一域一目录（或一文件）**：一个职责域 = 一个 `X/` 目录（`mod.rs` + 子模块）或一个 `X.rs` | 多类互不相干的职责同居一个文件——「文件」这层表达不出边界 |
| 2 | **编排层与领域层分离**：编排层只做装配、调度与收口，领域层内聚自己的机制；依赖单向（编排 → 领域） | 领域机制被编排逻辑切碎，改一个机制要穿过接线代码 |
| 3 | **副作用出口唯一**：领域对外的落库 / 广播 / 事件经该域的单一出口 | 同一件事有多个写入口，守卫看不见旁路 |
| 4 | **域内分层，测试恒外置**：生产与测试分文件，一个实现文件对应一个测试文件 | 读实现要在大段测试里翻找生产函数 |
| 5 | **跨域共享面走门面**：跨域共享符号以 `pub` + 域根 `pub use` 导出；`pub(super)` 只表达**域内部分工** | 跨域判据被 `pub(super)` 私藏，调用方被迫绕道或复制一份 |

五条判据的论证与边界见 [DE-018-模块架构](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")；本文件只回答**现在怎么分、
新代码落在哪**。两者互不复述。

---

## 1. 分层与落点

### 1.1 编排层（留守：轮生命周期与状态出口）

| 落点 | 职责 |
|---|---|
| `mod.rs` | 模块声明与公共面收敛 |
| `plugin.rs` + `plugin/{nodes,vdfs_provider,words}.rs` | 插件装配：路由分发与**退役路由的防回潮断言**、`traverse` 能力收集、VDFS 会话节点树与路径模型、会话域协议词表 |
| `orchestrator.rs` + `orchestrator/{entry,consume,runner,failure,sink,rate_limit,stop,consolidate}.rs` | 会话侧编排与**一轮的生命周期**：`TurnLauncher`（`runner.rs`）装配交付包并派发 `loop/<sid>` 的 `run_turn`，`consume.rs` 等待驱动器动作返回并按结局收尾（看门狗 / 接管 / 失败）；`Convergence`（`failure.rs` / `convergence.rs`）做状态广播与失败收口；`stop.rs` 保证 Stop 钩子恰好一次；`consolidate.rs` 是压缩快照的持令牌落格口（动作 `session/<sid>/_run/consolidate`）；`sink.rs` 与 `rate_limit.rs` 是执行期事件出口与发送限流。**主循环本体已不在本插件** |
| `convergence.rs` | 广播出口：会话运行态的唯一发射器与可恢复错误的收敛出口（3c-1 把存储句柄收成 `Convergence`，6b-③ 随留守层归位到插件根） |

### 1.2 领域层（留守：存储与生命周期）

| 域 | 落点 | 职责 |
|---|---|---|
| 会话读写 | `chat_session.rs` + `chat_session/{read,write}.rs` | 读路径：三层清理与 `User` 轮次对齐的上下文装配；写路径：保存边界（轮数对齐裁剪 / 历史工具链裁剪 / 归档配对清理） |
| 变更入口 | `commands.rs` | 会话与消息的**写语义**：新建 / 覆盖 / 删除会话，改写 / 截断 / 清空消息。VDFS（`write` / `delete` / `action`）与编排入口都只是它的调用方——「谁能改会话」的答案只在这一处（模块头说明它为什么仍在 `impl SessionPlugin` 上） |
| 转写 | `transcript.rs` + `transcript/{frames,deliver,publish,inbox,supplements,autonomous}.rs` | 消息级变更的**唯一写入点与发射器**、帧构造 / 日志合并 / 合帧窗口（`deliver.rs`）、**落库回包的唯一出口**（`publish.rs`——「谁落库、谁发帧」那条 §3.4 硬不变量，原先住在 `plugin.rs` 里，被切的两片因此反过来引用汇编层，归位理由见该文件头）、状态帧与删除帧出口、收件箱入队与消费挑选、轮边界把 n 条补充并成**一条** `User` 消息，且「取批 + 合并 + 成形」的整体同步单入口 `take_supplement_batch_sync` 也住在本域（`supplements.rs`——存储裁剪按 User 轮次计数，逐条追加会多吃掉保留预算；单入口归位见 [DE-018-模块架构](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")），core 执行期转写写柄的桥 `compression_writer`（`transcript.rs`——驱动器不再把 `Arc<Mutex<Transcript>>` 持作字段，经本域构造的写柄落格）、自主开轮的落格窗口（`autonomous.rs`，写臂识别 `meta.trigger` 后经它开这一轮） |
| 持久化 | `store/mod.rs` | 一种磁盘布局、一种驻留方式（落盘）；没有可切换的存储后端 |
| 运行态 | `active.rs` + `runtime.rs` | `active.rs`：进程内运行态注册表（待消费请求、请求 id、中止信号、变更订阅）；`runtime.rs`：上一轮结局从生命周期事实折出的**投影**（[DE-001-核心架构](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") §5.2） |
| 能力与选项收集 | `capabilities.rs` | `collect_capabilities`（工具）与 `collect_options`（选项字段）两个 traverse 收集器，同处一地 |
| 心跳 | **不住本插件**：宿主注入的调度能力 | 空闲扫描与触发调度不在本插件；本插件只提供两处交接面：收件箱写臂识别 `meta.trigger`（`plugin/nodes.rs::trigger_declaration`）与自主开轮的落格窗口（`transcript/autonomous.rs`） |
| 选项 | `options/mod.rs` | 会话自有选项的字段声明（`build_option_definition`；收集器在 `capabilities.rs`） |
| 工作目录 | `workdir/{mod,fs_watcher}.rs` | 会话工作目录的 VDFS 实现（`<根>/session/<id>/workdir/<rel>`）与其下的文件系统监听 |
| 消息变更事实 | `message_events.rs` | 「编辑 / 截断」入格的写方：`commands::patch_message` / `commands::truncate_messages` 经唯一写入口落事实，重建读路径不会让旧内容复活 |
| 统计读数 | `stats.rs` | `session/stats` 的只读投影出口 |
| worker 进展 | `worker_progress.rs` | Q3（worker 进展）的**唯一归属**：`WorkerProgress` 类型、旧落库读法（`progress_of`）与 WAL 读法（`progress_from_wal`）、逐子会话编排的存储侧单入口 `for_session`——驱动器只调这一个入口，「枚举 + 加载 + 复核 + 读法」这条链不在驱动器侧自拼 |

### 1.3 支撑单件（一文件一职责，留守）

`config.rs`（配置真源）· `paths.rs`（地址 ↔ 目录映射）·
`types.rs`（`Session` 实体与共享类型）。

> 「一文件一职责」不等于「文件越碎越好」：单消费方的小件落在**消费方所在域**
> （`rate_limit.rs` 在 `orchestrator/`、`fs_watcher.rs` 在 `workdir/`），只有被多个
> 域共享、或本身独立成域的支撑件才留在根级。

### 1.4 驱动器片（`crates/symbio-plugin-loop/src/`，独立插件）

| 层 | 落点 | 职责 |
|---|---|---|
| 入口 | `plugin.rs` · `provider.rs` · `mod.rs` | 插件的唯一入口：`loop/<sid>` 的 `Action("run_turn")`（授权三条见 `plugin.rs` 模块文档）。**没有 `route` 臂、没有能力行、没有常驻任务**——它贡献 1 条 Mount 行，停用即 `NotFound` |
| 跨墙面 | `session_port.rs` | 数据面经 `TurnDispatch`（`TurnSpec` 纯值 + 闭族对象）；会话侧读写只走 `port=session` 的 `session-config` / `session-facts` / `session-call` / `session-call-unit` 四行，不 import 会话模块（E-009 / E-013） |
| 主循环 | `chat_loop.rs` + `chat_loop/{state,inputs,turn,compose,delegate,drive}.rs` · `loop_rows.rs` · `report_exec.rs` · `stage_events.rs` · `turn_runner.rs` · `turn_exec.rs` | 环骨架、状态与契约、输入准备、**行表**、汇报的落点与能力口、阶段事实、执行引擎与单环派发器。`loop_rows.rs` 每行 = `Decl` 数据（`owes` / `until` / `unless` + 全局 `prio`）；**只有一张绑定表**，派发器 `turn_exec::drive` 向 `ring::drive` 传全表，结论由引擎现读；声明与视图在 `symbio_platform::kernel::decl`（DE-051） |
| 上下文与请求 | `context.rs` + `context/{pipeline,view,prompt}.rs` · `message_build.rs` · `tokenizer.rs` · `model_chat.rs` | 压缩 / 注入 / 请求视图、助手轮装配、Token 估算与头尾切分、Model 推理请求协议 |
| 工具与分派 | `tools.rs` + `tools/{tool_executor,tool_result_guard}.rs` · `tool_dispatch.rs` | 模型侧工具从分发到结果处理的全链路、L0 结果守卫与滚动存档 |
| 事实与记忆 | `derived_facts.rs` · `memory_events.rs` · `task_events.rs` · `resume.rs` | 轮末派生事实（承诺 / 任务表 / 熔断 + 记忆三段）经会话动作面落格；恢复与历史重写。技能观测与校准随技能能力住 `symbio-plugin-skill`（见 [DE-041-技能编译与自我改进](../../../../docs/003-design/041-技能编译与自我改进.md "DE-041-技能编译与自我改进")） |

> 两片的连接只有三条：**触发**（session → `loop/<sid>` `run_turn`）、**回程**（动作返回 `TurnReport`）、
> **会话侧读写**（loop → `session/<sid>/_run/<op>`）。任何第四条连接都要先完成 [DE-018-模块架构](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构") 的边界评审。

> 依赖方向是**规范不是机制**——本仓没有判定它的审计，由评审把持：编排层可以
> 引用领域层，领域层不得反向引用编排层（`plugin.rs` / `orchestrator/` 即会话侧
> 编排层；驱动器侧的 `plugin.rs` / `chat_loop.rs` 同理）。跨插件引用规则由
> [`scripts/audits/plugin-entry-audit.mjs`](../../../../scripts/audits/plugin-entry-audit.mjs)
> 的 **E-009**（不引兄弟插件）与 **E-013**（loop 的 import 根 ⊆ [DE-001-核心架构](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") §4 闭族）
> 执行；内核深引是 `core-export-audit` 的 **C-002**——原 E-010 已迁走，避免两个脚本
> 各判一遍（[`009-plugin-route-address.md`](../../../../docs/003-design/006-插件路由与地址.md "DE-006-插件路由与地址")
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
（`commands.rs` / `options` 皆如此），没有「一个类型一个 impl 文件」的集中处。

### 1.5 共享词表与本插件投影

跨插件共享的参数表住平台 SDK；会话私有的实现与同源测试住本插件 `src/projections/`。
归属按生产消费方分布确定，不按目录名保留第二份实现。

| 落点 | 装什么 | 取用方 |
|---|---|---|
| `symbio_platform::sdk::projection::vocab` | `ChatVocab` / `MessageVocab` / `CommitmentVocab` 与 `ChatSlot` 派生 | actors、session、门面及投影实现 |
| `symbio_platform::sdk::round::node_build` | 内核规则 `result_node_id` 的既有转发路径 | `symbio-plugin-local` 经门面 |
| `crate::projections` | 转写、分支、变更、省略、汇报记录、节点构造及测试期保真度判据 | 本插件生产代码与测试 |

任务词汇面 `TaskVocab` 随唯一生产消费方归 `symbio-plugin-task::decl`，不再是会话共享词表。
任务声明、派发机制与行为登记分别归 loop owner 和平台 SDK；它们不属于会话投影。
`prompt_fidelity` 没有生产消费方，只在测试构建中存在。

---

## 2. 测试布局

两种合法形态，由 [`test-layout-audit.mjs`](../../../../scripts/audits/test-layout-audit.mjs)
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
- **不动单点结构**：`persist_turn` 与压缩内核的内部结构是「单一实现」约束的落点，
 拆开会让两条入口各自维护一套流水线——见 [`../../loop/README.md`](../../../symbio-plugin-loop/README.md "crates/symbio-plugin-loop/README") 的环体一节。
- **不顺手收窄 / 放宽可见性**：搬移阶段只做归位；可见性调整是独立评审，
 要有跨域共享面或域内分工的判据支撑（§3）。
- **不抄结构快照**：文件数、行数、代码规模一律以
 [当前事实表](../../../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表") 为准；本文件若与它冲突，以它为准。

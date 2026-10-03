# Session 插件模块分工

> 状态：**现行规范**（域划分、判据与落点）
> 范围：`symbio/src/plugins/session/`
> 判据的「为什么」与被否决的方案见 [ADR-039](../../../../../docs/DECISIONS.md)
> （`session` 按域重组）；文件清单与行数属**结构事实**，由代码生成，
> 见 [CURRENT.md](../../../../../docs/CURRENT.md)，本文件不抄。
> 相关：[`./core-loop.md`](./core-loop.md)（主循环结构与收口点）、
> [`../README.md`](../README.md)（现行机制与配置面）

---

## 0. 五个判据

| # | 判据 | 它挡住什么 |
|---|---|---|
| 1 | **一域一目录（或一文件）**：一个职责域 = 一个 `X/` 目录（`mod.rs` + 子模块）或一个 `X.rs` | 多类互不相干的职责同居一个文件——「文件」这层表达不出边界 |
| 2 | **编排层与领域层分离**：编排层只做装配、调度与收口，领域层内聚自己的机制；依赖单向（编排 → 领域） | 领域机制被编排逻辑切碎，改一个机制要穿过接线代码 |
| 3 | **副作用出口唯一**：领域对外的落库 / 广播 / 事件经该域的单一出口 | 同一件事有多个写入口，守卫看不见旁路 |
| 4 | **域内分层，测试恒外置**：生产与测试分文件，一个实现文件对应一个测试文件 | 读实现要在大段测试里翻找生产函数 |
| 5 | **跨域共享面走门面**：跨域共享符号以 `pub` + 域根 `pub use` 导出；`pub(super)` 只表达**域内部分工** | 跨域判据被 `pub(super)` 私藏，调用方被迫绕道或复制一份 |

论证（为什么是这五条、否决了哪些切法）在 ADR-039；本文件只回答**现在怎么分、
新代码落在哪**。两者互不复述。

---

## 1. 分层与落点

### 1.1 编排层

| 落点 | 职责 |
|---|---|
| `mod.rs` | 模块声明与公共面收敛 |
| `plugin.rs` + `plugin/{nodes,vdfs_provider,words}.rs` | 插件装配：路由分发与**退役路由的防回潮断言**、`traverse` 能力收集、VDFS 会话节点树与路径模型、会话域协议词表 |
| `orchestrator.rs` + `orchestrator/{entry,consume,broadcast,failure,sink,rate_limit}.rs` | 会话编排：两个 one-off 入口与自动命名、Turn 消费循环、状态广播唯一出口、失败收口、执行期事件落地、发送限流（唯一消费方是 `consume`） |
| `chat_loop.rs` + `chat_loop/{state,inputs,turn,io}.rs` | 主循环骨架、状态与契约、输入准备与压缩响应、单轮尾结算、循环内的落库与广播出口 |

### 1.2 领域层

| 域 | 落点 | 职责 |
|---|---|---|
| 上下文治理 | `context.rs` + `context/{pipeline,view,window,prompt}.rs` | 「发给大模型之前」的全部处理：压缩策略与执行流水线（压缩内核 `compress_with_snapshot_core` 的唯一落点是 `context/pipeline.rs`）、请求视图四步剪裁、工具骨架化、压缩提示词协议 |
| 会话读写 | `chat_session.rs` + `chat_session/{read,write}.rs` | 读路径：三层清理与 `User` 轮次对齐的上下文装配；写路径：保存边界（轮数对齐裁剪 / 历史工具链裁剪 / 归档配对清理） |
| 变更入口 | `commands.rs` | 会话与消息的**写语义**：新建 / 覆盖 / 删除会话，改写 / 截断 / 清空消息。VDFS（`write` / `delete` / `action`）与编排入口都只是它的调用方——「谁能改会话」的答案只在这一处（模块头说明它为什么仍在 `impl SessionPlugin` 上） |
| 转写 | `transcript.rs` + `transcript/{frames,deliver,inbox}.rs` | 消息级变更的**唯一写入点与发射器**、帧构造 / 日志合并 / 合帧窗口（`deliver.rs`）、状态帧与删除帧出口、收件箱入队与消费挑选 |
| 持久化 | `store/mod.rs` | 一种磁盘布局、两种驻留方式；没有可切换的存储后端 |
| 工具执行 | `tools.rs` + `tools/{tool_executor,tool_result_guard,heartbeat_tool}.rs` | 模型侧工具从分发到结果处理的全链路、L0 结果守卫与滚动存档、心跳任务工具；域根 `tools.rs` 是对编排层的门面 |
| 能力与选项收集 | `capabilities.rs` | `collect_capabilities`（工具）与 `collect_options`（选项字段）两个 traverse 收集器，同处一地 |
| 心跳 | `heartbeat/mod.rs` | 空闲扫描与触发调度（15 秒节拍、错峰与每 tick 上限） |
| 选项 | `options/mod.rs` | 会话自有选项的字段声明（`build_option_definition`；收集器在 `capabilities.rs`） |
| 工作目录 | `workdir/{mod,fs_watcher}.rs` | 会话工作目录的 VDFS 实现（`<根>/session/<id>/workdir/<rel>`）与其下的文件系统监听 |
| 会话记忆 | `memory.rs` | 本会话自己的 `MEMORY.md`：落位、地址、两道闸门与系统提示词注册段（机制在 `providers/memory`） |
| 请求构造 | `message_build.rs` | 发给 LLM 的请求消息数组与工具结果节点构造，及挂在用户消息 `prompt` 上的时间上下文 |
| 会话恢复 | `resume.rs` | 会话恢复与历史重写 |
| 运行态 | `active.rs` | 进程内运行态注册表：待消费请求、请求 id、中止信号、变更订阅 |
| v2 事实桥 | `v2_bridge.rs` | `finish_turn` 收束点把每轮事实（用户发言 / 助手答复 / 实测耗时）转写进 per-session 的 v2 WAL——v1 行为零变化，纯增量记录；受 `v2_mode` 总开关管辖（`off` 不转写；`full` 档待引擎切换落地时增设）——[ADR-045](../../../../../docs/decisions/core.md) |

### 1.3 支撑单件（一文件一职责）

`config.rs`（配置真源）· `paths.rs`（地址 ↔ 目录映射）·
`tokenizer.rs`（Token 估算与头尾切分——切分由计量驱动，签名里就有 `&dyn Tokenizer`）·
`model_chat.rs`（Model 推理请求协议结构）·
`types.rs`（`Session` 实体与共享类型）。

> 「一文件一职责」不等于「文件越碎越好」：单消费方的小件落在**消费方所在域**
> （`rate_limit.rs` 在 `orchestrator/`、`fs_watcher.rs` 在 `workdir/`），只有被多个
> 域共享、或本身独立成域的支撑件才留在根级。

> 依赖方向是**规范不是机制**——本仓没有判定它的审计，由评审把持：编排层可以
> 引用领域层，领域层不得反向引用编排层（`plugin.rs` / `orchestrator/` /
> `chat_loop/` 即编排层）。跨插件引用规则（E-009；内核深引是
> `core-export-audit` 的 **C-002**——原 E-010 已迁走，避免两个脚本各判一遍）定义在
> [`scripts/plugin-entry-audit.mjs`](../../../../../scripts/plugin-entry-audit.mjs)
> 与 [`scripts/core-export-audit.mjs`](../../../../../scripts/core-export-audit.mjs)
> 的规则表（各是自己那批规则的唯一定义处；
> [`plugin-route-address.md`](../../../../../docs/design/plugin-route-address.md)
> 只收地址类规则 E-001 ~ E-007）。

**存量豁免（登记于此，只减不增）**——领域层当前仍引用的编排层符号及归位方向。
评审新 PR 时对照此表：表外的领域层 → 编排层引用即违例。

| 文件 | 符号 | 归位方向 |
|---|---|---|
| `context/pipeline.rs` | `append_and_publish`、`ChatOrchestrator`、`SessionContext` | 落库回包出口并入 commands 域；压缩流水线与主循环的循环依赖改签名 |
| `resume.rs` | `ChatOrchestrator` | 恢复入口由编排层注入回调，领域层不再命名编排类型 |
| `transcript.rs` | `message_path` | 归位 paths 域（寻址规则同源） |
| `transcript/inbox.rs` | `inbox_item_node`、`inbox_item_path` | inbox 域自持（条目的地址与节点视图） |
| `transcript/inbox.test.rs` | `SessionConfig` | 真源在 `config.rs`（plugin 只是 re-export），测试直引真源即消 |

`SessionPlugin` **类型本身**不入表：inherent impl 分布在领域层文件是全仓惯例
（`commands.rs` / `heartbeat` / `options` 皆如此），见 `symbio/src/plugins/mod.rs`
的架构原则。

---

## 2. 测试布局

两种合法形态，由 [`test-layout-audit.mjs`](../../../../../scripts/test-layout-audit.mjs)
判定（接入 `gate.mjs`，棘轮只降不升）：

```text
X.rs + X.test.rs           实现文件与测试同级，宿主末尾：
                           #[cfg(test)] #[path = "X.test.rs"] mod tests;

X/mod.rs + X/tests.rs      模块文件是 mod.rs 的（如 store/、heartbeat/）：
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
  调用方只认域根，不深入子模块（先例：`tools.rs` 的 `execute_tool_async` /
  `summarize_*`，`options` 对 `capabilities::collect_options` 的引用）。
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
  拆开会让两条入口各自维护一套流水线——见 [`./core-loop.md`](./core-loop.md)。
- **不顺手收窄 / 放宽可见性**：搬移阶段只做归位；可见性调整是独立决策，
  要有跨域共享面或域内分工的判据支撑（§3）。
- **不抄结构快照**：文件数、行数、代码规模一律以
  [CURRENT.md](../../../../../docs/CURRENT.md) 为准；本文件若与它冲突，以它为准。

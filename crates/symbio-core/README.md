---
kind: design
id: crates/symbio-core/README
title: symbio_core —— 内核契约层：命名与结构规范
status: active
created: 2026-09-26
lists: [core-domains=owner, core-modules=owner]
impl: done
---

# symbio_core —— 内核契约层：命名与结构规范

本目录是**内核契约层 + 跨插件共享内核**：放跨插件共享的 trait、协议类型、词表常量、
纯工具函数，以及**被多个模块消费的共享机制**（如 `event_bus` 的全局订阅表、`logger`
的结构化日志与级别闸门）。判据只有一个——**依赖方数量**：只被一个模块依赖的内容一律下沉回该
模块，不论它「够不够底层」（见 [D-023](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。**「依赖方」指生产模块，
测试不算**（口径与例外见 §4 第 1 问）。
本文是该目录**命名与结构**的唯一 owner。

封闭语法（五个语法 trait 与签名类型）的实现住 `symbio-kernel` crate（[DE-023 D-188](../../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性")），
本目录以同名私有 shim `pub use symbio_kernel::…` 转出——对外路径 `symbio_core::<符号>` 不变。
本目录**仍然拥有**不属于封闭语法的部分：共享机制（`event_bus` / `logger`）与读本机部署表的行为
（`authz` / `governance`、`ActorSpec` 的授权读法自由函数、VDFS 地址拼接）。

> 「不放实现」是**方向**而非字面事实：域里既有纯契约（`vdfs` / `capability` / `llm`
> 的 trait 面），也有共享实现（`event_bus` 的全局订阅表、`logger` 的进程级闸门）。
> 判据是「谁依赖它」，不是「它抽象不抽象」——把共享机制硬抽成 trait
> 只会多一层间接（见 [D-035](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。

各域职责、域清单与豁免词表在下面的 §1–§3；单个域的机制写在它的 `mod.rs` 文档头里
（本文不复述）。

## 1. 域模型（四条标准）

一个**域** = 本目录下一层目录，对应一个职责上自洽的概念面。四条标准同时成立才叫合规：

1. **一域一目录，域内私有**：域内子模块一律 `mod x;`（私有），公开面在该域 `mod.rs` 里
 逐符号 `pub use` 列出。域的公开面因此**可在一处读完**——要改对外形状只动这一屏。
 子模块可以深到 2–3 层（如 `llm/turn.rs`、`plugin/dir.rs`），但**没有**「域内再分域」的
 特权：判断标准是「它有没有独立的对外面」，不是「文件大不大」。
2. **符号带域前缀**：类型与常量的名字必须让**调用点自证归属**。理由不是好看，而是
 本层符号在全仓按根平铺导入（见第 4 条）——`EventSink` 这种名字在 `plugins/model/*`
 里读起来无从知道它属于谁。

 **默认手段**是域名前缀，三类符号都要：

 | 符号 | 形态 | 例 |
 |---|---|---|
 | 类型 / trait | 域名（PascalCase）+ 语义 | `ExecEventSink` · `VdfsNode` · `PluginDir` · `CapabilityMeta` |
 | 常量 / 静态量 | 域名（SCREAMING_SNAKE）+ 语义 | `VDFS_ACTION_ABORT` · `EVENT_BUS_KIND_VDFS` · `PLUGIN_KEY_ENABLED` |
 | 函数 / 自由函数 | 域名（snake_case）+ 语义 | `clock_now_ms` · `vdfs_notify_change` · `capability_resolve` · `creator_create_object` |

 **函数也要前缀，理由与类型 / 常量完全相同**：本层符号在全仓按根平铺导入（第 4 条），
 导入行里**没有**域路径——`use crate::symbio_core::{vdfs_host_ctx, vdfs_notify_change}`
 读不出它们属于谁。前缀**只写一次**：原名里已经出现**完整的域词**（词级，不是子串）
 就把它去掉再拼前缀（`init_logger` → `logger_init`、`invoke_capability` →
 `capability_invoke`、`has_creator` → `creator_has`）；否则直接拼（`create_object` →
 `creator_create_object`）。判据始终是「名字以 `<域名>_` 开头」，可机械核对。

 **三条被登记的替代手段**——它们与域名前缀**同等正式**，不是「例外」：

 | 手段 | 何时用 | 实例 |
 |---|---|---|
 | **子命名空间** | 域内按主题分了文件，且主题名比域名更有信息量 | `llm`：`Model*`（`model_provider.rs`）· `Turn*`（`turn.rs`）。`capability`：`Configurable*`（`configurable.rs`）· `Option*`（`option.rs`）· `Tool*`（`tool_name.rs`）· `Approval*`（`grants.rs`：审批策略的两级——工具授权清单与放行依据），外加**模块**形态的 `failure_kind`。`plugin`：`TRAVERSE_*`（`traverse.rs`） |
 | **后缀** | 该域的类型名有一个比域名更强的类别词 | `keys` 的 `…Key`（`PathKey`）——`KeyPath` 会读成「键的路径」，语义反了 |
 | **登记缩写** | 域名的大写形式冗余且不增加信息 | `logger` → `LOG_`（`LOG_LEVEL_INFO` 已足够定位） |

 子命名空间有两种形态：**类型前缀**（`Model*` / `Detail*`）与**模块**（`failure_kind`）。
 模块是**命名空间，不是符号**——它的名字是主题，前缀规则对它不适用。`core-naming-audit`
 识别模块并跳过；跳过时**计数**，因为静默跳过会让「公开面里有什么」出现看不见的口子。

 判据始终是「**调用点读不读得出归属**」，不是「字面是否等于目录名」：

 - `ChangeSubscriptions` 住在 `vdfs` 却既不姓 `Vdfs` 也不属于任何已登记主题 ⇒
 改名 `VdfsChangeSubscriptions`；
 - `KEY_PROVIDER` 住在 `plugin` 却用着 `keys` 域的前缀 ⇒ 改名 `PLUGIN_KEY_PROVIDER`；<!-- doc-symbol-allow: 历史：改名前的旧名，实名见下一行的 PLUGIN_KEY_PROVIDER -->
 - 插件工厂 id 描述的是「哪个插件」而非「哪个键」⇒ 归**装配方**的 id 表
 （`crates/crates/crates/assembly.rs`），命名 `PLUGIN_ID_*`——内核不认识具体插件名；
 - 反例：`LOG_LEVEL_INFO` **不**需要冠上 `LOGGER_` 前缀——多出来的那点长度不增加任何信息。

 **一域一前缀，且前缀不跨域复用。** 两个域共用同一个前缀就是「一个前缀两种东西」——
 同一个 `PLUGIN_` 既指清单键又指插件身份，读的人得先分辨是哪一个。故 `PLUGIN_`
 只属于 `plugin`（清单键靠 `PLUGIN_KEY_*` 细分段区分）；插件工厂 id 属装配事实，
 前缀仍叫 `PLUGIN_ID_*`，但已整体移出内核（见 `assembly` 模块）。
 `KEY_` 不属于 `keys`（该域没有字符串常量，见 §3）。

 **放置也是同一条规则的一部分**：符号住哪个域由「它描述什么」决定，不由「谁先用了它」
 决定。路由地址归 `crate::routes`、遍历端点归 `plugin::traverse`、
 嵌入服务 id 归 `embedding::ids`——它们都曾住在 `keys`；插件工厂 id 归**装配方**
 （`crates/crates/crates/assembly.rs`，部署事实，不进内核）。**上下文键同理，且判据更细**：
 键跟着**值类型**走（键与值同住，[D-095](../../docs/003-design/005-协议规范.md "DE-005-协议规范")）——
 `keys` 只留机制（`SymbioKey` trait）与值类型为通用词汇（`String` / `Value` /
 `Vec<String>`）的实例；值类型属某个域的，实例搬去那个域（`EVENT_SINK` / `ABORT_SIGNAL`
 → `exec`，`PLUGIN_DIR` / `PLUGIN_PARENT` → `plugin`，三条收集通道键与
 `CAPABILITY_ERRORS` → `capability`）。
3. **依赖单向、只依赖更底层**：域之间只允许「上层 → 下层」，不得互相回指。
 **本条是可复核的读数，不是一句宣称**：用 [`core-vocab-audit`](../../scripts/core-vocab-audit.mjs)
 实测（剥注释、按根出口归属建图、Tarjan 求强连通分量），代码层**现有 1 族环**：

 | 环族 | 根因 | 为什么现在不拆 |
 |---|---|---|
 | `adapters ⇄ projection` | `fallback` 投影读适配层的档位统计，适配层又按它的时延档表产出 | 拆法是「档位词表下沉一层」，属 `adapters` 自己的改造 |

 复核方式：`node scripts/core-vocab-audit.mjs`，看「依赖环族」一行与 V-002。基线按
 **环族**做棘轮：新增一族即红，只许降。2026-10-19 首次建图时是 2 族（上面那族加一族
 八域「万能胶」环）；2026-10-20 按 [D-095](../../docs/003-design/005-协议规范.md "DE-005-协议规范") 三刀
 拆掉第二族（边数 44 → 40，环族 2 → 1）——被拆掉的那族与三刀刀法登记在该脚本
 `DEFAULT_BASELINE` 的注释里，作为「环会长回来」的反例。

 插件专属的实现代码不进本层
 （如 `llm` 只留契约，HTTP 重试与 SSE 流循环留在 `plugins/model/`）。**宿主接缝的统一
 形态是「纯接口 + 一个桥文件」**：接口与域类型宿主无关，只有 `host.rs` 知道本宿主的
 信封类型（`vdfs/host.rs` 是范本），换宿主只重写那一个文件。
4. **一个出口（根平铺重导出，且只导"跨出 core 的"）**：**core 外**的消费方一律写
 `symbio_core::<符号>`——域 `mod.rs` 的公开面由根 [`lib.rs`](src/lib.rs) 逐条平铺重导出。
 **只在 core 内部用的符号不出根**：根 `pub use` 里没有它，core 内自己走域内路径
 `symbio_core::<域>::<符号>`（否则「没人用的东西占着公共面」会一直长）；只被测试或
 无人使用的存量，按 [`dead-code-audit`](../../scripts/dead-code-audit.mjs) R-002
 **逐项承认**（`// dead-code-allow R-002: <理由>`，理由必填、数量棘轮只许降）。
 **根 `lib.rs` 里域目录一律 `mod <域>;`（私有）**——
 子目录不开模块门，谁出现在公共面上由根 `pub use` 逐条声明。协议词按符号
 （`Verdict` / `DetailField`…）与**叶子模块**（`chat_node` / `node_build` / `facts`…）
 在根导出；**任何深路径**（`symbio_core::plugin::dir::` / `symbio_core::chat::chat_node::`
 之类）视为不规范，应改为根平铺。叶子模块只有一层，且名字是**主题**（见 §1.2 `chat` /
 `turn` 两行）——它不是「域的内部件也能被引用」的豁免。
 **可执行判据**：[`core-export-audit.mjs`](../../scripts/core-export-audit.mjs)——
 C-001（根出口唯一）/ C-002（无深引）/ C-003（**内核公共面**的符号必须被 ≥2 个模块消费，
 0 消费方基线已归零、单消费方存量走棘轮，豁免写 §4 四问的理由）/ C-004（内核之外的根
 账本条数冻结只许降——C-003 收窄后由它拦住「共享类型随手进根出口」，DE-023 D-186）。

**封闭语法另有一个汇合口 [`kernel.rs`](src/kernel.rs)**（DE-023 A2）：Event 信封 / Entity×Verb /
`ActorSpec` 五字段 / `Pattern`·`Scope` / 声明容器与 `seal` / 环七步 / 五个封闭语法 trait
只经它重导出，根用 `pub use kernel::*` 平铺（对外路径不变）。封闭语法的实现与
CS-001 / CS-004 的**判定范围**（`core-kernel-files:` 标记）都在 `crates/symbio-kernel/src/lib.rs`；
`kernel.rs` 只剩转出。域行为不进这个文件。
它不是域——没有目录、不进 §1.2 表。

### 域前缀对照表（全 28 域 —— 新增符号照此取名）

这张表是第 2 条的**唯一执行口径**：拿不准新符号叫什么，先在这里查它的域。
「—」= 该域没有这一类符号（不是「不用前缀」）。**一个前缀只属于一个域。**
「裸名」= 不要求前缀（见 §3 的 `keys` 域规则）。

**「跨 crate 内部词」** = 拆 crate（D-191）之后必须出根、但**不按域前缀取名**的那一批。
它们原本是 `pub(crate) use`——只在本 crate 内跨模块可见，因此从未进过这张表，也从未
受「符号带域前缀」约束。拆 crate 后跨 crate 只剩**一条**通路（根 `lib.rs` 的 `pub use`，
C-001），这层内部面只能跟着升为 `pub`，名字却还是内部助手时的名字（`lock_read` /
`join_addr` / `accept`），硬加域前缀只换来一堆读不出信息的改名。

所以这批名字**登记在所属域的那一格**，与其它名字一样逐个列出：登记是显式动作，
把新符号塞进这一格不会比塞进 `…*` 前缀更容易——N-001 只认这张表，表里没有就红。

<!-- core-naming:modifiers Dyn,Default -->

<!-- list:core-domains owner -->

| 域 | 类型 / trait | 常量 / 静态量 | 函数 / 自由函数 | 备注 |
|---|---|---|---|---|
| `authz` | `EffectiveMatrix`（部署表 + 撤销覆盖层的合成结果） | `PRINCIPAL_`（主体 id）· `CAP_`（能力名）· `SESSION_OWNER` | `bind_` · `unbind_` · `principal_` · `request_principal` · `content_visible` · `production_matrix` | 本机部署的**授权策略表**（谁持有什么能力 + 哪些能力已被淘汰裁决收回）：表与判定机制同住 `governance` 旁边（[D-099](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")）。整域 `pub(crate)`——表里写的是**部署事实**而不是契约，跨插件可见、不进对外公开面；前缀行只登记归属，不参与根公开面核对 |
| `decl` | `FactDecl` · `ViewDecl` · `ActorDecl` · `PluginDeclarations` · `Builder` · `Frozen` | — | `declare` · `seal` | 事实 / 视图**声明层**：`FactDecl` 答「这个事实有哪些字段」，`ViewDecl` 答「这个视图读了哪些字段、算一次多少成本」。事件名与格位的声明（`EventDecl`）住 `event` 域——它答的是网格登记而不是载荷形状。与 `event::grid` 的 `CELLS` 并存（一个管格位、一个管字段）。内核只声明协议五格；控制面事实与全部视图由 `plugins::loop::control_decl` 声明，经 `Builder::declare` 一次交齐 |
| `adapters` | 契约词 `LatencyTier` · `RuleOnly` · `FullModel` · `TokenIssuer` · `LlmAdapter` · `LlmTurn` · `AdapterError` · `StubLlmAdapter` · `ProviderLlmAdapter` · `DeltaSink` · `SilentDeltas` | — | — | `StubLlmAdapter` 是 `#[cfg(test)]` 项（只有测试构建里存在：零 LLM 桩，**没有生产接入位**，故不参与接线清偿）；其余全部是 [plan/DE-018 §3.3](../../docs/003-design/018-模块架构.md "DE-018-模块架构") 注入策略与 [plan/DE-001 §10](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 四层时延表的冻结契约名（出处 [`verify/latency_gate.rs`](../../docs/003-design/verify/latency_gate.rs)），名字先于模块存在——判据同冻结契约词（§3）。`LlmTurn` 是 [plan/PL-008 §2](../symbio-plugin-session/src/docs/102-core-loop.md "DE-102-core-loop") 的生成端口产物；工具分发契约与结果形状已归 loop 插件本地（[D-118](../../docs/003-design/015-义务环.md "DE-015-义务环")），不在本域公开面 |
| `actors` | `Actor*`（`ActorSpec`）；契约词 `Pattern` · `Scope` · `Decider`（模式名）· `CommitmentKeeper` · `EliminationDecider` | `SKILL_TAG`（loop 写、session 读的共享词汇） | `commitment_events` · `commitment_records` · `actor_can_*` · `pattern_for_principal` · `trivial_actor_spec` | [plan/DE-001 §4](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的冻结契约名（名字先于模块存在，判据同冻结契约词（§3））；`Decider` 是 [plan/DE-018 §4](../../docs/003-design/018-模块架构.md "DE-018-模块架构") S8 反射档判定者的**模式名**（`Pattern::Decider`）——规则应答的语义由 `classify` 规则表承接。**单消费方的策略实现归 owner 插件**（D-189 / D-190）：调度在 `plugins/loop/task_events`、技能编译与路由在 `plugins/loop/skill_events`、检索 Translator 在 `plugins/loop/memory_events`、自主层与意欲闸门在 `plugins/session/transcript/autonomous`、消息渲染在 `plugins/loop/turn_runner::prepare`、抢占判定在 `plugins/session/transcript/inbox`、外部执行闸门在 `plugins/loop/tools/dispatch/gates`；本域只留 commitment（写侧 / 读侧配对）与 control（只剩淘汰裁决——写侧在 loop、读侧翻译在 session，D-190）两类策略。**`TurnRunner` / `TurnOutcome` / `TurnInput` / `TurnResume` / `RoundInjector` 已于 2026-10-09 下沉 `plugins/session`**（单生产消费方，见 §4 第 1 问与 [D-023](../../docs/003-design/001-核心架构.md "DE-001-核心架构")） |
| `agent` | `Agent*`（`AgentPermissions` 声明 · `AgentScope` 作用域） | `AGENT_` | `agent_perms_` | Agent 包的**权限声明与它的执行方**（[DE-011 §5.3](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范") / [D-153](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")）：四字段 grant 语义（列了才能用、空清单即拒）、glob 只管自己的包、装载期限额拒载。写侧只有 agent 插件（解析 manifest），读侧是 web / vdfs 两族工具，互不相认 ⇒ 共享设施住 core。**键与值同住**（[D-095](../../docs/003-design/005-协议规范.md "DE-005-协议规范") 根因一）：`AGENT_PERMS` 的值是 `Arc<OnceLock<AgentScope>>`（本域类型）。glob 的匹配与合法性、作用域的发布都**不出根**（经 `AgentPermissions::path_allowed` / `check_globs` 与 `AgentScope::publish` 给出）——单消费方的自由函数不占根出口（§4 第 1 问） |
| `capability` | `Capability` · `Registry`（收集口契约，put/get/list 三方法——DE-008 §5 4f 的纯重命名） · `CapabilityRow` · `CapabilityFamily`（族值：名字 + 槽位策略） · 模块 `family`（内置六族常量）；子命名空间 `Configurable*` · `Option*` · `Tool*` · `Approval*` | 裸名（收集通道键实例） | `capability_` · `tool_metas` | 无常量字面量；三条收集通道键（`CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR`，值类型 `Arc<dyn Registry>`）与错误桶键 `CAPABILITY_ERRORS`（值带 `CapabilityError`）与值同住本域（键与值同住，[D-095](../../docs/003-design/005-协议规范.md "DE-005-协议规范") 根因一）。三个子命名空间各有对应文件。`CapabilityRow*` 是收集口三方法（put/get/list）的**行词汇**（`row.rs`，DE-008 §3）：一张表装下全部收集通道，行载荷是开放类型——新族由声明方给 `CapabilityFamily` 常量与载荷类型，内核零改动（[D-175](../../docs/003-design/021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分")） |
| `chat` | 叶子模块 `chat_node` · `principal` · `chat_request` · `session_chat`（四个**模块命名空间**，符号走模块路径，不出根） | — | — | [D-087](../../docs/003-design/001-核心架构.md "DE-001-核心架构")：会话线格式域——一条消息、一个主体长什么样。子模块形态是 §1.2 登记的「主题命名空间」：节点家族四十余个名字全部平铺进根出口会把内核公开面撑宽一倍，而它们只在同一个语义面里出现。**本域不依赖任何其它域**（只依赖 serde），`llm` / `turn` / `projection` 都指向它 |
| `clock` | — | — | `clock_` | 只有一个函数 |
| `creator` | 跨 crate 内部词 `ObjectConstructor` · `Submit` | — | `creator_` | 通用对象创建注册表：按 id 装配**任意**类型对象（`ObjectConstructor` / `Submit` 是 `submit_object_creator!` 宏展开的落点，宏跨 crate 使用 ⇒ 必须出根），见 §2 |
| `dialog` | 契约词 `Verdict` · `DecideRequest` · `DecideOutcome`（含 `DecisionPlan`）· `ParseVerdictRequest` · `ComposeRequest` · `WordingReply` · `RunSnapshot` | `REASON_` | — | [D-087](../../docs/003-design/001-核心架构.md "DE-001-核心架构")：对话面控制契约（session ⇄ `classify` / `compose`）。一条判据：**判决是枚举、措辞是文本**——`Verdict` 要被编排层执行，`ComposeRequest` 的产出只被展示。五个类型名是 [plan/DE-001](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的冻结契约词（改名 = 代码与契约漂移）；`DecideOutcome` / `ParseVerdictRequest` 是 [D-122](../../docs/003-design/015-义务环.md "DE-015-义务环") 决策 2 新增的两面契约（`classify/decide` 的判决结果 = 判完或待跑的计划；`classify/parse` 把自由文本解析回判决）；`REASON_*` 是判决的**理由码**闭集，与 `Verdict` 同进退 |
| `embedding` | `Embedding` | `EMBEDDING_` | — | 服务 id 在 `embedding/ids.rs` |
| `event` | `Event` / `EventEnvelope` / `EventDecl`（名字 → 格位）；契约词 `Seq` · `Entity` · `Verb` · `Timestamp` · `ClosureReason` | `EVENT_` | `event_` | 契约词来自 [plan/DE-001 §2](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的冻结契约文本，名字先于模块存在（判据同冻结契约词：改名 = 代码与契约漂移）。见 [D-043](../../docs/003-design/001-核心架构.md "DE-001-核心架构")。常量 = 事件**名字表**（名字是数据，单点定义）。函数 = 载荷里的**节点身份**规则（`event_payload_node_id` / `event_produced_text_node_id` / `event_tool_result_node_id` / `event_node_of`，[D-053](../symbio-plugin-session/README.md "crates/symbio-plugin-session/README")）：写侧与三个读侧出口（`rebuild` / `transcript` / `prompt_fidelity`）必须给出**同一个**「这条事件承载哪个节点」的答案，各写一遍的后果是静默的（`message.edited` 定位不上要改的节点 = 「编辑没生效」而无人报错） |
| `event_bus` | `EventBus` | `EVENT_BUS_` | `event_bus_` | |
| `exec` | `Exec` | 裸名（执行期通道键实例） | — | 本域无常量字面量；两条执行期通道键（`EVENT_SINK` / `ABORT_SIGNAL`）与值类型同住本域（键与值同住，[D-095](../../docs/003-design/005-协议规范.md "DE-005-协议规范") 根因一）——`keys` 域自此只留机制与通用词汇键 |
| `governance` | 契约词 `PermissionMatrix` · `PrincipalPolicy` · `PairingViolation` · `VisScope` · 跨 crate 内部词 `GrantsOverlay`；**Capability（能力闭集）刻意不进根平铺** | — | — | [plan/DE-001 §7](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的冻结契约名。governance 的 Capability（权限矩阵的键）与 `capability` 域的 `Capability`（LLM 工具描述符）是**两个概念**、计划文本同名——以路径限定消歧：两个 Capability 同时出现在调用点正是命名纪律要阻止的错位，故本域不把它登记为前缀 |
| `invariants` | `Violation` | — | **裸名**（不变量名） | 函数名即不变量的可执行名（N1/N3/N5 的 C1/C2/C3 形态），谓词名比域名更有信息量——判据同冻结契约词（§3）：名字先于模块存在 |
| `keys` | `…Key`（**后缀**） | **裸名**（实例） | — | 只有键：类型带 `Key` 后缀、实例裸名。**键与值同住**——值类型属某个域的，实例住那个域（执行期两条 → `exec`，插件域两条 → `plugin`，capability 四条 → `capability`，agent 一条 → `agent`，见各行）；本域只留机制（`SymbioKey` trait）与值类型为通用词汇（`String` / `Value` / `Vec<String>`）的实例。**本域不收字符串常量** |
| `llm` | 子命名空间 `Model*`；契约词 `TurnOutput` · `TurnToolCallInfo` | — | `llm_` | 对应 `model_provider.rs` / `turn.rs`（+ `frames.rs` 的状态帧、`…`）。**`Turn*` 家族前缀属 `turn` 域**：`llm` 的两个 `Turn*` 是**具体契约名**（`TurnOutput` = 一轮模型输出），不是「turn 家族」的一员——它们先于 `turn` 域存在，且改写要跨 `plugins/model` / `plugins/loop` 两侧，收益为零 |
| `logger` | — | `LOG_`（登记缩写） | `logger_` | 常量用登记缩写，函数用全名——两者不混 |
| `plugin` | `Plugin` · `SimpleResponse` · `SuccessResponse` | `PLUGIN_`；子命名空间 `TRAVERSE_*` | `plugin_` · `lock_read` · `lock_write` · `config_mount_dispatch` | `PLUGIN_` 下细分：`PLUGIN_KEY_*`（清单键）· `PLUGIN_FILE` · `PLUGIN_PAYLOAD_KEY` · `PLUGIN_PARENT` / `PLUGIN_DIR`（两条**键实例**，与值类型同住本域，[D-095](../../docs/003-design/005-协议规范.md "DE-005-协议规范") 根因一——`PLUGIN_DIR` 的值是 `PluginDir`，`PLUGIN_PARENT` 的值是 `Option<Weak<dyn Plugin>>`）。插件工厂 id（`PLUGIN_ID_*`）**不在这里**：它是装配方的部署身份。`SimpleResponse` / `SuccessResponse`（`response.rs`）是路由回体的最小壳——**插件面**的东西，故由 `schemas` 并入本域。生命周期钩子的事件词 `HookEvent` / `HookOutput` 与地址常量 `ROUTE_*` 随 `fire_hook` 住宿主（`crate::hooks` / `crate::routes`，D-184） |
| `present` | 子命名空间 `Detail*`（`DetailDefinition` / `DetailField` / `DetailSection` / `DetailAction` / `DetailOption` / `DetailCondition` / `DetailBadge` / `DetailPreset` / `DetailPresetSpec`）· `Reading*`（`ReadingPanel` / `ReadingRow` / `ReadingSection` / `ReadingTone`） | `DETAIL_` | — | [D-087](../../docs/003-design/001-核心架构.md "DE-001-核心架构")：宿主呈现方言——**后端下发描述、前端零业务知识渲染**。两个子命名空间各对应一个文件：`detail`（可编辑详情页）/ `reading`（只读读数面板）。形状里不含任何业务语义（分区 / 字段 / 标签 / 值 / 口径），故换一个产者前端零改动。`capability` 的选项族行载荷是一份 `DetailField`——表单字段本来就是呈现物，依赖方向是 `capability → present`（向下） |
| `projection` | `Projection`；契约词 `TurnState` · `CheckpointState` · `ConsolidateParams` · `Rejection` · `Reputation*`（`ReputationEntry` / `ReputationView`）· `Ready*`（`ReadySetView` / `ReadyTask`）· `Conation*`（`ConationView` / `ConationIntent`）· `Cost*`（`CostLedgerView` / `CostEntry`）· `Calibration*`（`CalibrationView` / `SkillStats`）· `Fallback*`（`FallbackRateView` / `TierStats`）· `Transcript*`（`TranscriptView` / `TranscriptEntry`）· `Slo*`（`SloLatencyView` / `TierLatency`）· `Prompt*`（`PromptMessage` / `PromptToolCall`——**结构化** prompt 出口：角色、正文、`tool_call_id`、`tool_calls`；线格式由 adapter 边界翻译，core 不认识 provider 的 `ChatNode`）· `PromptGap` / `PromptReport`（事实网格 → prompt 的**双向完整性**判据，[D-048](../symbio-plugin-model/README.md "crates/symbio-plugin-model/README")；它**不是投影**而是投影的复核，所以归本域但不改「纯函数视图」的形态约定：入参事件切片 + 投影产出，出参缺口清单） · 跨 crate 内部词 `FileOp` · `SpanView` · `AutonomyView` | — | `consolidate_` · `reputation` · `plain_score` · `readyset` · `conation` · `cost_` · `calibration` · `fallback_` · `transcript` · `mutations_` · `slo_` · `checkpoint` · `verify_prompt_fidelity` · `branch_` · `accept` · `count_user_messages` · `workdir_changes` | 纯函数视图（F2）：形参只有 `&[Event]` / `Timestamp` / `Budget`，返回 `View` 而非 `Result`。`TurnState` 是 [plan/DE-001 §8](../../docs/003-design/001-核心架构.md "DE-001-核心架构") `projection = turnstate` 的产出视图名。见 [D-043](../../docs/003-design/001-核心架构.md "DE-001-核心架构") |
| `store` | `Store` / `MemoryStore` / `EventStore`；契约词 `AppendError` | — | — | `Store` trait 冻结（F1）：append / range / head，无 update/delete；`EventStore` 是装 `Event` 信封的便捷别名；`AppendError` 是 [plan/DE-001 §2](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 冻结签名的一部分。`range` 的口径是**完整史**（含被压缩快照取代的归档前缀）——本域只有这一条读法。持久化布局（文件格式 / 写者令牌 / 归档段）住使用方插件，不进根出口（[plan/14 §2.2](../../docs/003-design/014-单环一表.md "DE-014-单环一表") 推论 2） |
| `turn` | `Turn*`（`TurnSpec` / `TurnSettings` / `TurnDispatch` / `TurnReport` / `TurnStatus` / `TurnDispatchKey`）· 动作载荷 `AddNodesRequest` · `EditNodesRequest` · `RemoveNodesRequest` · `ReplaceAllRequest` · `ConsolidateRequest` · `Compression*` · `EditFrame` · 跨 crate 内部词 `DrainedSupplement` · `FleetSnapshot` · `WorkerProgress` | `TURN_DISPATCH` · `LOOP_RUN_TURN` · `WORKER_` · `META_SUPPLEMENT` · `TASK_CAP` | `run_ops` · `llm_emit_message` · `llm_message_frame` · `llm_removed_frame` · `llm_short_id` | [D-087](../../docs/003-design/001-核心架构.md "DE-001-核心架构")：轮次面——一次轮次**怎么交付**（`dispatch`）、**怎么造节点**（`node_build`）、**产出什么记录**（`facts`）。它同时看见 `llm`（模型输出）与 `chat`（会话对象），是两者之上的装配面，依赖链 `chat ◄ llm ◄ turn ◄ projection` 无环。`node_build` / `facts` 以叶子模块形态出根（符号走模块路径）；交付包与动作载荷因被 `session` / `loop` 两侧逐字段构造而平铺 |
| `text` | — | — | `text_` | 只有两个纯函数 |
| `vdfs` | `Vdfs` · 跨 crate 内部词 `AddrRootDecl` | `VDFS_` | `vdfs_` · `absolute_addr` · `descend_addr` · `join_addr` | 跨 crate 内部词：地址根的声明体，以及三条地址运算（绝对化 / 下降 / 拼接）——挂载写侧、路径闸、四个插件各持一段语义，判据必须单点 |
| `view` | `View` / `Budget` / `Recall*`（`RecallView` / `RecallEntry`） | — | `visible_to` | 投影产出的共享类型层（F2 后半句）：`View{value, degraded, used}`，`Budget` 是 I3 记账口径。`visible_to` 是可见性判定的**唯一**读法（主体 × 可见范围 → 布尔），写侧与三个读侧出口共用一个判据 |

> **`TurnRunner` / `RoundInjector` 已下沉 `plugins/session`**（2026-10-09）：运行器本体
> 与工具轮注入口都只有一个生产消费方（session），留在此层即「架构层挂业务钩子」的污染。
> 判据见 §4 第 1 问（[D-023](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。两者**不属于**
> `plan/01` 的冻结契约名——此前在 §1.2 表里登记为冻结契约名是口径误判。

> **`ActorSpec` 反向升为根 `pub use`**（同批）：它是 **F3 冻结锚点**（`plan/01 §4` 主体规格
> 五字段），下沉前靠 `TurnInput.actor`（当时的根导出）**顺带**可达、`pub(crate)` 够用；运行器
> 搬走后这条路径消失，而它只能住 core（F3 冻的是字段集）⇒ 必须自己占根出口。D-190 后它有
> **两个**生产消费方（`plugins/session` 的 `turn_exec` 与 `transcript/inbox` 的判定者行构造），
> C-003 的「≥2 消费方」判据自行成立，因此 `core-export-audit.mjs` 的 `WAIVERS` 里**没有**它
> （内核公共面豁免账本为空）。

> 判据始终是「**调用点读不读得出归属**」，表只是把结论固化。所以 `logger` 用 `LOG_`
> 而不是 `LOGGER_`（多出来的四个字母不增加任何信息）；而 `plugin` 域里 `PLUGIN_ID_*` /
> `PLUGIN_KEY_*` / `PLUGIN_FILE` 共用 `PLUGIN_`——它们同属一个域，细分段让「具体是什么」
> 也一眼可辨。
>
> 名字允许带一个**限定词**（上方的 `core-naming:modifiers` 标记列出它们）——限定词
> **不算**域前缀：`DynVdfsProvider` = `Dyn` + `VdfsProvider`，`DefaultToolVisitor` =
> `Default` + `ToolVisitor`。限定词表放在标记里而不是散文里，是为了让守卫能**解析**它
> （与 `plugin-entry-audit` 的 `<!-- vocab:… -->` 同一约定：**标了才认，没标不猜**）。
> （`DefaultToolVisitor` 已随收集器默认实现迁到 `providers/collectors/`，此处只作**词法**示例。）
>
> 这张表由 `scripts/core-naming-audit.mjs` **机械核对**：它枚举本层公开面，逐个检查前缀
> 是否落在所属域登记的前缀里（并按「词」比对，`VDFS_PLUGIN_PROVIDER_FIELD` 不会因为含
> `PLUGIN_` 被误判）。改表要同时改代码，改代码要同时改表——规范不再是「靠人记」，
> 而是可执行的。

## 2. 域清单

<!-- list:core-modules owner -->

| 域 | 职责 | 关键符号 | 子路径 |
|---|---|---|---|
| `authz` | 本机部署的授权策略表：主体清单与各自的 grants / 可见域，以及淘汰裁决的撤销覆盖层。判定语义复用 `governance`，本域只装「谁」这条部署事实 | `PRINCIPAL_*`（主体 id）· `CAP_*`（能力名）· `SESSION_OWNER` · `production_matrix` · `content_visible` · `EffectiveMatrix`（部署表 `fold` 撤销事实后的合成结果） | — |
| `adapters` | v2 ⑤ 外部资源的**唯一接缝**：时延闸门 = 依赖注入策略——令牌按档位签发（ZST、私有构造），`LlmAdapter::generate` 只收 `FullModel` ⇒ 反射档在类型上拿不到生成能力（[plan/DE-018 §3.3](../../docs/003-design/018-模块架构.md "DE-018-模块架构")） | `LatencyTier`（四层预算 80/300/60k/86.4M）· `TokenIssuer` · 令牌 `RuleOnly` / `FullModel`（**两档**：快速档令牌 `ClassifyOnly` / `issue_fast` 已按 [D-096](../symbio-plugin-model/README.md "crates/symbio-plugin-model/README") 退役——它守的 `Classifier` 端口生产从未存在，「只分类不生成」改由 classify 的返回类型与工具集判据承载）· `LlmAdapter`（`generate` / `generate_streaming`——流式只是帧的形态，收束语义不变）· `AdapterError` · `StubLlmAdapter`（`#[cfg(test)]` 零 LLM 桩：彩排与测试的确定性替身，`succeed_streaming` 分片 / `aborting` 中止）· `ProviderLlmAdapter`（真实实现：`execute_turn` 折进 `LlmAdapter`，帧桥择要转发正文增量）· `DeltaSink` / `SilentDeltas` | — |
| `actors` | v2 ② 主体：只收类型化输入（事件切片 / `View`）、只产事件（[plan/DE-018 §3.1](../../docs/003-design/018-模块架构.md "DE-018-模块架构") ② 行）；零 LLM 彩排的应答是彩排内的**定值**（链路形状一条不少），规则语义归 `classify` 规则表，`Reasoner` / `Translator` 归 S2 / S5 | `ActorSpec` · `Pattern`（三模式闭集：`Decider` / `Reasoner` / `Translator`）· `Scope`（root / child） | — |
| `agent` | Agent 包的**权限声明与它的执行方**（[DE-011 §5.3](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范") / D-153）：manifest `permissions` 四字段是**授权清单**（grant——列了才能用、空清单即拒），glob 相对 Agent 目录根判且只管自己的包，装载期超限整轮拒载。写侧只有 agent 插件，读侧是 web / vdfs 两族工具；收集期的共享槽（键与值同住）把「以谁的身份跑」从 traverse 带到工具执行期 | `AgentPermissions`（声明）· `AgentScope`（作用域，带目录根）· `AGENT_PERMS`（槽键）· `agent_perms_init_slot` / `agent_perms_of` | — |
| `capability` | 能力系统：LLM 可见工具与插件遍历面 | `Capability` · `CapabilityMeta` · `CapabilityCategory` · `CapabilityToolContextRetention` · `capability_invoke` · 收集口契约（`Registry`——put/get/list 三方法一张表，行词汇 `CapabilityRow*`；**默认实现**住 `providers/collectors` 的 `DefaultToolVisitor`）· `capability_resolve` / `capability_to_wire` · `failure_kind` · 错误桶读写（`CapabilityError` / `capability_report_error` / `capability_take_errors`；**桶的键** `CAPABILITY_ERRORS` 住在 `keys`）· 行词汇（`CapabilityRow` / `CapabilityFamily`；`row.rs`——载荷是开放类型，新族自带具体类型，内核不加枚举臂）· 模型族读写协议（`capability_model_active` / `capability_model_resolve`；`model.rs`）· 提示词族读协议（`capability_prompts`；写入是提示词行的 `put`，见 `prompt.rs`）· 挂载族读协议（`capability_vdfs_root` / `capability_mounts` / `capability_mount`；写入是挂载行 / 根行的 `put`，见 `mount.rs`） | `configurable` · `error` · `family` · `grants` · `model` · `mount` · `option` · `prompt` · `row` · `tool_name` |
| `chat` | 会话**线格式**：一条消息 / 一个主体长什么样，以及两个入口请求值。只说形状与解析，不含机制 | `chat_node`（`ChatNode` · `NodeRole` / `NodeKind` / `NodeStatus` · `MessageContent` · `ResumeRequest`）· `principal`（`Principal` / `PrincipalKind`）· `chat_request` / `session_chat`（两个 `Request`） | `chat_node` · `principal` · `chat_request` · `session_chat` |
| `clock` | 全项目「当前时间（Unix 毫秒）」唯一实现 | `clock_now_ms` | — |
| `creator` | 通用对象创建注册表：按 id 装配**任意**类型对象（见 [D-036](../../docs/003-design/002-架构总览.md "DE-002-架构总览")） | `creator_create_object` · `creator_has` · `creator_ids` | — |
| `decl` | **事实与视图声明层**：`FactDecl`（事实的字段清单）与 `ViewDecl`（视图读了哪些字段、成本上界、求值函数）；`seal` 在装载期检查视图引用的每个 `(事实,字段)` 都在某条事实声明里。内核只声明协议五格（`request` / `sent` / `response` / `failed` / `retry`）；控制面声明归 `plugins::loop::control_decl` | `FactDecl` · `ViewDecl` · `PluginDeclarations` · `Builder` · `Frozen` · `declare` · `seal` | — |
| `dialog` | 对话面**控制契约**（session ⇄ `classify` / `compose`）——判决是枚举、措辞是文本。生产方是 session，消费方是两个不同插件 ⇒ 满足 D-023 的 core 准入判据 | `Verdict`（闭集）· `DecideRequest` · `DecideOutcome`（含 `DecisionPlan`）· `ParseVerdictRequest` · `ComposeRequest` · `WordingReply` · `RunSnapshot` · `REASON_*`（理由码） | — |
| `embedding` | 嵌入服务的**抽象**（实现在 `src/providers/embedding`） | `EmbeddingService` · `EmbeddingError` · `EMBEDDING_LOCAL` / `EMBEDDING_NOOP` | `ids` |
| `event` | v2 事件契约的**中性共享类型层**（④ store 与 ③ projection 都只 import 本域，见 [D-043](../../docs/003-design/001-核心架构.md "DE-001-核心架构")） | `Seq`（私有构造：只有 `Store` 能分配）· `Event`（信封：幂等键 / 网格坐标 / 溯源 / 记账）· `EventEnvelope` · 语法网格闭集 `Entity`(11) × `Verb`(5)（F5）· 名字 ↔ 格位一张表 `event/grid.rs`（`event_cell` / `Event::grid_cell`，判据在唯一写入口）· 节点身份规则 `event_node_of`（`event_payload_node_id` / `event_produced_text_node_id` / `event_tool_result_node_id`）· 停机原因闭集 `ClosureReason`（`reason` 载荷字段的唯一取值来源，该批：写侧、审计判据 `closure_reason_closed` 与验证程序三处共用一份 `ALL`，不各写一份） | — |
| `event_bus` | 跨插件全局发布设施门面 + 频道词表 | `EventBus` · `EventBusSubscribeRequest` · `EVENT_BUS_KIND_SYSTEM` · `EVENT_BUS_KIND_VDFS` · `EVENT_BUS_RESYNC_MARKER_TYPE` | — |
| `governance` | v2 ⑥ 权限与可见性（第零天，[plan/DE-001 §7](../../docs/003-design/001-核心架构.md "DE-001-核心架构")）：授权**读写成对**（构造即拒绝单侧策略）、查询 fail-closed；`thread_private` 缺省（C10 越界读取 0）。**生产接线**（[DE-016 §3.1 批⑥](../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：表与机制同住本层（`symbio_core::authz` 模块，能力名字符串 ⇒ `from_names` 按能力闭集校验），判定是 `plugins/session` 的闸（写侧收束、读侧可见域、外部执行闸门的授权判据） | `PermissionMatrix`（**根导出**；`new` 成对性校验 · `from_names` 表构造 · `grants_of` / `can_write` / `can_write_name` / `can_reply` 写侧 · `sees_of` / `can_see` 读侧）· `PrincipalPolicy` · `Capability`（能力闭集，**路径限定** `governance::Capability`，不进根平铺；授权表写能力名）· `VisScope`（**根导出**）· `PairingViolation` · `PolicyError`（表构造失败：认不出的能力名 / 不成对） | — |
| `exec` | 执行期原语：事件出口（出）与中止信号（入） | `ExecEventSink` · `ExecAbortSignal` · `ExecEnv` · `ExecTranscriptWriter` | — |
| `invariants` | v2 三条可执行不变量（N1 单调 / N3 每 turn 一条 final / N5 断言必带溯源）；每条检查都有反向用例 | `seq_monotonic` · `final_unique_per_turn` · `produced_by_coverage` · `unresolved_turns` · `budget_exceeded` · `acyclic_deps` · `rework_bounded` · `check_all` · `Violation` | — |
| `keys` | **类型安全上下文键**（只有键：trait + 类型 + 实例） | `SymbioKey` 及其实例（`PATH` · `WORKDIR` · `ID` · `NAME` · `PLUGIN_DIR` · `CAPABILITY_VISITOR` · `CAPABILITY_ERRORS` …） | — |
| `llm` | 模型服务的唯一契约面（协议无关、插件无关）——只留**多消费方**共用的符号（唯一例外 `llm_message_frame` 见 D-023） | `ModelProvider` · `ModelFinishReason` · `ModelUsage` · `ModelSpend` · `TurnOutput`（`tool_calls` 是**结果形态**，只装结果不装过程；累积过程住 `plugins/model/tool_accumulator.rs`，三个读方法 `is_reasoning_only` / `effective_text` / `into_messages` 住 `plugins/session/message_build.rs`）· `TurnToolCallInfo` · 帧 `llm_emit_message` / `llm_message_frame` / `llm_removed_frame` · id 原语 `llm_short_id` | `model_provider` · `turn` |
| `logger` | 结构化日志与级别闸门 | 日志宏 · `LOG_LEVEL_*`（`MIN_LEVEL` 是**私有**静态量，不是公开面） | — |
| `plugin` | 插件核心契约：trait、信封、错误、目录与地址 | `Plugin` · `PluginMeta` · `PluginInvokeRequest` / `PluginInvokeResponse` · `PluginError` / `PluginErrorCode` · `PluginChannel` / `PluginFrame` / `PluginPayload` / `PLUGIN_PAYLOAD_KEY` · `PluginDir` / `PluginConfigFile` / `PluginEntry` · `PLUGIN_KEY_*` / `PLUGIN_FILE`（清单）· `TRAVERSE_AVAILABLE_*`（遍历端点） | `dir` · `error` · `transport` · `traverse` |
| `present` | **宿主呈现方言**：后端下发描述、前端零业务知识渲染。两个面同一族哲学——`detail` 服务可编辑详情页，`reading` 服务只读读数面板 | `DetailDefinition` 家族（字段 / 分区 / 动作 / 预设 / 条件 / 徽标）· `ReadingPanel` 家族（分区 / 行 / 口径 / 色调）· `DETAIL_*` | `detail` · `reading` |
| `projection` | 纯函数视图（F2）：从事件切片派生一切读数，形参只有 `&[Event]` / `Timestamp` / `Budget`，返回 `View` 而非 `Result` | `Projection` · `TurnState` · `CheckpointState` · `ConsolidateParams` · `Rejection` · 视图家族 `Reputation*` / `Ready*` / `Conation*` / `Cost*` / `Token*` / `Spend*` / `Calibration*` / `Fallback*` / `Transcript*` / `Slo*` · 结构化 prompt 出口 `Prompt*` · 双向完整性判据 `PromptGap` / `PromptReport` | — |
| `store` | 一切事实的**唯一写入口与读取面**（append-only：trait 上不存在 update/delete，F1）；`memory` 是平凡基线，持久化实现住使用方插件 | `Store`（trait）· `MemoryStore` · `EventStore`（便捷别名）· `AppendError` | — |
| `text` | 字符串安全截断（避免按字节切多字节字符 panic） | `text_truncate_bytes` · `text_floor_char_boundary` | — |
| `turn` | **轮次面**：一次轮次怎么交付（`dispatch`）、怎么按形状造节点（`node_build`）、产出什么记录（`facts`）。同时看见 `llm` 与 `chat`，是两者之上的装配面 | `TurnSpec` / `TurnSettings` / `TurnDispatch` / `TurnReport` / `TurnStatus` · 动作词表 `run_ops` 与载荷（`AddNodesRequest` / `EditFrame` / `ConsolidateRequest` / `Compression*`）· 写侧构造 `llm_build_*` / `dialog_node` / `PresentationTree` · 产物记录 `Delegation` / `TaskDeclaration` / `ReportDraft` | `dispatch` · `node_build` · `facts` |
| `vdfs` | 统一资源访问契约（规范见 [DE-010-VDFS前端页面规范](../../docs/003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范")） | `VdfsProvider` · `VdfsNode` · `VdfsRequest` / `VdfsResponse` / `VdfsListResponse` · `VdfsError` · `VdfsChangeSubscriptions` · `VDFS_*` 词表 | `address` · `host` |
| `view` | 投影产出的**共享类型层**（与 `event` 同理：③ projection 与将来的 ② actors 都只 import 本域） | `View`（`value` / `degraded` / `used`——「可降级」是类型义务）· `Budget` | — |

> `capability` / `chat` / `embedding` / `llm` / `plugin` / `present` / `turn` 的第四列是**协作者视角**的子路径，
> 供直接定位；它们同样是「域的公开面」，只是按主题分了文件。**消费方不按这里深引**（第 4 条），
> 例外是**叶子模块**：`chat_node` · `chat_request` · `session_chat` · `principal` · `facts` · `node_build`。
> 它们是**主题命名空间**（字段家族 / 记录家族 / 构造家族），不是深引捷径——路径只有一层，
> 且名字必须自证主题（`node_build::dialog_node` 读得出「造节点的」，不是「某个域的内部件」）。

**域不按代码量分。** `clock`（25 行）与 `text`（49 行）各自成域，不并成一个「工具域」：
域名要**自证内容**，而一个能同时装下「纯函数」（`text`，输入决定输出）与「非确定性来源」
（`clock`，无输入、随系统时钟变化）的名字只能是杂物抽屉——它随后会收下所有「暂时没处放」
的东西，`keys` 一度收下插件清单正是这一类错放。合并判据见 [D-035](../../docs/003-design/001-核心架构.md "DE-001-核心架构")。

## 3. 域级命名规则

§1.2 的域前缀表覆盖**绝大多数**符号。本节登记的是**整个域**层面的替代规则——
它们不是「某个符号的豁免」，而是该域**所有**符号都遵循的规则。

| 域 | 规则 | 理由 |
|---|---|---|
| `keys` | 类型用 `…Key` **后缀**；实例用**裸名** | 见下 |

**这里没有「符号级豁免」。** 新增符号若不符合 §1.2 的表，只有两条路：改名，或者在本表
新增一条**域级**规则并把判据说清。不接受「它已经有消费方了」作为理由——标识符改名只需
一次全仓替换，**值**（跨进程字面量）不受影响。

**「改标识符」与「改值」是两件事，不要混成一件。** 一个常量名（标识符）只在本仓出现，
改名是一次全仓替换；它携带的**值**（跨进程字面量）一个字节不变。值层面的兼容性由协议
守卫（`scripts/protocol-mirror-audit.mjs` 的常量镜像组）管，与标识符叫什么无关——因此
「这个常量已经有消费方了」不构成不改名的理由。

### `keys` 域的两种形态不是「两套风格」

`keys` 域里同时有 `PathKey`（类型）与 `PATH`（实例），看似风格分裂；逐行
`grep "^pub const" keys/mod.rs` 可验证它其实是一条**有判别力的规则**：

| 形态 | 一律写成 | 它是什么 | 例 |
|---|---|---|---|
| **类型**（`SymbioKey` 的实现） | `…Key`（**后缀**） | 键的**类型** | `PathKey` · `CapabilityVisitorKey` |
| **实例**（该类型的唯一实例） | **裸名** | 键**对象**：`ctx.get(&…)` 的凭据 | `PATH: PathKey` · `ID: IdKey` · `CAPABILITY_VISITOR: CapabilityVisitorKey` |

判别力来自两侧**互不重叠**：一个名字要么是类型（带 `Key` 后缀），要么是实例（不带）。
消费形态也不同——类型出现在类型位置，实例是**取值的凭据**（`ctx.get(&PATH)`）。

把实例改成 `KEY_PATH` 会让 `KEY_` 同时指代「类型」与「键对象」——**用一个统一前缀换掉
一个真实的区分**，是净亏损。故维持裸名。

本域**不收字符串常量**：插件 id、路由地址、遍历端点、载荷键都按「它描述什么」归了
`plugin` / `embedding` 的域（见 §1.2 的「放置也是同一条规则的一部分」）。

## 4. 新增内容时的四问

1. **有几个依赖方？** 只有一个 → 下沉回那个模块，不进本层（D-023）。
 **依赖方指生产模块，测试不算**——测试依赖的是**被测对象**，不是它要长期持有的接口；
 让「另一个模块的测试顺手写了一次名字」算作第二个依赖方，等于让测试决定一个实现的归属
 （`TurnRunner` 就这样长期显为 ≥2 而逃过下沉，2026-10-09 修正，口径见
 [`core-export-audit.mjs`](../../scripts/core-export-audit.mjs) 的头注）。

 **完整规则**（C-003 按 [`core-surface.mjs`](../../scripts/core-surface.mjs) 的
 `isSinkCandidate` 判，判定型与报告型共用同一份实现）：

 | 情形 | 结论 |
 | --- | --- |
 | 含测试消费方 **0** | **待处置（收窄）**——根出口对它没用 |
 | 含测试 > 0、生产 **0** | **合法根出口**，既不收窄也不下沉 |
 | 生产消费方 **≥ 2** | **保留** |
 | 生产消费方 **= 1**，且 **core 内（任意域）有消费者** | **保留** |
 | 生产消费方 **= 1**，且 core 内无消费者 | **待处置**（按 §4 四问判：下沉 / 成文契约 / 登记豁免） |

 五条补充，都影响结果：

 - **core 内的使用也算依赖方**（2026-10-11 ；2026-10-14 修正粒度）。
 **消费就是依赖，自己依赖自己也是依赖**——core 内部在用它，它就 ≥2 个单位共享；
 下沉到那唯一的外部消费方会让 core 反向依赖插件（当场编译不过）⇒ 它本来就该住本层。
 ⚠️ 此前这里写的是「core 内**另一个**域在用」，实现按「声明域自身不算」过滤——
 那个粒度把 core 内部的依赖整个漏掉了，连带产生过一次真实误判（`LlmAdapter` /
 `DeltaSink` 的实现方就在自己的声明域 `adapters` 里，被判成「该下沉」）。
 同时排除的只有两样：**声明行**与**`pub use` 导出清单**——那不是使用。
 - **唯一消费方是 core 自己 ⇒ 无需下沉**。它在模型里落不到「单消费方」档：`units`
 数的是**跨 core** 的生产消费方，core 内部的依赖单独数。这类符号落「0 消费方」档
 （根出口对它没用，该收窄），**不会被建议下沉**——下沉的目标本就是它自己。
 - **待处置 ≠ 自动下沉**。「1 个消费方」不等于「该下沉」：宿主接缝
 （`ExecTranscriptWriter`）、跨 crate 契约（`EventBusSubscribeRequest`）、
 类型行为（`SloLatencyView::merge`）都只有一个消费方却必须留在 core。
 判据能判的是**待处置**；动作按 §4 四问走——判据替人做架构判断，是它做不好的事。
 - **自引用**（符号名与某插件撞名）与**宏调用点**（`plugin_error!(…)` 里名字不落地）
 单列判，不混进上述计数——前者会造出幽灵消费方，后者纯文本扫不到。
 - **棘轮不是「当场判红」**：存量按 `BASELINE` 压住，**新增才红**。
2. **是契约还是实现？** 只有**一个**实现方认的东西（重试机器、流循环、拓扑知识）留在
 插件里；被**多个**模块共享的机制可以进本层（"依赖方数量"判据，见上）。
3. **放进哪个域、叫什么？** 先找职责最近的既有域；找不到再建新域——建域的成本是
 一个目录 + 一条 §1–§4 的完整合规，不要为单个 trait 开域。
4. **该是什么形态？** 契约 / 值对象 / 纯函数 / 全局单例 / provider——判据见
 [D-035](../../docs/003-design/001-核心架构.md "DE-001-核心架构")。**共享不等于要抽 trait**：provider 化要同时满足
 「≥2 实现且编译期不知选哪个」「无状态或状态可共享」「按 id 装配而非处理数据」三条。
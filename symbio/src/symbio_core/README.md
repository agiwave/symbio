# symbio_core —— 内核契约层：命名与结构规范

本目录是**内核契约层 + 跨插件共享内核**：放跨插件共享的 trait、协议类型、词表常量、
纯工具函数，以及**被多个模块消费的共享机制**（如 `event_bus` 的全局订阅表、`logger`
的结构化日志与级别闸门）。判据只有一个——**依赖方数量**：只被一个模块依赖的内容一律下沉回该
模块，不论它「够不够底层」（见 [ADR-023](../../../docs/DECISIONS.md)）。
本文是该目录**命名与结构**的唯一 owner。

> 「不放实现」是**方向**而非字面事实：域里既有纯契约（`vdfs` / `capability` / `llm`
> 的 trait 面），也有共享实现（`event_bus` 的全局订阅表、`logger` 的进程级闸门）。
> 判据是「谁依赖它」，不是「它抽象不抽象」——把共享机制硬抽成 trait
> 只会多一层间接（见 [ADR-035](../../../docs/DECISIONS.md)）。

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
   | 常量 / 静态量 | 域名（SCREAMING_SNAKE）+ 语义 | `VDFS_ACTION_ABORT` · `EVENT_BUS_KIND_VDFS` · `ASSEMBLY_SUB_AGENT_PLUGINS` |
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
   | **子命名空间** | 域内按主题分了文件，且主题名比域名更有信息量 | `llm`：`Model*`（`model_provider.rs`）· `Turn*`（`turn.rs`）。`capability`：`Configurable*`（`configurable.rs`）· `Option*`（`option.rs`）· `Tool*`（`tool_name.rs`），外加**模块**形态的 `failure_kind`。`plugin`：`ROUTE_*`（`route.rs`）· `TRAVERSE_*`（`traverse.rs`） |
   | **后缀** | 该域的类型名有一个比域名更强的类别词 | `keys` 的 `…Key`（`PathKey`）——`KeyPath` 会读成「键的路径」，语义反了 |
   | **登记缩写** | 域名的大写形式冗余且不增加信息 | `logger` → `LOG_`（`LOG_LEVEL_INFO` 已足够定位） |

   子命名空间有两种形态：**类型前缀**（`Model*` / `ROUTE_*`）与**模块**（`failure_kind`）。
   模块是**命名空间，不是符号**——它的名字是主题，前缀规则对它不适用。`core-naming-audit`
   识别模块并跳过；跳过时**计数**，因为静默跳过会让「公开面里有什么」出现看不见的口子。

   判据始终是「**调用点读不读得出归属**」，不是「字面是否等于目录名」：

   - `ChangeSubscriptions` 住在 `vdfs` 却既不姓 `Vdfs` 也不属于任何已登记主题 ⇒
     改名 `VdfsChangeSubscriptions`；
   - `KEY_PROVIDER` 住在 `plugin` 却用着 `keys` 域的前缀 ⇒ 改名 `PLUGIN_KEY_PROVIDER`；
   - 插件工厂 id 描述的是「哪个插件」而非「哪个键」⇒ 从 `keys` 搬到 `plugin::ids`，
     并改名 `PLUGIN_ID_*`；
   - 反例：`LOG_LEVEL_INFO` **不**需要改成 `LOGGER_LOG_LEVEL_INFO`——多出来的四个
     字母不增加任何信息。

   **一域一前缀，且前缀不跨域复用。** 两个域共用同一个前缀就是「一个前缀两种东西」——
   同一个 `PLUGIN_` 既指插件工厂 id 又指清单键，读的人得先分辨是哪一个。故 `PLUGIN_`
   只属于 `plugin`（工厂 id 与清单键靠 `PLUGIN_ID_*` / `PLUGIN_KEY_*` 细分段区分），
   `KEY_` 不属于 `keys`（该域没有字符串常量，见 §3）。

   **放置也是同一条规则的一部分**：符号住哪个域由「它描述什么」决定，不由「谁先用了它」
   决定。`SymbioKey` 实例一律定义在 `keys`（机制与键分家：`capability/error.rs` 只放错误
   桶的读写函数，桶的键 `CAPABILITY_ERRORS` 住 `keys`）；插件 id 归 `plugin::ids`、路由
   地址归 `plugin::route`、遍历端点归 `plugin::traverse`、嵌入服务 id 归 `embedding::ids`
   ——它们都曾住在 `keys`。
3. **依赖单向、只依赖更底层**：域之间只允许「上层 → 下层」，不得互相回指。当前域间图
   在**代码层**是无环的（对照 `symbio_core/mod.rs` 的 `pub use` 与各域 `mod.rs` 的导入；
   大量「环」是文档链接造成的假阳性——统计时先剥注释）。插件专属的实现代码不进本层
   （如 `llm` 只留契约，HTTP 重试与 SSE 流循环留在 `plugins/model/`）。**宿主接缝的统一
   形态是「纯接口 + 一个桥文件」**：接口与域类型宿主无关，只有 `host.rs` 知道本宿主的
   信封类型（`vdfs/host.rs` 是范本），换宿主只重写那一个文件。
4. **一个出口（根平铺重导出，且只导"跨出 core 的"）**：**core 外**的消费方一律写
   `symbio_core::<符号>`——域 `mod.rs` 的公开面由根 [`mod.rs`](./mod.rs) 逐条平铺重导出。
   **只在 core 内部用的符号不出根**：根 `pub use` 里没有它，core 内自己走域内路径
   `symbio_core::<域>::<符号>`（否则「没人用的东西占着公共面」会一直长）；只被测试或
   无人使用的存量，按 [`dead-code-audit`](../../../scripts/dead-code-audit.mjs) R-002
   **逐项承认**（`// dead-code-allow R-002: <理由>`，理由必填、数量棘轮只许降）。
   **根 `mod.rs` 里域目录一律 `mod <域>;`（私有）**——
   子目录不开模块门，谁出现在公共面上由根 `pub use` 逐条声明；`schemas/` 同样如此：
   契约按符号（`Verdict` / `ChatMessage`…）与两个叶子模块（`chat_message` / `session_chat`）
   在根导出，不再有 `symbio_core::schemas::…` 的深引豁免。任何深路径
   （`symbio_core::plugin::dir::` 之类）视为不规范，应改为根平铺。
   **可执行判据**：[`core-export-audit.mjs`](../../../scripts/core-export-audit.mjs)——
   C-001（根出口唯一）/ C-002（无深引）/ C-003（根输出的符号必须被 ≥2 个模块消费，
   0 消费方基线已归零、单消费方存量走棘轮，豁免写 §4 四问的理由）。

### 域前缀对照表（全 14 域 —— 新增符号照此取名）

这张表是第 2 条的**唯一执行口径**：拿不准新符号叫什么，先在这里查它的域。
「—」= 该域没有这一类符号（不是「不用前缀」）。**一个前缀只属于一个域。**
「裸名」= 不要求前缀（见 §3 的 `keys` 域规则）。

<!-- core-naming:modifiers Dyn,Default -->

| 域 | 类型 / trait | 常量 / 静态量 | 函数 / 自由函数 | 备注 |
|---|---|---|---|---|
| `assembly` | — | `ASSEMBLY_` | — | 本域只有两个常量 |
| `adapters` | 契约词 `LatencyTier` · `RuleOnly` · `ClassifyOnly` · `FullModel` · `CanClassify` · `CanGenerate` · `TokenIssuer` · `LlmAdapter` · `LlmTurn` · `AdapterError` · `StubLlmAdapter` · `ProviderLlmAdapter` · `DeltaSink` · `SilentDeltas` · `DispatchPort` · `DispatchOutcome` | — | — | 全部是 [plan/05 §3.3](../../../docs/plan/05-模块架构.md) 注入策略与 [plan/01 §10](../../../docs/plan/01-核心架构.md) 四层时延表的冻结契约名（出处 [`verify/latency_gate.rs`](../../../docs/plan/verify/latency_gate.rs)），名字先于模块存在——判据同 `schemas`。`LlmTurn` / `DispatchPort` / `DispatchOutcome` 是 [plan/10 §2](../../../docs/plan/10-工具轮v2化实施方案.md) 的两只端口（生成 / 工具分发），与前缀同域：**不能叫 `Tool*`**——那是 `capability` 域的子命名空间 |
| `actors` | `Actor*`（`ActorSpec`）；契约词 `Pattern` · `Scope` · `Decider`（含 `DeciderMiss`）· `Reasoner` · `RecallTranslator` · `CommitmentKeeper` · `PreemptionDecider`（含 `Preemption`）· `CircuitBreaker`（含 `GateDecision`）· `AutonomousInitiator` · `IntentGate`（含 `ConationPolicy` / `ConationCandidate` / `GateWarrant` / `IntentDecision` / `ApprovedIntent`）· `SkillCompiler` · `SkillRouter`（含 `SkillRoute`）· `TurnRunner`（含 `TurnOutcome` / `TurnInput` / `TurnResume`） | — | — | [plan/01 §4](../../../docs/plan/01-核心架构.md) 的冻结契约名（名字先于模块存在，判据同 `schemas`）；`Decider` 是 [plan/05 §4](../../../docs/plan/05-模块架构.md) S8 反射档判定者，S1 先以规则应答形态落地 |
| `capability` | `Capability`；子命名空间 `Configurable*` · `Option*` · `Tool*` | — | `capability_` | 无常量；三个子命名空间各有对应文件 |
| `clock` | — | — | `clock_` | 只有一个函数 |
| `creator` | — | — | `creator_` | 通用对象创建注册表：按 id 装配**任意**类型对象，见 §2 |
| `embedding` | `Embedding` | `EMBEDDING_` | — | 服务 id 在 `embedding/ids.rs` |
| `event` | `Event` / `EventEnvelope`；契约词 `Seq` · `Entity` · `Verb` · `Timestamp` | `EVENT_` | — | 契约词来自 [plan/01 §2](../../../docs/plan/01-核心架构.md) 的冻结契约文本，名字先于模块存在（判据同 `schemas`：改名 = 代码与契约漂移）。见 [ADR-043](../../../docs/DECISIONS.md)。常量 = 事件**名字表**（名字是数据，单点定义） |
| `event_bus` | `EventBus` | `EVENT_BUS_` | `event_bus_` | |
| `exec` | `Exec` | — | — | 本域无常量 |
| `governance` | 契约词 `PermissionMatrix` · `PrincipalPolicy` · `PairingViolation` · `VisScope`；**Capability（7 个封顶）刻意不进根平铺** | — | — | [plan/01 §7](../../../docs/plan/01-核心架构.md) 的冻结契约名。governance 的 Capability（权限矩阵的键）与 `capability` 域的 `Capability`（LLM 工具描述符）是**两个概念**、计划文本同名——以路径限定消歧：两个 Capability 同时出现在调用点正是命名纪律要阻止的错位，故本域不把它登记为前缀 |
| `invariants` | `Violation` | — | **裸名**（不变量名） | 函数名即不变量的可执行名（N1/N3/N5 的 C1/C2/C3 形态），谓词名比域名更有信息量——判据同 `schemas`：名字先于模块存在 |
| `keys` | `…Key`（**后缀**） | **裸名**（实例） | — | 只有键：类型带 `Key` 后缀、实例裸名。**本域不收字符串常量** |
| `llm` | 子命名空间 `Model*` · `Turn*` | — | `llm_` | 对应 `model_provider.rs` / `turn.rs` |
| `logger` | — | `LOG_`（登记缩写） | `logger_` | 常量用登记缩写，函数用全名——两者不混 |
| `plugin` | `Plugin` | `PLUGIN_`；子命名空间 `ROUTE_*` · `TRAVERSE_*` | `plugin_` | `PLUGIN_` 下细分：`PLUGIN_ID_*`（工厂 id）· `PLUGIN_KEY_*`（清单键）· `PLUGIN_FILE` · `PLUGIN_PAYLOAD_KEY` |
| `projection` | `Projection`；契约词 `TurnState` · `CheckpointState` · `ConsolidateParams` · `Rejection` · `Reputation*`（`ReputationEntry` / `ReputationView`）· `Ready*`（`ReadySetView` / `ReadyTask`）· `Cost*`（`CostLedgerView` / `CostEntry`）· `Calibration*`（`CalibrationView` / `SkillStats`）· `Fallback*`（`FallbackRateView` / `TierStats`）· `Transcript*`（`TranscriptView` / `TranscriptEntry`）· `Slo*`（`SloLatencyView` / `TierLatency`） | — | `consolidate_` · `reputation` · `plain_score` · `readyset` · `cost_` · `calibration` · `fallback_` · `transcript` · `slo_` | 纯函数视图（F2）：形参只有 `&[Event]` / `Timestamp` / `Budget`，返回 `View` 而非 `Result`。`TurnState` 是 [plan/01 §8](../../../docs/plan/01-核心架构.md) `projection = turnstate` 的产出视图名。见 [ADR-043](../../../docs/DECISIONS.md) |
| `schemas` | 协议词 | 协议词 | 协议词 | 命名空间就是协议本身，见 §3 |
| `store` | `Store` / `MemoryStore` / `EventStore` / `WalStore` / `EventWalStore`；契约词 `AppendError` | — | — | `Store` trait 冻结（F1）：append / range / head，无 update/delete；`EventStore` 是装 `Event` 信封的便捷别名；`AppendError` 是 [plan/01 §2](../../../docs/plan/01-核心架构.md) 冻结签名的一部分 |
| `text` | — | — | `text_` | 只有两个纯函数 |
| `vdfs` | `Vdfs` | `VDFS_` | `vdfs_` | |
| `view` | `View` / `Budget` / `Recall*`（`RecallView` / `RecallEntry`） | — | — | 投影产出的共享类型层（F2 后半句）：`View{value, degraded, used}`，`Budget` 是 I3 记账口径 |

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

| 域 | 职责 | 关键符号 | 子路径 |
|---|---|---|---|
| `assembly` | 装配策略：一棵标准插件树挂哪些插件、哪些插件不许被停用 | `ASSEMBLY_SUB_AGENT_PLUGINS` · `ASSEMBLY_UNDISABLABLE_PLUGINS` | — |
| `adapters` | v2 ⑤ 外部资源的**唯一接缝**：时延闸门 = 依赖注入策略——令牌按档位签发（ZST、私有构造），`LlmAdapter::generate` 只收 `FullModel` ⇒ 反射/快速档在类型上拿不到生成能力（[plan/05 §3.3](../../../docs/plan/05-模块架构.md)） | `LatencyTier`（四层预算 80/300/60k/86.4M）· `TokenIssuer` · 令牌 `RuleOnly` / `ClassifyOnly` / `FullModel` · trait `CanClassify` / `CanGenerate` · `LlmAdapter`（`generate` / `generate_streaming`——流式只是帧的形态，收束语义不变）· `AdapterError` · `StubLlmAdapter`（零 LLM 桩，演练兜底；`succeed_streaming` 分片）· `ProviderLlmAdapter`（真实实现：`execute_turn` 折进 `LlmAdapter`，帧桥择要转发正文增量）· `DeltaSink` / `SilentDeltas` | — |
| `actors` | v2 ② 主体：只收类型化输入（事件切片 / `View`）、只产事件（[plan/05 §3.1](../../../docs/plan/05-模块架构.md) ② 行）；S1 只落 `Decider` 规则应答（零 LLM 平凡值），`Reasoner` / `Translator` 归 S2 / S5 | `ActorSpec` · `Pattern`（三模式闭集）· `Scope`（root / child）· `Decider` · `Miss`（规则未命中 = 兜底触发条件，不是错误） | — |
| `capability` | 能力系统：LLM 可见工具与插件遍历面 | `Capability` · `CapabilityMeta` · `CapabilityCategory` · `CapabilityToolContextRetention` · `capability_invoke` · 收集**三**表契约（`CapabilityVisitor` / `OptionVisitor` / `ConfigurableVisitor`；三者的**默认实现**住 `providers/collectors`）· `capability_resolve` / `capability_to_wire` · `failure_kind` · 错误桶读写（`CapabilityError` / `capability_report_error` / `capability_take_errors`；**桶的键** `CAPABILITY_ERRORS` 住在 `keys`） | `configurable` · `error` · `option` · `tool_name` |
| `clock` | 全项目「当前时间（Unix 毫秒）」唯一实现 | `clock_now_ms` | — |
| `creator` | 通用对象创建注册表：按 id 装配**任意**类型对象（见 [ADR-036](../../../docs/DECISIONS.md)） | `creator_create_object` · `creator_has` · `creator_ids` | — |
| `embedding` | 嵌入服务的**抽象**（实现在 `src/providers/embedding`） | `EmbeddingService` · `EmbeddingError` · `EMBEDDING_LOCAL` / `EMBEDDING_NOOP` | `ids` |
| `event` | v2 事件契约的**中性共享类型层**（④ store 与 ③ projection 都只 import 本域，见 [ADR-043](../../../docs/DECISIONS.md)） | `Seq`（私有构造：只有 `Store` 能分配）· `Event`（信封：幂等键 / 网格坐标 / 溯源 / 记账）· `EventEnvelope` · 语法网格闭集 `Entity`(10) × `Verb`(5)（F5） | — |
| `event_bus` | 跨插件全局发布设施门面 + 频道词表 | `EventBus` · `EventBusSubscribeRequest` · `EVENT_BUS_KIND_SYSTEM` · `EVENT_BUS_KIND_VDFS` · `EVENT_BUS_RESYNC_MARKER_TYPE` | — |
| `governance` | v2 ⑥ 权限与可见性（第零天，[plan/01 §7](../../../docs/plan/01-核心架构.md)）：授权**读写成对**（构造即拒绝单侧策略）、查询 fail-closed；`thread_private` 缺省（C10 越界读取 0） | `PermissionMatrix`（`grants_of` / `can_write` 写侧 · `sees_of` / `can_see` 读侧）· `PrincipalPolicy` · `Capability`（7 个封顶，**路径限定** `governance::Capability`，不进根平铺）· `VisScope` · `PairingViolation` | — |
| `exec` | 执行期原语：事件出口（出）与中止信号（入） | `ExecEventSink` · `ExecAbortSignal` · `ExecEnv` · `ExecTranscriptWriter` | — |
| `invariants` | v2 三条可执行不变量（N1 单调 / N3 每 turn 一条 final / N5 断言必带溯源）；每条检查都有反向用例 | `seq_monotonic` · `final_unique_per_turn` · `produced_by_coverage` · `unresolved_turns` · `budget_exceeded` · `acyclic_deps` · `rework_bounded` · `check_all` · `Violation` | — |
| `keys` | **类型安全上下文键**（只有键：trait + 类型 + 实例） | `SymbioKey` 及其实例（`PATH` · `WORKDIR` · `ID` · `NAME` · `PLUGIN_DIR` · `CAPABILITY_VISITOR` · `CAPABILITY_ERRORS` …） | — |
| `llm` | 模型服务的唯一契约面（协议无关、插件无关）——只留**多消费方**共用的符号（唯一例外 `llm_message_frame` 见 ADR-038） | `ModelProvider` · `ModelFinishReason` · `ModelUsage` · `TurnOutput`（`tool_calls` 是**结果形态**，只装结果不装过程；累积过程住 `plugins/model/tool_accumulator.rs`，三个读方法 `is_reasoning_only` / `effective_text` / `into_messages` 住 `plugins/session/message_build.rs`）· `TurnToolCallInfo` · 帧 `llm_emit_message` / `llm_message_frame` / `llm_removed_frame` · id 原语 `llm_short_id` | `model_provider` · `turn` |
| `logger` | 结构化日志与级别闸门 | 日志宏 · `LOG_LEVEL_*`（`MIN_LEVEL` 是**私有**静态量，不是公开面） | — |
| `plugin` | 插件核心契约：trait、信封、错误、目录、身份与地址 | `Plugin` · `PluginMeta` · `PluginInvokeRequest` / `PluginInvokeResponse` · `PluginError` / `PluginErrorCode` · `PluginChannel` / `PluginFrame` / `PluginPayload` / `PLUGIN_PAYLOAD_KEY` · `PluginDir` / `PluginConfigFile` / `PluginEntry` · `PLUGIN_ID_*`（工厂 id）· `PLUGIN_KEY_*` / `PLUGIN_FILE`（清单）· `ROUTE_*`（路由地址）· `TRAVERSE_AVAILABLE_*`（遍历端点） | `dir` · `error` · `ids` · `route` · `transport` · `traverse` |
| `schemas` | 跨端协议 schema（前端逐字段镜像） | `ChatMessage` · `HookEvent` · `SuccessResponse` · 详情表 schema | `common` · `detail` · `hook` · `session` |
| `store` | 一切事实的**唯一写入口与读取面**（append-only：trait 上不存在 update/delete，F1）；S0 只落 `memory` 实现，S4 的 `wal` 从这里长出 | `Store`（trait）· `MemoryStore` · `EventStore`（便捷别名）· `AppendError` | — |
| `text` | 字符串安全截断（避免按字节切多字节字符 panic） | `text_truncate_bytes` · `text_floor_char_boundary` | — |
| `vdfs` | 统一资源访问契约（规范见 [design/vdfs.md](../../../docs/design/vdfs.md)） | `VdfsProvider` · `VdfsNode` · `VdfsRequest` / `VdfsResponse` · `VdfsError` · `VdfsChangeSubscriptions` · `VDFS_*` 词表 | `address` · `host` |
| `view` | 投影产出的**共享类型层**（与 `event` 同理：③ projection 与将来的 ② actors 都只 import 本域） | `View`（`value` / `degraded` / `used`——「可降级」是类型义务）· `Budget` | — |

> `capability` / `embedding` / `llm` / `plugin` / `schemas` 的第四列是**协作者视角**的子路径，
> 供直接定位；它们同样是「域的公开面」，只是按主题分了文件。**消费方不按这里深引**（第 4 条），
> 例外只有 `schemas`。

**域不按代码量分。** `clock`（25 行）与 `text`（49 行）各自成域，不并成一个「工具域」：
域名要**自证内容**，而一个能同时装下「纯函数」（`text`，输入决定输出）与「非确定性来源」
（`clock`，无输入、随系统时钟变化）的名字只能是杂物抽屉——它随后会收下所有「暂时没处放」
的东西，`keys` 一度收下插件清单正是这一类错放。合并判据见 [ADR-035](../../../docs/DECISIONS.md)。

## 3. 域级命名规则

§1.2 的域前缀表覆盖**绝大多数**符号。本节登记的是**整个域**层面的替代规则——
它们不是「某个符号的豁免」，而是该域**所有**符号都遵循的规则。

| 域 | 规则 | 理由 |
|---|---|---|
| `schemas` | 类型与常量**即协议词**，不带 `Schemas` 前缀 | 协议词就是它的命名空间；逐字段与前端镜像，改名要跨栈同步 |
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

1. **有几个依赖方？** 只有一个 → 下沉回那个模块，不进本层（ADR-023）。
2. **是契约还是实现？** 只有**一个**实现方认的东西（重试机器、流循环、拓扑知识）留在
   插件里；被**多个**模块共享的机制可以进本层（"依赖方数量"判据，见上）。
3. **放进哪个域、叫什么？** 先找职责最近的既有域；找不到再建新域——建域的成本是
   一个目录 + 一条 §1–§4 的完整合规，不要为单个 trait 开域。
4. **该是什么形态？** 契约 / 值对象 / 纯函数 / 全局单例 / provider——判据见
   [ADR-035](../../../docs/DECISIONS.md)。**共享不等于要抽 trait**：provider 化要同时满足
   「≥2 实现且编译期不知选哪个」「无状态或状态可共享」「按 id 装配而非处理数据」三条。
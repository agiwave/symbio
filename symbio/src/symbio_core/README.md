# symbio_core —— 内核契约层：命名与结构规范

本目录是**内核契约层 + 跨插件共享内核**：放跨插件共享的 trait、协议类型、词表常量、
纯工具函数，以及**被多个模块消费的共享机制**（如 `memory` 的记忆文件内核、`logger`
的结构化日志与级别闸门）。判据只有一个——**依赖方数量**：只被一个模块依赖的内容一律下沉回该
模块，不论它「够不够底层」（见 [ADR-023](../../../docs/DECISIONS.md)）。
本文是该目录**命名与结构**的唯一 owner。

> 「不放实现」是**方向**而非字面事实：域里既有纯契约（`vdfs` / `capability` / `llm`
> 的 trait 面），也有共享实现（`memory` 的值对象 + 纯函数、`event_bus` 的全局订阅表）。
> 判据是「谁依赖它」，不是「它抽象不抽象」——把共享机制硬抽成 trait
> 只会多一层间接（见 [`memory/mod.rs`](./memory/mod.rs) 的论述）。

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

   **默认手段**是域名前缀，三条细则：

   | 符号 | 形态 | 例 |
   |---|---|---|
   | 类型 / trait | 域名（PascalCase）+ 语义 | `ExecEventSink` · `VdfsNode` · `MemoryFile` · `CapabilityMeta` |
   | 常量 / 静态量 | 域名（SCREAMING_SNAKE）+ 语义 | `VDFS_ACTION_ABORT` · `EVENT_BUS_KIND_VDFS` · `ASSEMBLY_SUB_AGENT_PLUGINS` |
   | 函数 / 自由函数 | **不强求**前缀 | `resolve` · `to_wire` · `create_object` · `now_ms` |

   函数的导入行已经带着模块路径，前缀只是噪音。

   **三条被登记的替代手段**——它们与域名前缀**同等正式**，不是「例外」：

   | 手段 | 何时用 | 实例 |
   |---|---|---|
   | **子命名空间** | 域内按主题分了文件，且主题名比域名更有信息量 | `llm`：`Model*`（`model_provider.rs`）· `Turn*`（`turn.rs`）。`capability`：`Configurable*`（`configurable.rs`）· `Option*`（`option.rs`）· `Tool*`（`tools.rs`）。`plugin`：`ROUTE_*`（`route.rs`）· `TRAVERSE_*`（`traverse.rs`） |
   | **后缀** | 该域的类型名有一个比域名更强的类别词 | `keys` 的 `…Key`（`PathKey`）——`KeyPath` 会读成「键的路径」，语义反了 |
   | **登记缩写** | 域名的大写形式冗余且不增加信息 | `logger` → `LOG_`（`LOG_LEVEL_INFO` 已足够定位） |

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
4. **一个出口（根平铺重导出）**：域 `mod.rs` 的公开面被根 [`mod.rs`](./mod.rs) 平铺重导出，
   消费方一律写 `symbio_core::<符号>`。**唯一允许的深引是 `schemas::` 子树**——
   协议 schema 的词汇表就是它的命名空间，收敛成平铺反而丢失 `session::chat_message`
   这类语义。其余深路径（`symbio_core::plugin::dir::` 之类）视为不规范，应改为根平铺。

### 域前缀对照表（全 14 域 —— 新增符号照此取名）

这张表是第 2 条的**唯一执行口径**：拿不准新符号叫什么，先在这里查它的域。
「—」= 该域没有这一类符号（不是「不用前缀」）。**一个前缀只属于一个域。**
「裸名」= 不要求前缀（见 §3 的 `keys` 域规则）。

<!-- core-naming:modifiers Dyn,Default -->

| 域 | 类型 / trait | 常量 / 静态量 | 备注 |
|---|---|---|---|
| `assembly` | — | `ASSEMBLY_` | 本域只有两个常量 |
| `capability` | `Capability`；子命名空间 `Configurable*` · `Option*` · `Tool*` | — | 无常量；三个子命名空间各有对应文件 |
| `clock` | — | — | 只有 `now_ms` 一个函数 |
| `embedding` | `Embedding` | `EMBEDDING_` | 服务 id 在 `embedding/ids.rs` |
| `event_bus` | `EventBus` | `EVENT_BUS_` | |
| `exec` | `Exec` | — | 本域无常量 |
| `keys` | `…Key`（**后缀**） | **裸名**（实例） | 只有键：类型带 `Key` 后缀、实例裸名。**本域不收字符串常量** |
| `llm` | 子命名空间 `Model*` · `Turn*` | — | 对应 `model_provider.rs` / `turn.rs` |
| `logger` | — | `LOG_`（登记缩写） | |
| `memory` | `Memory` | `MEMORY_` | |
| `plugin` | `Plugin` | `PLUGIN_`；子命名空间 `ROUTE_*` · `TRAVERSE_*` | `PLUGIN_` 下细分：`PLUGIN_ID_*`（工厂 id）· `PLUGIN_KEY_*`（清单键）· `PLUGIN_FILE` · `PLUGIN_PAYLOAD_KEY` |
| `schemas` | 协议词 | 协议词 | 命名空间就是协议本身，见 §3 |
| `text` | — | — | 只有两个纯函数 |
| `vdfs` | `Vdfs` | `VDFS_` | |

> 判据始终是「**调用点读不读得出归属**」，表只是把结论固化。所以 `logger` 用 `LOG_`
> 而不是 `LOGGER_`（多出来的四个字母不增加任何信息）；而 `plugin` 域里 `PLUGIN_ID_*` /
> `PLUGIN_KEY_*` / `PLUGIN_FILE` 共用 `PLUGIN_`——它们同属一个域，细分段让「具体是什么」
> 也一眼可辨。
>
> 名字允许带一个**限定词**（上方的 `core-naming:modifiers` 标记列出它们）——限定词
> **不算**域前缀：`DynVdfsProvider` = `Dyn` + `VdfsProvider`，`DefaultToolVisitor` =
> `Default` + `ToolVisitor`。限定词表放在标记里而不是散文里，是为了让守卫能**解析**它
> （与 `plugin-entry-audit` 的 `<!-- vocab:… -->` 同一约定：**标了才认，没标不猜**）。
>
> 这张表由 `scripts/core-naming-audit.mjs` **机械核对**：它枚举本层公开面，逐个检查前缀
> 是否落在所属域登记的前缀里（并按「词」比对，`VDFS_PLUGIN_PROVIDER_FIELD` 不会因为含
> `PLUGIN_` 被误判）。改表要同时改代码，改代码要同时改表——规范不再是「靠人记」，
> 而是可执行的。

## 2. 域清单

| 域 | 职责 | 关键符号 | 子路径 |
|---|---|---|---|
| `assembly` | 装配策略：一棵标准插件树挂哪些插件、哪些插件不许被停用 | `ASSEMBLY_SUB_AGENT_PLUGINS` · `ASSEMBLY_UNDISABLABLE_PLUGINS` | — |
| `capability` | 能力系统：LLM 可见工具与插件遍历面 | `Capability` · `CapabilityMeta` · `CapabilityVisitor` · `CapabilityCategory` · `CapabilityToolContextRetention` · `invoke_capability` · 详情**三**表（`ConfigurableVisitor` / `OptionVisitor` / `DefaultToolVisitor`）· `resolve` / `to_wire` · `failure_kind` · 错误桶读写（`CapabilityError` / `report_error` / `take_errors`；**桶的键** `CAPABILITY_ERRORS` 住在 `keys`） | `configurable` · `error` · `option` · `tool_name` · `tools` |
| `clock` | 全项目「当前时间（Unix 毫秒）」唯一实现 | `now_ms` | — |
| `embedding` | 嵌入服务的**抽象**（实现在 `src/providers/embedding`） | `EmbeddingService` · `EmbeddingError` · `EMBEDDING_LOCAL` / `EMBEDDING_NOOP` | `ids` |
| `event_bus` | 跨插件全局发布设施门面 + 频道词表 | `EventBus` · `EventBusSubscribeRequest` · `EVENT_BUS_KIND_SYSTEM` · `EVENT_BUS_KIND_VDFS` · `EVENT_BUS_RESYNC_MARKER_TYPE` | — |
| `exec` | 执行期原语：事件出口（出）与中止信号（入） | `ExecEventSink` · `ExecAbortSignal` · `ExecEnv` · `ExecTranscriptWriter` | — |
| `keys` | **类型安全上下文键**（只有键：trait + 类型 + 实例） | `SymbioKey` 及其实例（`PATH` · `WORKDIR` · `ID` · `NAME` · `PLUGIN_DIR` · `CAPABILITY_VISITOR` · `CAPABILITY_ERRORS` …） | — |
| `llm` | 模型服务的唯一契约面（协议无关、插件无关）——只留 session 与 model **两侧共用**的符号 | `ModelProvider` · `ModelFinishReason` · `ModelUsage` · `TurnOutput` · `TurnToolCallAccumulator` · `TurnStreamChildIds` · `TurnToolCallInfo` · `emit_*` / `build_*` 家族 | `model_provider` · `turn` |
| `logger` | 结构化日志与级别闸门 | 日志宏 · `LOG_LEVEL_*`（`MIN_LEVEL` 是**私有**静态量，不是公开面） | — |
| `memory` | 「单文件长期记忆」共用内核（各层记忆同一份实现） | `MemoryFile` · `MemoryInjection` · `MemoryNodeSpec` · `MemorySegmentSpec` · `render_segment` · `MEMORY_AGENTS_FILE` | — |
| `plugin` | 插件核心契约：trait、信封、错误、目录、对象工厂、身份与地址 | `Plugin` · `PluginMeta` · `PluginInvokeRequest` / `PluginInvokeResponse` · `PluginError` / `PluginErrorCode` · `PluginChannel` / `PluginFrame` / `PluginPayload` / `PLUGIN_PAYLOAD_KEY` · `PluginDir` / `PluginConfigFile` / `PluginEntry` · `create_object` · `PLUGIN_ID_*`（工厂 id）· `PLUGIN_KEY_*` / `PLUGIN_FILE`（清单）· `ROUTE_*`（路由地址）· `TRAVERSE_AVAILABLE_*`（遍历端点） | `creator` · `dir` · `error` · `ids` · `route` · `transport` · `traverse` |
| `schemas` | 跨端协议 schema（前端逐字段镜像） | `ChatMessage` · `HookEvent` · `SuccessResponse` · 详情表 schema | `common` · `detail` · `hook` · `session` |
| `text` | 字符串安全截断（避免按字节切多字节字符 panic） | `truncate_bytes` · `floor_char_boundary` | — |
| `vdfs` | 统一资源访问契约（规范见 [design/vdfs.md](../../../docs/design/vdfs.md)） | `VdfsProvider` · `VdfsNode` · `VdfsRequest` / `VdfsResponse` · `VdfsError` · `VdfsChangeSubscriptions` · `VDFS_*` 词表 | `address` · `host` |

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
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
   里读起来无从知道它属于谁。三条细则：

   | 符号 | 形态 | 例 |
   |---|---|---|
   | 类型 / trait | 域名（PascalCase）+ 语义 | `ExecEventSink` · `VdfsNode` · `MemoryFile` · `CapabilityMeta` |
   | 常量 / 静态量 | 域名（SCREAMING_SNAKE）+ 语义 | `VDFS_ACTION_ABORT` · `EVENT_BUS_KIND_VDFS` · `ASSEMBLY_SUB_AGENT_PLUGINS` |
   | 函数 / 自由函数 | **不强求**前缀 | `resolve` · `to_wire` · `create_object` · `now_ms` |

   函数的导入行已经带着模块路径，前缀只是噪音。**子命名空间**：域内按主题分文件时，
   可用**主题前缀**代替域名前缀——但主题名必须**比域名更有信息量**，且必须在 §3 登记。
   `llm` 的 `Model*` / `Turn*`（对应 `model_provider` / `turn` 两个子模块）是唯一实例。

   判据是「调用点读不读得出归属」，不是「字面是否等于目录名」：`ChangeSubscriptions`
   曾住在 `vdfs` 却既不姓 `Vdfs` 也不属于任何已登记主题，调用点读不出它是 VDFS 的东西
   ⇒ 已改名 `VdfsChangeSubscriptions`。反过来的例子见 §3：`LOG_LEVEL_INFO` 不需要改成
   `LOGGER_LOG_LEVEL_INFO`，因为「日志级别」这个词本身已经把它指到了唯一的域。

   **放置也是同一条规则的一部分**：`SymbioKey` 实例（键对象）一律定义在 `keys`，
   机制与它的键分家——`capability/error.rs` 只放错误桶的读写函数，`CapabilityErrorsKey`
   住在 `keys/mod.rs`。
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

### 域前缀对照表（全 13 域 —— 新增符号照此取名）

这张表是第 2 条的**唯一执行口径**：拿不准新符号叫什么，先在这里查它的域。
「—」= 该域没有这一类符号（不是「不用前缀」）。

| 域 | 类型 / trait | 常量 / 静态量 | 备注 |
|---|---|---|---|
| `assembly` | — | `ASSEMBLY_` | 本域只有两个常量 |
| `capability` | `Capability` | `CAPABILITY_` | `ConfigurableVisitor` / `OptionVisitor` / `Default*Visitor` 是登记词，见 §3 |
| `clock` | — | — | 只有 `now_ms` 一个函数 |
| `embedding` | `Embedding` | — | 本域无常量 |
| `event_bus` | `EventBus` | `EVENT_BUS_` | |
| `exec` | `Exec` | — | 本域无常量 |
| `keys` | `…Key`（**后缀**） | `KEY_` · `PLUGIN_` · `EMBEDDING_` · `<PLUGIN>_<OP>` | 键**实例**用裸名，见 §3 |
| `llm` | `Model` · `Turn` | — | **子命名空间**：`Model`→`model_provider`，`Turn`→`turn` |
| `logger` | — | `LOG_` | 域名词取 `LOG` 而非 `LOGGER`——「日志级别」只有一个域 |
| `memory` | `Memory` | `MEMORY_` | |
| `plugin` | `Plugin` | `KEY_` | 清单键词表。**刻意不用 `PLUGIN_`**：那个前缀已被 `keys` 的插件 id 家族（`PLUGIN_SESSION` …）占用，两者共用会变成「一个前缀两种东西」 |
| `schemas` | 豁免 | 豁免 | 协议词即命名空间，见 §3 |
| `text` | — | — | 只有两个纯函数 |
| `vdfs` | `Vdfs` | `VDFS_` | 注册表字段名（`PLUGIN_PROVIDER_FIELD` 等）见 §3 |

> 判据始终是「**调用点读不读得出归属**」，表只是把结论固化。所以 `logger` 用 `LOG_`
> 而不是 `LOGGER_`（多出来的四个字母不增加任何信息），而 `plugin` 的清单键宁可另起
> `KEY_` 也不去撞 `PLUGIN_`（撞了反而更难读）。

## 2. 域清单

| 域 | 职责 | 关键符号 | 子路径 |
|---|---|---|---|
| `assembly` | 装配策略：一棵标准插件树挂哪些插件、哪些插件不许被停用 | `ASSEMBLY_SUB_AGENT_PLUGINS` · `ASSEMBLY_UNDISABLABLE_PLUGINS` | — |
| `capability` | 能力系统：LLM 可见工具与插件遍历面 | `Capability` · `CapabilityMeta` · `CapabilityVisitor` · `CapabilityCategory` · `invoke_capability` · 详情**三**表（`ConfigurableVisitor` / `OptionVisitor` / `DefaultToolVisitor`）· `resolve` / `to_wire` · `failure_kind` · 错误桶读写（`CapabilityError` / `report_error` / `take_errors`；**桶的键** `CAPABILITY_ERRORS` 住在 `keys`） | — |
| `clock` | 全项目「当前时间（Unix 毫秒）」唯一实现 | `now_ms` | — |
| `embedding` | 嵌入服务的**抽象**（实现在 `src/providers/embedding`） | `EmbeddingService` · `EmbeddingError` | — |
| `event_bus` | 跨插件全局发布设施门面 + 频道词表 | `EventBus` · `EventBusSubscribeRequest` · `EVENT_BUS_KIND_SYSTEM` · `EVENT_BUS_KIND_VDFS` · `EVENT_BUS_RESYNC_MARKER_TYPE` | — |
| `exec` | 执行期原语：事件出口（出）与中止信号（入） | `ExecEventSink` · `ExecAbortSignal` · `ExecEnv` · `ExecTranscriptWriter` | — |
| `keys` | 类型安全上下文键：键 trait + 键实例 + 插件 id + 路由地址常量 | `SymbioKey` 及其实例（`PATH` · `WORKDIR` · `ID` · `NAME` · `CAPABILITY_VISITOR` · `CAPABILITY_ERRORS` …）· `PLUGIN_*` · `EMBEDDING_*` · `KEY_PAYLOAD` · `<PLUGIN>_<OP>` 地址常量 | `ids` · `paths` |
| `llm` | 模型服务的唯一契约面（协议无关、插件无关）——只留 session 与 model **两侧共用**的符号 | `ModelProvider` · `ModelFinishReason` · `ModelUsage` · `TurnOutput` · `TurnToolCallAccumulator` · `TurnStreamChildIds` · `TurnToolCallInfo` · `emit_*` / `build_*` 家族 | `model_provider` · `turn` |
| `logger` | 结构化日志与级别闸门 | 日志宏 · `LOG_LEVEL_*`（`MIN_LEVEL` 是**私有**静态量，不是公开面） | — |
| `memory` | 「单文件长期记忆」共用内核（各层记忆同一份实现） | `MemoryFile` · `MemoryInjection` · `MemoryNodeSpec` · `MemorySegmentSpec` · `render_segment` · `MEMORY_AGENTS_FILE` | — |
| `plugin` | 插件核心契约：trait、信封、错误、目录与对象工厂 | `Plugin` · `PluginMeta` · `PluginInvokeRequest` / `PluginInvokeResponse` · `PluginError` / `PluginErrorCode` · `PluginChannel` / `PluginFrame` / `PluginPayload` · `PluginDir` / `PluginConfigFile` / `PluginEntry` · `create_object` | `creator` · `dir` · `error` · `transport` |
| `schemas` | 跨端协议 schema（前端逐字段镜像） | `ChatMessage` · `HookEvent` · `SuccessResponse` · 详情表 schema | `common` · `detail` · `hook` · `session` |
| `text` | 字符串安全截断（避免按字节切多字节字符 panic） | `truncate_bytes` · `floor_char_boundary` | — |
| `vdfs` | 统一资源访问契约（规范见 [design/vdfs.md](../../../docs/design/vdfs.md)） | `VdfsProvider` · `VdfsNode` · `VdfsRequest` / `VdfsResponse` · `VdfsError` · `VdfsChangeSubscriptions` · `VDFS_*` 词表 | `address` · `host` |

> `keys` / `llm` / `plugin` / `schemas` 的第三列是**协作者视角**的子路径，供直接定位；
> 它们同样是「域的公开面」，只是按主题分了文件。**消费方不按这里深引**（第 4 条），
> 例外只有 `schemas`。

**域不按代码量分。** `clock`（25 行）与 `text`（49 行）各自成域，不并成一个「工具域」：
域名要**自证内容**，而一个能同时装下「纯函数」（`text`，输入决定输出）与「非确定性来源」
（`clock`，无输入、随系统时钟变化）的名字只能是杂物抽屉——它随后会收下所有「暂时没处放」
的东西，`keys` 一度收下插件清单正是这一类错放。合并判据见 [ADR-035](../../../docs/DECISIONS.md)。

## 3. 豁免词表

§1.2 的域前缀表已覆盖**域一级**的取名；本表登记的是**单个符号级**的例外——它们要么是
协议词本身，要么是已被大量跨插件消费方按现名引用的登记词。新增符号默认**不**享受豁免，
需要时在此登记（**先查 §1.2 的表，查不到再查本表**）。

| 豁免 | 内容 | 理由 |
|---|---|---|
| 协议 schema | `schemas::*` 的全部类型与常量 | 协议词即命名空间；逐字段与前端镜像，改名要跨栈同步 |
| 键面字面量 | `keys/ids.rs` 的 `PLUGIN_*`（插件 id）· `EMBEDDING_*` · `KEY_PAYLOAD`；`keys/paths.rs` 的 `<PLUGIN>_<OP>` 地址常量 | 线上键名与 id 是**跨进程字面量**，改的是值不是名字 |
| `PLUGIN.yml` 清单键 | `plugin/dir.rs` 的 `KEY_*` · `RESERVED_KEYS` · `PLUGIN_FILE` | 清单 schema 的闭集词表（值一律 `plugin_*`）。**归属是 `plugin` 不是 `keys`**——本表曾把它们记在 `keys` 名下，是笔误 |
| 类型化上下文键 | `PATH` · `WORKDIR` · `ID` · `NAME` · `KIND` · `SCOPE` · `CONTENT` · `DESCRIPTION` · `MODE` · `AGENT_ID` · `SESSION_ID` · `TRACE_ID` · `PROVIDER_ID` · `RISK_LEVEL` · `TOOL_CALL_ID` · `RESULT_MSG_ID` · `VDFS_PARENT_ADDR` · `PARENT` · `CONFIG` · `PLUGIN_DIR` · `REQUIRED_PLUGINS` · `CAPABILITY_VISITOR` · `OPTION_VISITOR` · `CONFIG_VISITOR` · `EVENT_SINK` · `ABORT_SIGNAL` · `CAPABILITY_ERRORS` | 见下方「两条命名规则」——它们是**实例**不是名字，加 `KEY_` 前缀会与字符串键名撞成一个前缀。（它们的**类型** `PathKey` / `IdKey` … 由 §1.2 表里的 `…Key` 后缀覆盖，不在此重复登记。） |
| 遍历端点 | `TRAVERSE_AVAILABLE_TOOLS` · `TRAVERSE_AVAILABLE_OPTIONS` | 协议端点（非插件路径），见 [design/plugin-route-address.md](../../../docs/design/plugin-route-address.md) |
| 详情**三**表 | `ConfigurableVisitor` · `OptionVisitor` · `DefaultConfigurableVisitor` · `DefaultToolVisitor` · `entry_of` · `announce_configurable` | 跨插件登记词，已成为稳定词汇。**原为「四表」**：`DefaultOptionVisitor` 与 `collect_options` 只有会话宿主一个消费方，已按 ADR-023 下沉到 `plugins/session/options.rs` |
| VDFS 注册表字段名 | `PLUGIN_PROVIDER_FIELD` · `VDFS_PLUGIN_NAME_FIELD` · `VDFS_PLUGINS_FIELD` | 线上协议字面量（同上）；`PLUGIN_` 在这里指「注册表里的插件条目」，与 `keys` 的插件 id 同名不同义，故保留原样 |
| 函数与自由函数 | `resolve` · `to_wire` · `create_object` · `emit_*` · `build_*` · `now_ms` … | 导入行已含模块路径，前缀是噪音 |

### 两条命名规则 —— `keys` 域的「两种形态」不是「两套风格」

`keys` 域里同时有 `KEY_PAYLOAD`（`&str`）与 `PATH`（`PathKey`），看似风格分裂；
逐行 `grep "^pub const" keys/*.rs` 可验证它其实是一条**有判别力的规则**：

| 形态 | 一律写成 | 它是什么 | 例 |
|---|---|---|---|
| **字符串常量** | **带前缀** | **名字**：跨进程 / 跨文件的字面量 | `PLUGIN_SESSION` · `EMBEDDING_LOCAL` · `KEY_PAYLOAD` · `VDFS_ROOT` |
| **`SymbioKey` 实例** | **裸名** | **键对象**：某类型的唯一实例 | `PATH: PathKey` · `ID: IdKey` · `CAPABILITY_VISITOR: CapabilityVisitorKey` |

判别力来自两侧**互不重叠**：一个常量要么是 `&str`，要么是某个 `XxxKey` 类型的实例，没有第三种。
消费形态也不同——字符串是**值**（`name == KEY_PAYLOAD`），实例是**取值的凭据**（`ctx.get(&PATH)`）。

把裸名改成 `KEY_PATH` 会让 `KEY_` 同时指代「字符串名字」与「键对象」——**用一个统一前缀
换掉一个真实的区分**，是净亏损。故**维持裸名，在此登记豁免**；新增键时照上表选形态
（新增 `SymbioKey` 实例用裸名，新增字面量用前缀）。

## 4. 新增内容时的四问

1. **有几个依赖方？** 只有一个 → 下沉回那个模块，不进本层（ADR-023）。
2. **是契约还是实现？** 只有**一个**实现方认的东西（重试机器、流循环、拓扑知识）留在
   插件里；被**多个**模块共享的机制可以进本层（"依赖方数量"判据，见上）。
3. **放进哪个域、叫什么？** 先找职责最近的既有域；找不到再建新域——建域的成本是
   一个目录 + 一条 §1–§4 的完整合规，不要为单个 trait 开域。
4. **该是什么形态？** 契约 / 值对象 / 纯函数 / 全局单例 / provider——判据见
   [ADR-035](../../../docs/DECISIONS.md)。**共享不等于要抽 trait**：provider 化要同时满足
   「≥2 实现且编译期不知选哪个」「无状态或状态可共享」「按 id 装配而非处理数据」三条。
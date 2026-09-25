# symbio_core —— 内核契约层：命名与结构规范

本目录是**内核契约层 + 跨插件共享内核**：放跨插件共享的 trait、协议类型、词表常量、
纯工具函数，以及**被多个模块消费的共享机制**（如 `memory` 的记忆文件内核、`homedir`
的系统根注册表）。判据只有一个——**依赖方数量**：只被一个模块依赖的内容一律下沉回该
模块，不论它「够不够底层」（见 [ADR-023](../../../docs/DECISIONS.md)）。
本文是该目录**命名与结构**的唯一 owner。

> 「不放实现」是**方向**而非字面事实：域里既有纯契约（`vdfs` / `capability` / `llm`
> 的 trait 面），也有共享实现（`memory` 的值对象 + 纯函数、`homedir` 的全局单例与
> bootstrap）。判据是「谁依赖它」，不是「它抽象不抽象」——把共享机制硬抽成 trait
> 只会多一层间接（见 [`memory/mod.rs`](./memory/mod.rs) 的论述）。

各域职责、域清单与豁免词表在下面的 §1–§3；单个域的机制写在它的 `mod.rs` 文档头里
（本文不复述）。

## 1. 域模型（四条标准）

一个**域** = 本目录下一层目录，对应一个职责上自洽的概念面。四条标准同时成立才叫合规：

1. **一域一目录，域内私有**：域内子模块一律 `mod x;`（私有），公开面在该域 `mod.rs` 里
   逐符号 `pub use` 列出。域的公开面因此**可在一处读完**——要改对外形状只动这一屏。
   子模块可以深到 2–3 层（如 `llm/turn.rs`、`plugin/dir.rs`），但**没有**「域内再分域」的
   特权：判断标准是「它有没有独立的对外面」，不是「文件大不大」。
2. **符号带域前缀**：类型与常量的名字以域名开头（`ExecEventSink` / `PluginErrorCode` /
   `VdfsNode` / `MemoryFile` / `CapabilityMeta`）。理由不是好看，而是**调用点自证归属**：
   本层符号在全仓按根平铺导入（见第 4 条），`EventSink` 这种名字在 `plugins/model/*` 里
   读起来无从知道它属于谁。函数与自由函数不强求前缀（`resolve` / `to_wire` /
   `create_object`），因为它们的导入行已经带着模块路径；不适用前缀的符号见 §3 豁免词表。
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

## 2. 域清单

| 域 | 职责 | 关键符号 | 子路径 |
|---|---|---|---|
| `capability` | 能力系统：LLM 可见工具与插件遍历面 | `Capability` · `CapabilityMeta` · `CapabilityVisitor` · `CapabilityCategory` · `invoke_capability` · 详情四表（`ConfigurableVisitor` / `OptionVisitor` / `DefaultToolVisitor`）· `resolve` / `to_wire` · `failure_kind` · 错误桶（`CapabilityError` / `report_error` / `take_errors`） | — |
| `clock` | 全项目「当前时间（Unix 毫秒）」唯一实现 | `now_ms` | — |
| `embedding` | 嵌入服务的**抽象**（实现在 `src/providers/embedding`） | `EmbeddingService` · `EmbeddingError` | — |
| `event_bus` | 跨插件全局发布设施门面 + 频道词表 | `EventBus` · `EventBusSubscribeRequest` · `EVENT_BUS_KIND_SYSTEM` · `EVENT_BUS_KIND_VDFS` · `EVENT_BUS_RESYNC_MARKER_TYPE` | — |
| `exec` | 执行期原语：事件出口（出）与中止信号（入） | `ExecEventSink` · `ExecAbortSignal` · `ExecEnv` · `ExecTranscriptWriter` | — |
| `homedir` | 系统根目录（homedir）运行时注册表与 bootstrap | `HomedirRegistry` · `expand_tilde_path` · `DEFAULT_HOMEDIR` | — |
| `keys` | 类型安全上下文键：键 trait + 插件 id + 路由地址常量 | `SymbioKey` · `PLUGIN_*` · `<PLUGIN>_<OP>` 地址常量 | `ids` · `paths` |
| `llm` | 模型服务的唯一契约面（协议无关、插件无关） | `ModelProvider` · `ModelProtocolEvent` · `ModelFinishReason` · `ModelUsage` · `SseLineParser` · `TurnOutput` · `emit_*` / `build_*` 家族 | `model_provider` · `sse` · `turn` |
| `logger` | 结构化日志与级别闸门 | 日志宏 · `MIN_LEVEL` · `LOG_LEVEL_*` | — |
| `memory` | 「单文件长期记忆」共用内核（各层记忆同一份实现） | `MemoryFile` · `MemoryInjection` · `MemoryNodeSpec` · `MemorySegmentSpec` · `render_segment` · `MEMORY_AGENTS_FILE` | — |
| `plugin` | 插件核心契约：trait、信封、错误、目录与对象工厂 | `Plugin` · `PluginMeta` · `PluginInvokeRequest` / `PluginInvokeResponse` · `PluginError` / `PluginErrorCode` · `PluginChannel` / `PluginFrame` / `PluginPayload` · `PluginDir` / `PluginConfigFile` / `PluginEntry` · `create_object` | `creator` · `dir` · `error` · `transport` |
| `schemas` | 跨端协议 schema（前端逐字段镜像） | `ChatMessage` · `HookEvent` · `SuccessResponse` · 详情表 schema | `common` · `detail` · `hook` · `session` |
| `text` | 字符串安全截断（避免按字节切多字节字符 panic） | `truncate_bytes` · `floor_char_boundary` | — |
| `vdfs` | 统一资源访问契约（规范见 [design/vdfs.md](../../../docs/design/vdfs.md)） | `VdfsProvider` · `VdfsNode` · `VdfsRequest` / `VdfsResponse` · `VdfsError` · `VDFS_*` 词表 | `address` · `host` |

> `keys` / `llm` / `plugin` / `schemas` 的第三列是**协作者视角**的子路径，供直接定位；
> 它们同样是「域的公开面」，只是按主题分了文件。**消费方不按这里深引**（第 4 条），
> 例外只有 `schemas`。

## 3. 豁免词表

以下符号**刻意不带域前缀**——它们要么是协议词本身，要么已被大量跨插件消费方按现名引用，
改名不会带来可读性收益、只带来漂移风险。新增符号默认**不**享受豁免，需要时在此登记。

| 豁免 | 内容 | 理由 |
|---|---|---|
| 协议 schema | `schemas::*` 的全部类型与常量 | 协议词即命名空间；逐字段与前端镜像，改名要跨栈同步 |
| 键面常量 | `keys` 的 `KEY_*` / `RESERVED_KEYS` / `PLUGIN_FILE` / `PLUGIN_*`；`keys::paths` 的地址常量 | 线上键名与 id 是**跨进程字面量**，改的是值不是名字 |
| 遍历端点 | `TRAVERSE_AVAILABLE_TOOLS` · `TRAVERSE_AVAILABLE_OPTIONS` | 协议端点（非插件路径），见 [design/plugin-route-address.md](../../../docs/design/plugin-route-address.md) |
| 详情四表 | `ConfigurableVisitor` · `OptionVisitor` · `Default*Visitor` · `entry_of` · `announce_configurable` · `collect_options` | 跨插件登记词，已成为稳定词汇 |
| VDFS 词表 | `VDFS_*` · `PLUGIN_PROVIDER_FIELD` · `VDFS_PLUGIN_NAME_FIELD` 等 | 线上协议字面量（同上） |
| 函数与自由函数 | `resolve` · `to_wire` · `create_object` · `emit_*` · `build_*` · `now_ms` … | 导入行已含模块路径，前缀是噪音 |

## 4. 新增内容时的三问

1. **有几个依赖方？** 只有一个 → 下沉回那个模块，不进本层（ADR-023）。
2. **是契约还是实现？** 只有**一个**实现方认的东西（重试机器、流循环、拓扑知识）留在
   插件里；被**多个**模块共享的机制可以进本层（"依赖方数量"判据，见上）。
3. **放进哪个域、叫什么？** 先找职责最近的既有域；找不到再建新域——建域的成本是
   一个目录 + 一条 §1–§4 的完整合规，不要为单个 trait 开域。
# Symbio 架构决策记录 (ADRs)

> **文档类型：阐述** — 记录"为什么这样设计"的关键决策。

## ADR-001: 分形插件架构

**状态**：已接受

**背景**：
需要一种架构，使新增功能无需修改核心代码，且能灵活组合。

**决策**：
采用分形插件架构，容器与叶子使用相同接口。

**理由**：
- 自相似性：任何插件可被容器包裹，增加中间件行为
- 可组合性：新能力只需实现 `Plugin` 并注册
- 可测试性：每个插件可独立测试

**后果**：
- 路由解析有运行时开销 (可忽略)
- 需要 `inventory` 静态注册机制

---

## ADR-002: 路径即路由

**状态**：已接受

**背景**：
需要一种方式让 LLM 直接调用工具，无需代码生成。

**决策**：
用 `/` 分隔的字符串定位能力 (如 `agent/chat`)。

**理由**：
- LLM 输出字符串即可调用工具
- 新增能力无需修改路由逻辑
- 工具名自带命名空间，避免冲突

**后果**：
- 路径是运行时字符串，无编译期检查
- 需要文档维护路径清单

---

## ADR-003: Session 作为编排入口

**状态**：已接受

**背景**：
早期 `agent` 插件独占会话编排，导致耦合严重。

**决策**：
重构后 `session` 成为唯一编排入口，`agent` 只负责"人格"。

**理由**：
- 关注点分离：Agent 负责人格，Session 负责对话
- 可组合性：同一 Session 可绑定不同 Agent
- 灵活性：支持无 Agent 的纯工具模式

**后果**：
- Session 需要收集全树工具
- Agent 插件需要适配为被动角色

---

## ADR-004: 多协议 LLM 适配

**状态**：已接受

**背景**：
需要对接多家 LLM 供应商，各家 API 格式不同。

**决策**：
Model 插件内置 4 套协议适配器 (OpenAI Chat, OpenAI Responses, Anthropic, Gemini)。

**理由**：
- 供应商无关：同一套代码对接多家
- 协议演进：OpenAI 从 Chat 演进到 Responses
- 功能差异：各家工具调用格式不同

**后果**：
- Model 插件代码量较大
- 新增协议需要添加适配器

---

## ADR-005: OAB 协议宿主实现

**状态**：已接受

**背景**：
Agent 需要一种标准化的打包与分发格式。

**决策**：
实现 OAB (Open Agent Bundle) 协议，作为 Agent 插件的核心。

**理由**：
- 标准化：约定目录结构 (prompts/, skills/, mcps/)
- 可分发：zip 格式打包
- 可扩展：新增能力通过 MCP 接入

**后果**：
- Agent 插件变为 OAB 宿主
- 需要维护 OAB 协议规范

---

## ADR-006: 薄宿主层设计

**状态**：已接受

**背景**：
Tauri 前端需要与 Rust 核心库通信。

**决策**：
Tauri 后端仅暴露 3 个命令 (`route_v2`, `route_v2_send`, `route_v2_close`)。

**理由**：
- 极简适配层：所有业务逻辑在核心库
- 前端无业务规则：只做配置与展示
- 可替换：Tauri 可被其他宿主替代

**后果**：
- 前端无法直接调用插件，必须通过路由
- 需要 IPC 序列化开销

---

## ADR-007: 静态注册 (inventory)

**状态**：已接受

**背景**：
插件需要一种方式注册到全局工厂。

**决策**：
使用 `inventory` crate 实现编译期静态注册。

**理由**：
- 零配置：新增插件无需修改注册代码
- 编译期保证：未注册插件链接期报错
- 惰性初始化：首次使用时收集

**后果**：
- 依赖 `inventory` crate
- 注册顺序不确定 (通常无关)

---

## ADR-008: 多存储后端

**状态**：已接受

**背景**：
Agent 认知数据需要持久化。

**决策**：
支持 DirStorage (多 YAML 文件) 和 SQLite 两种后端，可热切换。

**理由**：
- DirStorage：人类可读，便于调试
- SQLite：高性能，支持复杂查询
- 可切换：按场景选择

**后果**：
- 需要维护两套存储实现
- 数据迁移需要工具

**澄清（避免命名混淆）**：本 ADR 说的「多后端」指的是**会话存储**——
`SessionStore` 的三个实现由会话配置项 `store_kind` 选择（`file` 默认 / `sqlite` /
`memory`，工厂 `plugins/session/store/mod.rs::create_store`）。它与已废除的
`StorageService` / `EntityStore`（旧 `providers/storage_service`）**不是一回事**，
也和 `~/.symbio/plugins/<类别>/<id>/<主文件>` 的资源存储无关——后者**没有**第二种后端、
不可配置（见 ADR-011）。

---

## ADR-009: 机制化认知 (v9)

**状态**：已接受

**背景**：
Agent 认知类型与关系类型经常变化。

**决策**：
关系类型与展示行为由 `prop` CU 驱动，不在核心代码硬编码。

**理由**：
- 可扩展：新增认知类型无需改核心代码
- 数据驱动：同一份 `seed_cus.jsonl` 驱动解析与展示
- 灵活：用户可自定义认知模型

**后果**：
- 启动时需要加载 seed 数据
- 调试更复杂 (行为由数据决定)

---

## ADR-010: 统一实体管理

**状态**：已被取代（S11 起 `entities/*` 调用协议下线；VDFS 收敛期结束后
后端 `EntityProvider` 抽象与 `EntityVdfsAdapter` **一并删除**，各插件直接实现
`VdfsProvider`，见 [design/vdfs.md](./design/vdfs.md) §13.4）

**背景**：
多种实体 (Agent, Model, Session, MCP, Skill) 需要 CRUD 操作。

**决策**：
统一为 `entities/list|get|upload|delete|status` 契约。

**理由**：
- 前端一份 `EntityManagerView` 实例化多类
- 后端渐进实现，无需改协议
- 降低前端复杂度

**后果**：
- 所有实体共享相同接口，可能有个别实体特殊需求
- 需要 `EntityCapabilities` 能力开关区分

**后续（已取代）**：该契约与 `EntityCapabilities` 均已下线——`entities/*` 随
S11 停止路由，能力开关随 S12 删除。资源访问统一经 VDFS（`.vdfs/<kind>/…`）：
能力改由**访问位**、注册表（supports_upload / supports_import）与**声明式动作**
（`vdfs/action`）表达，前端页面只剩一台三栏工作台。理由（一份页面实例化多类）
由 VDFS 以更强的方式满足——**一份机制、零类型知识**。

**收敛终局（EntityProvider 下线）**：过渡期保留的 `EntityProvider` trait（20 个
钩子）与 `EntityVdfsAdapter`（把 trait 接成 VDFS 子目录）在收敛期结束时删除：
每个资源插件**直接实现 `VdfsProvider`**，用现成的 `list` / `stat` / `read` /
`write` / `delete` / `action` / `watch` 表达自身语义；跨插件共用的只剩
`symbio_core::entities` 里的**存储原语自由函数**（写盘 / 删除 / 导入 / 导出，
无 trait 约束）。

理由：trait + 适配器这一层曾以「新增实体类型 VDFS 侧零改动」为价值主张，但
实际代价是把**每类资源的差异**（清单来源、摘要口径、manifest 校验、内存同步、
容器语义）挤进一张 20 钩子的通用接口里，再由一个 1500 行的适配器去猜；
去掉它之后，每类资源的语义回到自己的 `impl VdfsProvider` 里，一眼可见、
改一处只影响一处。

**终局（存储抽象层一并废除）**：ADR-010 收敛后仍留着两套并行的资源访问抽象——
对外是 VDFS，对内是 `StorageService` / `EntityStore`（`providers/storage_service`）
加一组存储原语自由函数（`symbio_core::entities`）。问题不是「多了一层」，而是
**那一层讲的不是 VDFS 的话**：磁盘资源用一套私有 trait（`list_entities` /
`read_entity` / `write_entity`）表达，再由每个插件手翻成 `VdfsNode` / `VdfsContent`。
现已整层删除，资源存储 = `VdfsProvider` 的三个集中实现（ADR-011）；`entity` 词汇
在后端清零（`EntitySummary` / `EntityUploadResponse` / `EntityExport` /
`ENTITY_*` 常量与 `kind = "entity"` 事件频道全部删除），只剩
`schemas/entities.rs` 里的 `DetailDefinition` 表单方言（它是 VDFS `ext = form` 的
宿主方言，与「实体」无关）。

---

## ADR-011: 资源存储 = `VdfsProvider` 的集中实现

**状态**：已接受

**背景**：
ADR-010 删掉了「差异集中在一张 trait」的适配层，但落盘那一层仍是**与 VDFS 并行的
第二套抽象**：`providers/storage_service`（`StorageService` + `EntityStore` +
`FileEntityStore` + `path_resolver::safe_id`）与 `symbio_core::entities` 的 13 个
存储原语自由函数。每接一类资源就要「实现一次存储抽象 + 写一次到 VDFS 形状的翻译」，
且 `StorageService` 这个名字与 session 侧的多后端存储（ADR-008）极易混淆。

**决策**：
在宿主实现层新增 `symbio/src/providers/vdfs_service/`，提供**基于 `VdfsProvider`
接口的三个集中实现**——`SingleFileVdfs`（一个条目 = 一份主文件，条目内部不外露）、
`DirVdfs`（一个条目 = 一个目录，可下钻，主文件承载条目内容）、`MemoryVdfs`
（条目只在进程内，不落盘）。三者共用 `entry.rs` 的条目寻址与落盘原语、`pack.rs`
的 zip / base64 与导出载荷 `VdfsPack`。**磁盘布局一字未改**
（`<homedir>/plugins/<category>/<id>/<manifest>`）。同时删除
`providers/storage_service`、`symbio_core::entities`、
`symbio_core/providers/storage.rs`（含 `categories` / `manifests` 常量）与
`symbio_core/schemas/entities.rs` 里的协议时代类型。

**理由**：
- **不再有两套并行的资源访问抽象**：存储实现本身就是 `VdfsProvider`，
  「一类资源 = 一份存储抽象 + 一份翻译」收缩为「一类资源 = 一次挂载点选型」。
- **换拓扑不动数据、换类别不碰协议**：三型的差别只在访问拓扑，磁盘上同一份布局。
- **不是重蹈 EntityProvider 覆辙**：这里**没有**通用钩子 trait、没有注册表、没有
  适配器去猜资源形状（「有目录就当容器」那类规则的反面）。三个各自**完整**的具体
  实现，差异（标题、状态、`ext`、`schema`、写前校验、写后内存同步）由调用点以普通
  Rust 参数显式传入。之所以也**不走** `create_object` 工厂：不存在第二种实现，
  套 `dyn` 只是把一次构造换成一次字符串查表。
- **协议零改动**：`symbio_core/vdfs_provider.rs`（纯接口）与 `plugins/vdfs/*`
  （协议 / 访问层 / 物理层）本次**未修改**——这是对边界刻意的自我约束。

**后果**：
- `notify_change` 只报 `(kind, path, change)` 三元组、**不携带载荷**（不为此扩展
  core 协议）。因此 `created` / `updated` / `deleted` 一类粗粒度变更，消费者只能
  **防抖重拉**；带载荷的增益投递只存在于 provider 自己实现的 `watch` 里
  （如会话消息的 `appended` + `delta`）。前端由此退掉第二条 `kind = "entity"` 订阅。
- **三种拓扑要显式选**：挂载点必须声明自己用哪一型、主文件叫什么
  （类别段名 = 插件名 = `PLUGIN_*`，主文件名 = 插件内部 `const MANIFEST`）；
  机制不去目录里猜。选错了表现为「条目内部不该外露却外露」这类可见问题，
  而不是静默错乱。
- 「manifest 补齐 id」这条不变量不再有唯一实现，`model` / `mcp` 各持一份 `with_id`
  （有意为之：它是该资源的写入语义，不是跨插件共享原语）。
- 内存镜像与磁盘清单**同一条广播频道、同一套操作语义**，消费者无需区分条目住在哪儿。

---

> **维护原则**：每个架构决策必须记录在此，包括背景、决策、理由、后果。

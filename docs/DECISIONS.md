# Symbio 架构决策记录 (ADRs)

> **文档类型：阐述** — 记录"为什么这样设计"的关键决策。

## ADR-001: 分形插件架构

**状态**：已接受

> **当前状态**：**已实现（现行）**。容器与叶子同接口，`home` → `worker(composite)` → 14 个业务插件即为这棵树（总 16 个插件 = 根 `home` + 容器 `composite` + 14 叶子，清单见 [CURRENT.md](./CURRENT.md) §1）。

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

> **当前状态**：**已实现（现行）**，但资源与配置**不再走 `{plugin}/{action}`**——统一经 `vdfs/*` + `<根>/…` 地址（见 ADR-010 / ADR-011）。

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

> **当前状态**：**已实现（现行）**。会话编排唯一入口在 `plugins/session/`，`chat_loop` 为其核心循环。

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

> **当前状态**：**部分实现 / 形态已变**。4 套适配器（`openai_chat` / `openai_responses` / `anthropic_messages` / `gemini_api`）**均真实存在**于 `plugins/model/protocols/`；但 `model` 已**无自有路由**——收为无状态单轮网关 `execute_turn`，由 `session` 在循环内直连调用。

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

## ADR-005: Agent 目录宿主实现

**状态**：已接受

> **当前状态**：**已演进**。Agent 插件仍是 agent 目录宿主、导入仍走 `<根>/agent` 新建类型 `zip`；
> 但 OAB v1 的**约定目录协议**（`prompts/` `skills/` `mcps/` 由宿主硬编码解释）已被
> [`agent-dir/v2`](./design/agent-directory-spec.md) 取代——**Agent 就是一棵插件树**：
> 技能 / MCP 复用宿主既有的 `skill` / `mcp` 插件目录，人格改为根 `AGENTS.md`。
> v1 规范见 [archive/open-agent-bundle-spec.md](./archive/open-agent-bundle-spec.md)。

**背景**：
Agent 需要一种标准化的打包与分发格式。

**决策**：
Agent 插件作为智能体域的唯一所有者：托管 agent 目录、经 `<根>/agent` 的 `zip` 新建类型导入、并装配为子树；OAB v1 的约定目录协议已废弃。

**理由**：
- 标准化：约定目录结构 (prompts/, skills/, mcps/)
- 可分发：zip 格式打包
- 可扩展：新增能力通过 MCP 接入

**后果**：
- Agent 插件成为 agent 目录宿主
- 需要维护 agent 目录规范（agent-dir/v2）

---

## ADR-006: 薄宿主层设计

**状态**：已接受

> **当前状态**：**已实现**。Tauri 后端仍仅 `route_v2` / `route_v2_send` / `route_v2_close` 三个 command（`tauri/src-tauri/src/main.rs` 的 `generate_handler!`，`meta` 已注释停用）。

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

> **当前状态**：**已实现（现行）**。`submit_object_creator!` + `inventory` 收集。

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

> **当前状态**：**已回退（会话存储部分）**。`store_kind` 与 `file` / `sqlite` / `memory` 三后端**已删除**，会话存储收为单一具体类型（`session/store/mod.rs::SessionStore`，持久=磁盘布局 / 临时=进程内驻留）。下方「澄清」段所述 `store_kind` 选型已是历史口径，以本行为准；资源存储侧见 ADR-011。

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
也和 `~/.symbio/<类别>/<id>/<主文件>` 的资源存储无关——后者**没有**第二种后端、
不可配置（见 ADR-011）。

---

## ADR-009: 机制化认知 (v9)

**状态**：已接受

> **当前状态**：**未落地 / 已回退**。CU（认知单元）+ `prop` 驱动的认知层已从代码中移除——无 `seed_cus.jsonl`、无认知单元解析；`ids.rs` 也已无本 ADR 的常量残留（原「Agent 能力 id」区的 `CAPABILITY_AGENT_COGNITION` / `_CHAT` / `_IDENTITY` / `_CREATE` 四个悬空常量于 2026-09-17 随 OAB v1 装配实现一并清理，见同日提交）。现存的只有 `CapabilityCategory::Metacognition` 一个分类枚举值。

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

> **当前状态**：**已被取代**（见下方状态行与其「后续 / 终局」三段）——`entities/*` 协议、`EntityCapabilities`、`EntityProvider` trait + 适配器、`StorageService` 已逐层删除。

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
S11 停止路由，能力开关随 S12 删除。资源访问统一经 VDFS（`<根>/<kind>/…`）：
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
`ENTITY_*` 常量与 `kind = "entity"` 事件频道全部删除）。
> 更正（2026-09-20 复核）：本段原先写「只剩 `schemas/entities.rs` 里的
> `DetailDefinition` 表单方言」，**该文件已随 ADR-011 一并删除**；`DetailDefinition`
> 与整套详情方言现住在各插件自己的 `detail.rs`（`plugins/agent/host/detail.rs` /
> `plugins/local/plugin.rs` / `plugins/gateway/plugin.rs` …）。它是 VDFS
> `ext = form` 的宿主方言，与「实体」无关——这一点不变。

---

## ADR-011: 资源存储 = `VdfsProvider` 的集中实现

**状态**：已接受

> **当前状态**：**已实现（现行）**。资源存储 = `providers/vdfs_service/` 的 `SingleFileVdfs` / `DirVdfs` / `MemoryVdfs` 三型，磁盘布局未变。
>
> **批次 G 补充（2026-09-22）**：本条「变更不带载荷」现在**在类型上也成立**——
> `VdfsChange` 只剩 `{ path, change }` 两个字段；`to` / `delta` / `node` / `content`
> 四个可选载荷与 `renamed` / `appended` / `truncated` 三个变更取值**已删除**
> （源自「消息寄生 VDFS」时代，S23–S25 拆完后没有任何生产性生产者）。因此下面「后果」
> 里举的「会话消息的 `appended` + `delta`」这个带载荷增益投递的例子**已不存在**；
> 立的判据是「一个变更取值（或一个载荷字段）必须有生产性生产者」。

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
（`<homedir>/<category>/<id>/<manifest>`）。同时删除
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
- `notify_change` 只报 `(kind, path, change)`、**不携带载荷**（不为此扩展
  core 协议）。因此 `created` / `updated` / `deleted` 一类粗粒度变更，消费者只能
  **防抖重拉**；前端由此退掉第二条 `kind = "entity"` 订阅。
- **三种拓扑要显式选**：挂载点必须声明自己用哪一型、主文件叫什么
  （类别段名 = 插件名 = `PLUGIN_*`，主文件名 = 插件内部 `const MANIFEST`）；
  机制不去目录里猜。选错了表现为「条目内部不该外露却外露」这类可见问题，
  而不是静默错乱。
- 「manifest 补齐 id」这条不变量不再有唯一实现，`model` / `mcp` 各持一份 `with_id`
  （有意为之：它是该资源的写入语义，不是跨插件共享原语）。
- 内存镜像与磁盘清单**同一条广播频道、同一套操作语义**，消费者无需区分条目住在哪儿。

---

## ADR-012: 读侧成本是设计约束（「现在是什么」必须有一张可核对的表）

**状态**：已接受

> **当前状态**：**已实现（现行）**。`docs/CURRENT.md`（`scripts/gen-current-facts.mjs`
> 自动生成 + CI `--check` 门禁）与骨架化占位符的语义摘要/取回指引即本决策的落地。

**背景**：
一个真实消费者（审查本仓库的 agent 会话）复盘了自己的低效，指出的五类摩擦里
有三类直接指向**读侧**：

1. **无法一次建模**——工具历史在请求视图中被骨架化后，摘要只有
   `count=23 first[name=.editorconfig]` 一个条目，模型无法据此判断"目录里有没有
   我要找的东西"，只能逐轮重跑工具（每次核对 1-3 轮往返）。
2. **没有"读侧事实表"**——ADR 全在论证"为什么"，README/ROUTES 这类手抄清单
   会漂移（实测漂移 6 处，见 2026-09-16 第一批提交）；
   "现在是什么"只能从代码反推。
3. **核对不划算就会放弃核对**——验证成本线性上升而收益递减时，模型会自动
   切换成"停止验证、退回文档口径"，并用自信语气包装未验证结论。

**决策**：
把"读侧成本"当作与"写侧成本"（少一层抽象、少一次翻译）对等的设计约束：

- **事实从代码提取，不从人手抄**：`docs/CURRENT.md` 由
  `scripts/gen-current-facts.mjs` 只做提取（插件注册名 / VDFS 挂载点 /
  自有路由臂 / LLM 工具 / 存储布局），提取不到的如实标注"运行期动态"，
  绝不臆造；CI `--check` 门禁保证它与代码不漂移。ADR 的「当前状态」行
  同理——它回答"这条决策现在是死是活"，而不是"它为什么被提出"。
- **骨架化摘要必须支撑一次建模**：列表类 JSON 输出给 `count=N` +
  前若干条条目名（`names=[…,…+余量]`），文本输出给首个有内容的行；
  摘要预算按"token × 2 → 字符"的截断口径推导条目名预算，保证列表收尾
  不被截断（截在半个名字上的摘要等于噪声）。
- **占位符必须自描述取回路径**：骨架化结果附带定位锚点（path/command…）
  与 `Re-run <tool> to get the full output.`——把"要不要赌一把摘要"变成
  一次可预期的工具调用。

**理由**：
- 写侧收益是真实的：`一份机制、零类型知识`（ADR-010）省的是**改代码**。
- 读侧成本是真实的：每多一个读者（人或 agent）冷启动都要重新反推一遍，
  成本随读者数**相乘**；而反推贵到一定程度，读者会开始输出未验证的结论——
  这不是能力问题，是成本结构问题。
- 修复方式不需要推翻任何既有设计：事实表是**提取**而非第二份手抄，
  骨架化只是把"摘要"写得更能用，占位符只是多说一句话。

**后果**：
- `CURRENT.md` 与手抄清单并存：手抄的（README / ROUTES.md）仍是"速览"，
  但必须注明以生成表为准；两处交叉核对本身构成漂移门禁的一部分。
- 骨架化摘要变长（48 → 64 token 上限，列表类多列 **8** 个条目名：
  `SKELETON_DIGEST_TOKEN_CAP = 64` / `JSON_DIGEST_MAX_NAMES = 8`，
  `plugins/session/context_window.rs`）：
  > 更正（2026-09-20 复核）：本条原写「7 个条目名」，与代码不符（代码恒为 8，
  > 两者由同一次提交引入 ⇒ 是 ADR 的笔误，不是后来的改动）。
  每个过期调用多花约 20 token，换来的是少一轮重跑工具的往返——
  这笔交换对"读侧"场景始终是正的。
- 新增插件 / 工具 / 路由后若忘记重跑生成器，CI `--check` 会失败——
  维护者由此承担"同步事实表"的显式义务，而不是把它留给下一个读者。

---

## ADR-013: TLS 后端 = 平台原生栈（`native-tls`），不做纯 Rust 密码学

**状态**：已接受

> **当前状态**：**已实现（现行）**。`symbio/Cargo.toml` 的 `reqwest` 改为
> `default-features = false` + `native-tls`；`aws-lc-sys` / `aws-lc-rs` / `rustls` 族已退出
> 构建图（`cargo tree -i aws-lc-sys` 报 "did not match any packages"）。

**背景**：
本仓的 HTTP 出口（LLM 适配、MCP http transport、web 插件、telegram）统一走 `reqwest`，其
TLS 原由 rustls 提供，而 rustls 的两个官方密码学后端**都是 C**：默认的 `aws-lc-rs` 与备选的
`ring`。这带来两个具体代价：

1. `aws-lc-sys` 是整份 BoringSSL 分支 —— 实测 414 个 `.c` + 178 个 `.cc` + 941 个汇编、
   145,513 行 C、69 MB 源码，且**无条件**依赖 `cmake`（`build-dependencies.cmake`）。
   它是依赖树里最脆的一环：在本机沙箱里其 C 编译必然失败（写 `.obj` 被拦 → `fatal error C1056`），
   连 MSRV 门禁的真编译分支都跑不通。
2. 纯 Rust 的替代 `rustls-rustcrypto` 上游自述 **DO NOT USE IN PRODUCTION**（仅 0.0.x-alpha）
   ⇒ "纯 Rust + 生产可用"这条路今天不存在。

于是可选项只有两个：换 rustls 的 provider（`ring`），或换掉 rustls 本身（OS 原生栈）。

**决策**：
HTTP 出口的 TLS 后端改为**平台原生栈**（`reqwest` 的 `native-tls` feature）：

- Windows → SChannel（`schannel`，纯 Rust FFI 绑定，**零 C 源**）
- macOS → Security.framework（`security-framework`）
- Linux → 系统 OpenSSL（`openssl` / `openssl-sys`）

**明确接受**由此引入的**平台分支**：Linux 需要系统 OpenSSL 开发包。

**理由**：
- 选它而非 `ring`，唯一理由是它能把目标平台上的 C 编译**真正降到零**。`ring` 路线只是把
  69 MB 的 C 换成 8.3 MB 的 C（`ring` 自身 17 个 `.c` + 90 个汇编），
  "依赖树里不要 C"这个目标达不到；而 `native-tls` 在 Windows/macOS 上完全不编 C，
  并连带移除 `cmake` 这个构建期前置工具。
- 它**几乎不新增依赖族**：`native-tls` / `schannel` 早已因 `ort-sys` 的 build-dependency
  `ureq`（ONNX Runtime 下载链）在树里；本次净增只有 `hyper-tls` + `tokio-native-tls` 两个包
  （锁文件 `[[package]]` 386 → 357，唯一包名 357 → 330）。
- 语义上**证书信任本就来自系统库**：原路线的 `rustls-platform-verifier` 做的正是同一件事，
  两条路线在企业根证书 / 系统更新上的行为接近 ⇒ 属于**换实现而非换语义**，
  且 `symbio/src` 未使用任何 rustls 专属的 `reqwest` 配置项（`tls_certs_only` / `crls` /
  `hostname_verification` 全仓零命中），故无功能面损失。
- 代价被判定为可接受：项目的主要目标形态是 Windows 桌面端；Linux 侧只需在构建环境装
  `libssl-dev`，属常规做法。

**后果**：
- **`docs/design/http-api-transport.md` 里「HTTPS 不做」的前提失效**：该决策当初的理由正是
  "rustls 默认后端 aws-lc-rs 依赖 `aws-lc-sys`（C），与无 C 编译冲突"。这条理由已不成立 ⇒
  网关是否开放 HTTPS 成为一个**重新可议**的产品决策（本次不改行为，仍默认回环 HTTP + 外部反代）。
- **剩余 C 编译源只剩 `onig_sys`**（←`onig`←`tokenizers`←`fastembed`）：
  `fastembed` 硬编码 `tokenizers/onig` 关不掉；`esaxx-rs` 虽是 tokenizers 的非可选依赖，
  但被声明为 `default-features = false`（其 `cpp` feature 关闭）⇒ **不编 C++**。
  即"零 C 编译"仍未达成，但已从 4 条链收敛到 1 条。
- **跨平台行为从此随 OS 变**：CI（Ubuntu）与本地（Windows）跑不同 TLS 栈，
  TLS 版本上限 / 密码套件 / 错误文案都不同 ⇒ 一类"CI 绿本地红"的问题排查成本上升；
  这是本决策**明确接受的代价**。
- Linux 构建前置：`libssl-dev` + `pkg-config`（Debian/Ubuntu）或 `openssl-devel`（RHEL/Fedora），
  已记入 [CONTRIBUTING.md](../CONTRIBUTING.md) §1。
- **顺带解掉一个未验证项**：此前 MSRV 门禁的**真编译**分支在本机沙箱跑不通，根因正是
  `aws-lc-sys` 的 C 编译（写 `.obj` 被拦 → `fatal error C1056`），与 MSRV 本身无关。
  `aws-lc-sys` 退树后，`node scripts/gate.mjs --only=msrv` 在本机沙箱**完整跑通**
  （1.91.0 实编译 `symbio` + `cli`，2/2；隔离 target 目录，不污染日常构建缓存）。
- 将来若要 HTTP/3 或后量子（ML-KEM），需回到 rustls 并重新接受 C 依赖——本决策不阻断该回退，
  只是要重付一次代价。

---

## ADR-014: 本地嵌入 = `tract-onnx` 纯 Rust ONNX 推理，废弃 `fastembed`（ORT / onig 彻底退出）

**状态**：⚠️ **已被 [ADR-016](#adr-016-本地嵌入改用-ortonnx-runtime推翻-adr-014-的性能前提并接受它当初拒绝的代价) 部分推翻**（2026-09-18）

> **推翻范围**：本 ADR 的"零 C/C++ 编译"目标**仍然保持**（ADR-016 同样满足）；
> 但"用 `tract-onnx` 做推理"与"推理性能秒级可接受"两点**已作废**——
> 实测每 token ≈ 22.5 ms、全仓建索引 ≈ 6 小时，`codebase_search` 永远撞 600 s 硬超时。
> 下文保留原样作为决策记录，**不要据此认为当前实现是 tract**。

> **当前状态**：`symbio/Cargo.toml` 直接依赖 `tract-onnx`（0.23.7，MSRV 1.91 与本仓持平）+
> `tokenizers`（`default-features = false` + `fancy-regex` 纯 Rust 正则后端）。
> `cargo tree -i fastembed / ort-sys / onig` → 全部 "did not match any packages"。
> 实现：`symbio/src/providers/embedding/local.rs`；注册 id 由 `EMBEDDING_FASTEMBED`（"fastembed"）
> 改为 `EMBEDDING_LOCAL`（"local"），消费方仅 `codebase_search.rs` 一处。

**背景**：
ADR-013 收敛后，依赖树里**最后一条 C 编译链**是 `onig_sys`（←`onig`←`tokenizers`←`fastembed`
硬编码 `tokenizers/onig`，关不掉）。同时 `fastembed` → `ort`（ONNX Runtime）在 Windows x64
强制 `directml` flavour：预编译产物缓存 341 MB `.lib` + 18 MB DLL（合计 ~391 MB），
且 `ort-sys` 的 build-dep `ureq` 下载链拖着一串无谓包。体积与"零 C 编译"两个目标都指向同一结论：
嵌入推理不能经由 ONNX Runtime 预编译二进制。

**决策**：
- 拆掉 `fastembed`，**直接依赖 `tract-onnx`**（不用伞包 `tract`——它会带 17 个 tract-* crate，
  直连 `tract-onnx` 只需 11 个，少 40+ 包）。
- 分词保留 `tokenizers`，但关掉 default（onig），启用 `fancy-regex` 纯 Rust 正则后端；
  只需要 `tokenizer.json` 一个文件（`config.json` / `special_tokens_map.json` /
  `tokenizer_config.json` 是 fastembed 的 `TokenizerFiles` 专用，随迁删除）。
- `local.rs` 行为与 fastembed 逐项对齐：`encode(text, true)`、CLS 池化、**L2 归一化**
  （spike 实测 fastembed 输出范数恒为 1.0 ⇒ 它做了这一步，修正此前"不归一化"的误判）。
- 模型/分词器仍 `include_bytes!` 内嵌（int8 量化 `model.onnx`，24 MB）。
- 单例 `LazyLock` 懒加载，`into_typed()?.into_optimized()?.into_runnable()` 一次编译；
  符号化 seq 维（S）⇒ 任意长度输入复用同一计划；`plan` 即 `Arc<SimplePlan>`，`&self` 并发
  安全无需 Mutex；推理放 `spawn_blocking`。

**理由**：
- `tract` 是纯 Rust ONNX 推理器（无 C/C++、无预编译二进制、无下载步骤），
  是"更小、更成熟、原生"三个维度的唯一交集；`ort` 不行（directml 391 MB）。
- 数值等价有实证：迁移 spike（umbrella `tract` 时代）对同一份模型+分词器，
  tract 管线 vs fastembed 管线 **余弦相似度 0.999558**（int8 模型内计算序差异所致，
  语义等价）；余弦相似度对尺度不变，L2 归一化保证向量表示与 fastembed 一致。
- 瘦身是显式要求：伞包 `tract` 在 lock 上 +102 包（459 blocks），直连 `tract-onnx` 417 blocks。

**后果**：
- **Windows / macOS 构建的 C/C++ 编译真正归零**（Linux 仅剩 TLS 的系统 OpenSSL，
  见 ADR-013 的平台分支）；`onig_sys` 退出 ⇒ 最后一条 C 链消失。
- **~391 MB 的 ORT 预编译开销消失**；lock `[[package]]` 357 → 398（净增 41）：
  退出 fastembed / ort / ort-sys / onig / onig_sys / ureq 下载链 / winapi 族等 19 包，
  进入 tract 11 件套及其纯 Rust 依赖（prost、rustfft、nom 等）。
- **二进制体积实测（2026-09-17，release + rust-lld，同机 A/B）**：以 worktree 隔离重建
  2f9c04a（fastembed + ort 静态链接）作基线，`symbio-cli.exe` 59.8 MiB（62,718,464 B）；
  HEAD（tract）61.2 MiB（64,150,528 B）——**exe 反而 +1.4 MiB（+2.3%）**。原因：326 MiB 的
  `onnxruntime.lib` 大部分并未被链接器拉入 exe，静态链接进来的 ORT 实际对象小于 tract 的
  Rust 代码量。**结论要诚实：exe 体积基本持平、略增**；tract 的收益不在 exe 体积，而在
  构建链与交付面——① Windows/macOS 构建零 C/C++ 编译；② 326 MiB 的 `ort.pyke.io`
  预编译缓存不再参与构建（可直接删除）；③ 不再需要随程序交付 `DirectML.dll`（18 MB）；
  ④ 离线/全新环境构建不再依赖 ORT 下载链（实测：离线下 ort-sys 解析不到预编译 flavour
  会静默退化为「不链接」，链接期才以 `undefined symbol: OrtGetApiBase` 报错，很隐蔽）。
- **family 11 crate 已是下限，无进一步可裁空间**：`tract-onnx` 0.23.7 对 `tract-extra` /
  `tract-hir` / `tract-nnef` / `tract-onnx-opl` / `tract-transformers` 均为**硬依赖**
  （manifest 无 `optional`，features 仅 `getrandom-js` 一项）；`tract-pulse` 链经
  `tract-onnx-opl → tract-extra` 间接硬拉。伞包 `tract` 多出的 6 个 crate
  （tensorflow 域等）才是真冗余，已通过直连 `tract-onnx` 避开。
- 推理性能与 ORT 有差距（tract 无 GPU/图级极致优化），但嵌入场景是离线索引 + 单条查询，
  秒级可接受；初始化编译为秒级一次性开销。
- 注册 id 变更 `"fastembed"` → `"local"`：3 处触点（`ids.rs` / 注册 / `codebase_search.rs`），
  配置就是 `PLUGIN.yml`、无外部引用，无迁移成本。
- MSRV 仍 1.91：`node scripts/gate.mjs --only=msrv` 2/2 通过（tract 0.23.7 `rust_version = 1.91`）。

### 修订（2026-09-18）： tract 加载该模型的两条硬约束

> ⚠️ **本节已作废（2026-09-20 复核标注）**：`tract` 已随
> [ADR-016](#adr-016-本地嵌入改用-ortonnx-runtime推翻-adr-014-的性能前提并接受它当初拒绝的代价)
> 彻底退出依赖树，因此本节描述的两条"硬约束"（`with_ignore_value_info(true)`、
> 不得自建 `SymbolScope`）与那张余弦对照表**都不再是现行约束**，是 `tract-onnx
> 0.23.7` 的历史行为记录。保留它是为了决策可追溯，**不要据此判断当前实现**。

上线后发现 `LocalEmbeddingService` 初始化失败并静默回退 Noop（`Failed analyse for node #203
"/Unsqueeze" AddDims`），语义搜索被禁用。排查结论：**模型本身完好**（`onnx.load()` 通过，
opset 11 / 527 节点），问题全在 tract 侧的形状推断配置。两条约束由此确立并写进 `build_plan`
的注释：

1. **必须 `.with_ignore_value_info(true)`**。该模型是 ONNX Runtime 动态量化导出，图里带
   `value_info`，把中间张量声明成 `batch_size` / `sequence_length` 符号。保留这些声明时，
   tract 会拿输入 fact 的 `1` 与 `value_info` 的 `batch_size` 做 unify，直接报
   `Impossible to unify Sym(batch_size) with Val(1)`。
   ——这也是「固定 seq」兜底无效的原因：报错与 seq 是不是符号无关。
2. **动态长度不能自建 `SymbolScope` + `set_input_fact`**，那样 tract 0.23.7 会在
   `ProofCacheSession` 里触发 `scope_id mismatch` 断言（是 panic 不是 Err）。正确做法是
   **不覆盖输入 fact**，直接沿用模型自己声明的符号维。

顺带修掉一个会让服务静默失效的坑：**`outlet_label` 对图输入返回空串**，取输入名必须走
`model.node(outlet.node).name`，否则三个输入全部落进「未知输入名」分支而返回 `None`。

数值复核（同一句「你好，世界」，7 个 token，CLS + L2）：

| 路径 | vs ONNX Runtime 余弦相似度 |
|---|---|
| 动态长度（首选） | **0.999961** |
| 固定 seq=512 兜底 | 0.994482 |

兜底路径精度下降的原因：补位改变了 `DynamicQuantizeLinear` 的 per-tensor scale，量化误差
随之变化。故固定长度只作兜底，不作默认。

---

## ADR-015: 前端显示由**节点状态**驱动，不由事件顺序驱动

**状态**：已接受

> **当前状态**：**已实现（现行，含 S22 补完；S25 调整了传输载体）**。会话域的实时面
> 收在 `session/stream` **一条**转写流上（`PluginPayload::Session`，不是 `event_bus`）：
> 消息帧与运行态帧**共用同一个 `seq` 空间** ⇒ 顺序由结构保证。`kind = "vdfs"` 频道
> 仍在，但只承载会话**资源**变更（创建 / 删除 / 改名 / 标题 / metadata），收敛方式为
> 幂等重拉。`services/sessionBusWatcher.ts`、`eventBus` 的防乱序缓冲、以及「按地址分派」
> 的实现（`schemas/vdfs.ts::sessionRouteOf`）均已删除——**本 ADR 的结论（显示由节点
> 状态驱动）不变，改的只是节点状态怎么送到前端**。进程内消费者（子智能体转播、CLI）
> 同样改订阅这条流，因此 `kind = "session"` 频道已连同 `KIND_SESSION` 常量一并废除
> （`event_bus` 只剩 `system` / `vdfs` 两个频道，退化为纯传输层）。详见
> [`symbio/src/plugins/session/docs/node-state-streaming.md`](../symbio/src/plugins/session/docs/node-state-streaming.md) §5.1 与 §11。

**背景**：
会话的实时显示原先基于**事件流**：`kind = "session"` 上发 `Status{busy|idle}` / `Abort` /
`Error` / `Connected`，前端 `switch (event.type)` 逐类处理，并靠 `last_failed` 布尔、
`sessionErrors` 平行状态等补丁拼出完整状态。

代价是**正确性依赖到达顺序**，而顺序不是免费保证的。最直白的证据就是
`eventBus.ts` 里那段 `replayBuffer`（切会话防乱序缓冲）——它的注释写着：

> 「旧实现：先订阅，再异步拉 snapshot。副作用：实时事件先到 handler，snapshot 中的
> "更早的事件"反而晚到，造成 Status / Abort 顺序错乱。」

**一段只为修顺序而存在的机制，说明模型本身选错了**：把「现在是什么」表达成
「刚刚发生了什么」的增量序列。

**决策**：
**把运行态从事件里拿出来，变成节点的属性**，前端按地址消费节点状态。

- 会话节点 `<根>/session/<sid>` 承载 `status`（`working` / `active` / `failed`）+
  `attributes.outcome`（`completed` / `aborted` / `failed`）+ `attributes.error`；
- 消息节点 `<根>/session/<sid>/message/<mid>` 承载 `status`
  （`pending` / `streaming` / `waiting_user_action` / `completed` / `failed`）；
- 状态类变更（`updated`）**必带全量节点视图**，消费端**零回读**；
- 前端只有 `sessionRouteOf(地址)` 分派，**没有 `switch (event.type)`**；
- 等待审批、活动文字、可重试性等都由节点表**派生**（纯函数）。

**理由**：

- **状态幂等、可交换、丢一次不影响正确性**；事件是增量、有顺序、丢了没有第二次机会。
  一次变更携带全量视图后，「谁先到」只影响收敛**速度**，不影响**正确性**。
- **补丁随之消失**：`replayBuffer` / `fetchPendingSnapshot` / `sessionBusWatcher`
  存在的理由都是"顺序敏感"，前提没了，补丁也就不需要了。
- **判据只剩一处**：`failed` 单独成态后，「上一轮失败了吗」= `status == 'failed'`，
  不再需要同时读 `status` 与 `last_failed` 两个字段（漏读一处即静默错）。
- **修掉一处有损映射**：`completed` 与「未标注」曾被后端都映射成 `active`，消费端必须
  把 `active` **猜回** `completed`。现在状态原样透传。

**后果**：

- **新增一条不变量**：任何送达前端的会话状态变化都必须经 `emit_session_state`
  （运行态的唯一出口），且 `updated` 必带 `node`。漏发即 UI 永久停在旧状态且无人纠正
  ——两条链路互不校验。消费者侧**不做静默回读兜底**（那会把它掩盖成"看起来能用"），
  只记 warn，靠下一次 `list` 快照收敛。
- **三条残留假设写进文档**（不假装没有）：总线是单条有序通道；`list` 快照只能把
  `active` 升级为 `working`、不得降级；`appended` 依赖路径级串行。前两条是既有的，
  第三条是增量语义的固有属性。
  > **后记（2026-09-23，ADR-025 之后）：三条的现状——** 第三条（增量载荷依赖路径级
  > 串行）**成立**：`delta` 回到变更载荷上，依赖的正是**路径级串行**（同一文件的内容
  > 只由单写入者追加）。第二条（`list` 快照只升不降）**不再需要**：快照一律靠**回读**
  > （幂等、永远最新），不再有「快照 vs 流」的竞争。第一条（单条有序通道）仍是事实
  > 描述，但**不再是任何推理的前提**——顺序是节点属性。
- **`kind = "session"` 已整体删除**（S22 补完）：进程内消费者——`agent/host/subagent.rs`
  的审批透传 / 文本累积与 `cli/src/client.rs` 的渲染 / 完成判定——都改成「订阅总线 +
  `vdfs/watch` 登记」后消费 VDFS 变更。完成判据也随之从「等 `Status idle` 帧」改为
  「读会话节点的 `status`」——同一个判据在前端、subagent、CLI 三处首次真正同源。
  > **后记（2026-09-23）：** 消息的实时面中途曾改走 `session/stream` 转写流，并在那里
  > 引出过一个折算层 `symbio_core::schemas::session::transcript`（全量视图与增量补丁
  > 折成一种形态）。**该形态已按 ADR-025 整体撤销**——转写流与折算层均已删除，消息与
  > 运行态同走 `kind = "vdfs"`，按载荷形状（`delta` 追加 / `content` 替换 / 无载荷回读）
  > 落地，**不需要折算**。上面两条结论（`kind = "session"` 的处置、「读会话节点
  > `status` 判本轮结束」）都不变。
- **词汇不合并**：`streaming`（消息）与 `working`（会话）保持两个词。合并会连带改
  `status-*` CSS 类名与 `isWorkingStatus()`，而**漏改 CSS 类名不报错、不失败，只会让
  流式动画静默消失**——正是"体验不得变差"要防的那类回归。

## ADR-016: 本地嵌入改用 `ort`（ONNX Runtime）——推翻 ADR-014 的性能前提，并接受它当初拒绝的代价

**状态**：已接受（已实现）

> **当前状态**：`symbio/Cargo.toml` 依赖 `ort = "2.0.0-rc.13"`（默认 features）+ `tokenizers`
> （`default-features = false` + `fancy-regex`）。`tract-onnx` 已移除；
> `cargo tree -i cc` / `-i ring` / `-i onig_sys` 三者皆空（**零 C/C++ 编译仍成立**）。
> 实现：`symbio/src/providers/embedding/local.rs`。

### 背景：ADR-014 的性能前提被实测证伪

ADR-014 选 `tract-onnx`（纯 Rust）而弃 ORT，理由有三：① 零 C/C++ 编译；② 不引入
~391 MB 的 ORT 预编译产物；③ 不必随程序交付 `DirectML.dll`（18 MB）。它对性能的判断是：

> 推理性能与 ORT 有差距……但嵌入场景是离线索引 + 单条查询，秒级可接受。

**这句话对"单条查询"成立，对"建索引"完全不成立。** 2026-09-18 实测（本机，release）：

| 输入 | token | tract | ort | 倍数 |
|---|---|---|---|---|
| 短查询 | 13 | 386 ms | 2.4 ms | 161× |
| 一个 40 行块（≈1500 字符） | ~600 → 截 512 | 9.83 s | 19 ms | 517× |
| 超长（截到上限） | 512 | 9.80 s | 13 ms | 754× |

即 **每 token ≈ 22.5 ms、纯线性**；release 只比 debug 快 16% ⇒ 是**结构性**慢
（tract 没有 ORT 级的算子融合与多线程 GEMM），不是"没调好"。

后果：`codebase_search` 要给整个工作区分块嵌入。本仓 500 个文件 / 6,681 块 ⇒
**全量建索引 ≈ 6 小时**（tract）vs **59 秒**（ort）。而工具执行有
`TOOL_EXEC_HARD_TIMEOUT_SECS = 600` 的硬超时——**语义检索在 tract 下永远拿不到结果**，
用户看到的是"一直回复中，永远没有响应"。ADR-014 的"秒级可接受"必须撤回。

### 决策

1. **推理后端换 `ort`**（ONNX Runtime 的 Rust 绑定），**其余一律不动**：模型仍是内嵌的
   int8 `bge-small-zh-v1.5`，分词仍用 `tokenizers` + `fancy-regex`，后处理仍是
   `encode(text, true)` + CLS 池化 + L2 归一化。
2. **不恢复 `fastembed`**。它的 C 编译链主犯是硬编码的 `tokenizers/onig`（⇒ `onig_sys`），
   不是 ORT；它唯一不可替代的部分只是"调 ONNX Runtime"。直接依赖 `ort` 就能拿到同样的
   推理速度，而不必把 `onig_sys` 请回来。
3. **接受 ADR-014 当初拒绝的三项代价**（逐条核实见下），因为"不可用"比"贵"更糟。

### 代价（逐条核实，不粉饰）

- **C/C++ 编译：仍然为零。** `ort-sys` 的 ORT 是 build 期下载的**预编译二进制**；
  `cargo tree -i cc / -i ring / -i onig_sys` 三者皆空。ADR-014 的这个目标保住了。
- **341 MB + 18 MB 的预编译产物：回来了。** Windows x64 拿到的是 **DirectML flavour**——
  `ort-sys` 的 `BinariesSource::Pyke` 分支源码注释写明 *"pyke libs always ship compiled with
  DirectML on Windows"*，且 `directml` 虽是可关 feature，**pyke 预编译本身已含 DML EP，
  用 features 关不掉**。缓存落在 `%LOCALAPPDATA%\ort.pyke.io`：
  `onnxruntime.lib` 341,152,186 B + `DirectML.dll` 18,527,776 B（与 ADR-014 引用的数字一致）。
- **`DirectML.dll` 是静态导入，不是延迟加载。** 解析构建产物的 PE 导入表确认
  （`directml.dll` / `d3d12.dll` / `dxgi.dll` 都在静态导入列表里，延迟导入为空）。
  好消息：**Win10 1903+ / Win11 由系统提供该 DLL**（本机 `C:\Windows\System32\DirectML.dll`
  3.0 MB），实测把随包那份删掉后 `cargo test --release --lib embedding` 仍 4/4 通过。
  故目标平台（Win11 23H2）不必额外交付 18 MB；**跨到更老的 Windows 则需要随包**。
- **离线构建会静默退化。** 拉不到预编译 flavour 时 `ort-sys` 不报错，只是"不链接"，
  直到链接期才以 `undefined symbol: OrtGetApiBase` 失败（ADR-014 已记录过这个坑）。
  CI 需要能联网，或经 `ORT_LIB_LOCATION` 指向本地副本。
- **exe 体积：反而变小了。** `symbio-cli.exe` release 实测 **62,310,912 B = 59.42 MiB**，
  比 ADR-014 记的 tract 61.18 MiB **小 1.76 MiB**，也比 fastembed+ort(1.x) 的 59.81 MiB
  略小。原因同 ADR-014 的分析：341 MB 的 `onnxruntime.lib` 是静态库归档，链接器只拉
  被引用到的对象，实际进 exe 的部分小于 tract 的 Rust 代码量。
  （口径说明：这三次测量不是同一次提交的 A/B——本轮还改了 `codebase_search`。
  但量级与方向可信：**换 ort 没有让交付物体积变差**。）

### 数值一致性（换引擎不该换语义）

一次性对照探针（`.workbuddy-ai/align-probe/`，跑完即删）让 tract 与 ort 跑同一批文本：

| 样本 | token | tract | ort | 余弦 |
|---|---|---|---|---|
| query_zh | 13 | 386 ms | 2.4 ms | 1.000000 |
| query_en | 20 | 470 ms | 2.4 ms | 0.999852 |
| code_rs | 91 | 1,889 ms | 4.0 ms | **0.996661** |
| mixed | 39 | 788 ms | 2.2 ms | 1.000000 |
| tiny | 3 | 156 ms | 1.1 ms | 1.000000 |
| long_512 | 512 | 9,832 ms | 18.8 ms | 0.998969 |

- **最低余弦 0.996661**，比 ADR-014 记录的 fastembed↔tract（0.999961）大一个量级。
- **这是偏差不是噪声**：两个引擎各自的**自一致性 ≥0.999999**（同一输入重跑逐位可复现），
  所以差异来自 int8 核的累加顺序与中间精度，是可复现的**引擎差异**。
- 结论按传递性给出：|ort − fastembed| ≤ |ort − tract| + |tract − fastembed|，故 ort 与
  fastembed 的余弦**不低于约 0.9966**。而 `ort` 本来就是 fastembed 用的那个引擎，
  这个量级是 int8 量化噪声，不是质量退步。
- **未做的验证（如实说明）**：**没有**量"换引擎是否改变 top-k 检索排序"。判断性能差距
  已足够悬殊、不必再跑，该对照被中止。若后续要补：同一语料两个引擎各排一次序，
  比 top-10 重合度与平均名次位移。

### 顺带确立的两条索引侧决策

- **索引落盘 + 按 `mtime` 增量重建**（`{workdir}/.symbio/cache/codebase-index.bin`）。
  这是"索引静默过期"的修复：此前 `INDEX_CACHE` 是纯进程内、**零失效机制**
  （全文件搜 `mtime` / `modified` / `invalidate` / `stale` 命中 0 次），agent 会拿一份
  不含自己刚写的代码的索引去搜，且完全无从察觉。现在每次调用都扫一遍文件指纹，
  未变的文件零嵌入。实测：冷启动（读盘 + 增量）**9.9 ms**，改 1 个文件 **434 ms**。
  已知边界：指纹 = `mtime` + 字节数，**刻意保留 `mtime` 的写入（如 `rsync -t`）检测不到**，
  这是 `mtime` 型增量的固有取舍，`rebuild=true` 是兜底。
- **生成物不进索引**：`tokenizer.json`（21,277 行）与 `package-lock.json`（7,249 行）
  两个文件就占掉全库 8,104 块里的 1,426 块（≈18%），且会**挤占 top-k**（词表里全是短
  token，对任何查询都有中等相似度）。按名字排除锁文件/压缩产物 + 按 5,000 行上限排除
  生成的数据文件 ⇒ 块数 8,104 → 6,681，冷建 69 s → 59 s，索引 29.5 MB → 25.2 MB。
- **未做的一件计划内改动**：原先打算顺手去掉 `CHUNK_STEP(20)` / `CHUNK_LINES(40)` 的
  50% 重叠（工作量减半）。**没有做**——当初提这个是为了把 6 小时砍到 3 小时，而那个
  前提在 ort 下已消失；重叠对跨块边界的召回有实际价值，不该为已不存在的性能问题让路。
  如需再压冷建时间，正确顺序是：先做批推理（同长度分桶，避免 ADR-014 记录过的
  「补位改变 `DynamicQuantizeLinear` scale」精度损失），而不是先砍重叠。

### 后果

- `codebase_search` 从"永远超时"变成可用：首次建索引 59 s（一次性、落盘），
  之后每次调用 ~10 ms 内确认新鲜度、仅重嵌改动过的文件。
- `symbio` 的 lock 净减：退出 tract 11 件套及其纯 Rust 依赖。
- 构建环境新增两个外部依赖：**联网**（或 `ORT_LIB_LOCATION`）与
  `%LOCALAPPDATA%\ort.pyke.io` 的 ~359 MB 缓存。
- **ADR-014 的状态改为「已被 ADR-016 部分推翻」**：其"零 C/C++ 编译"的目标仍然保持，
  "用 tract 做推理"与"性能秒级可接受"两点作废。

---

## ADR-017: `session` 的规模债**暂不拆分**（触发条件驱动的延后，不预设未来模块）

**状态**：已接受（**带触发条件的延后**）

**背景**：
`session` 目前 **14517 行实现代码，占 `symbio/src` 生产代码的 25.3%**
（2026-09-20 复测：生产总计 57426 行。⚠️ 本段原写「14340 行 / 29.3%」，是 2026-09 的
度量——**session 本身只涨了 1.2%，占比下降是因为其余代码涨得更快**，故"规模债"的
相对严重程度其实是**减轻**了的）。
它是 16 个插件里唯一"一个插件 ≈ 一整个应用"的
存在——会话编排、`chat_loop` 状态机、消息存储、工具执行与审批、VDFS 适配、心跳等
都在其中。它是与**插件边界划分**最相关的结构债（见 ADR-003「Session 作为编排入口」）。

**决策**：
**现在不拆。** `session` 的边界是 ADR-003 确立的"编排唯一入口"；智能体域的归属**已在
`agent` 插件内稳定**（ADR-005 当前状态：agent 插件仍是 agent 目录宿主；`agent-dir/v2` 使
Agent 本身就是一棵插件树，技能/MCP 复用宿主既有插件目录、人格为根 `AGENTS.md`——
不存在需要被"替代"的独立旧 `agent` 模块）。因此**不再预设某个未来模块来触发拆分**：
`session` 的重组只在下方触发条件命中时才做，且按当时的实际边界一次性完成，而不是先
按现状切几刀。

**理由**：
- **边界已清楚，但先切仍需重切**：`session` 的边界是 ADR-003 确立的"编排唯一入口"；
  智能体域归 `agent` 插件（ADR-005 / `agent-dir/v2`），两 domain 的归属都已确定。
  但 `session` 内部职责是否要下沉到子模块，取决于下方触发条件是否命中——若未命中，
  现在切只是把"一个插件 ≈ 一整个应用"变成"几个子模块 ≈ 一整个应用"，付拆分成本却无收益。
- **规模本身不是本轮发现的问题的成因**：这一轮修掉的缺陷——路径穿越、shell 白名单旁路、
  会话丢更新、网关无上限、死代码——**没有一个**源于 `session` 体量大，全部已就地修复。
  规模是"可维护性利息"，不是当下的故障源。
- **`session` 的测试是全仓最厚的**（7021 行，2026-09-20 复测；且**全部**在独立
  `*.test.rs` 里，内联为 0——实测 `plugins/session` 下 `mod tests {` 命中 0，
  这条仍然成立）。
  此刻大动会同时搬代码与搬测试，把"重构"和"回归"混在一次提交里，反而降低可验证性。

**触发条件（命中任一即应重启此决策，由新 ADR 取代本 ADR）**：
1. 出现"改 A 必须同时改 B"的跨文件连锁，而其根因是 `session` 内部职责混杂；
2. 单次改动的 diff 因 `session` 体量而无法被人完整审阅；
3. 需要把 `session` 的某部分职责（如工具执行与审批）下放到独立模块、且能明确其归属与作用域时。

**后果**：
- 本阶段不引入新的内部模块边界，`session` 继续作为单一插件存在；
- 在此期间对 `session` 的改动坚持"**就地最小化**"，不做顺手的大搬迁；
- 本 ADR 被触发条件命中时，应被**新 ADR 取代**，而不是直接删除——延后是有理由的，
  理由失效这件事本身也值得记录。

---

## ADR-018: 压缩失败**不得裁剪历史**——失败必须是可见、可重试、可持久化的状态

**状态**：已接受

> **当前状态**：**已实现（现行）**。`CompressionFailure::InputOverLimit` 取代了
> `emergency_tail_compression`；`ResumeAction::RetryCompaction` 提供重试入口。

**背景**：

上下文压缩（L2 语义压缩）失败时，历史上有一条"本地机械兜底"路径：
若预判 LLM 摘要请求必然超限（请求体就携带完整待压缩历史，与上下文**同源超限**），
就按 token 预算机械砍掉早期消息、插入一条截断说明头，然后 `return Ok(Some(..))`
——**回报成功**。

这条路径的触发时机恰恰是"压缩反复失败的终点"：
压缩 LLM 请求失败 → 回滚（历史完整）→ 下一轮再失败 → 熔断（跳过自动压缩）→
上下文继续增长 → 越过模型输入上限 → **本地截断**。
即：**压缩失败的最终代价由历史买单**，而且用户看到的是「已压缩上下文（N → M 条）」
的成功态节点——没有原因、没有重试入口、历史凭空变短、后续内容与截断前失联。

**决策**：

1. **压缩失败一律不改动历史**。四个出口（LLM 失败 / 快照校验失败 / 输入超限 / 用户中止）
   全部还原为调用前的消息列表，一条不丢。
2. **失败是可见的**：压缩节点落 `Failed` + `meta.failure_kind` + 可读原因
   （`llm_error` / `invalid_snapshot` / `input_over_limit`），落库持久化。
3. **失败是可重试的**：新增 `ResumeAction::RetryCompaction`，失败节点上给「重试」入口，
   删除失败节点后重新执行一次压缩。
4. **删除机械裁剪**：`emergency_tail_compression` 与其 4 条单测一并移除。
   预判仍然保留——但只保留"跳过注定失败的巨型请求"这一收益，不再顺势裁历史。

**理由**：

- **历史是用户的资产，不是压缩机制的资源**。"这次压缩放不下"只说明压缩需要更强的
  模型或更小的输入，不构成"可以静默丢历史"的授权。截断是**破坏性且不可逆**的
  （虽然存档可回溯，但用户不知道要回溯），而失败是可逆的。
- **静默降级会把故障伪装成成功**。回报 `Ok` 让压缩节点显示"已压缩上下文"，于是
  用户既不知道出了事，也无从判断该做什么。这与 ADR-015（显示由节点状态驱动）的
  精神一致：状态必须如实，不能用一个好看的状态掩盖一次失败。
- **可诊断性已经具备，缺的只是"不兜底"**。失败原因（`failure_kind` + `error`）
  与熔断机制此前都已实现，说明"失败可以被如实表达"——那么机械兜底就是**多余的
  第二套语义**，只会与它冲突。
- **重试必须有出路**。`input_over_limit` 当场重试必然再失败，看似该"聪明地"不给
  入口；但那会掐掉唯一的出路——用户换一个上下文更大的模型后，同一次压缩就能成功。
  因此**不按 `failure_kind` 分档**：给入口，把判断留给用户（原因里含
  `待压缩量 vs 上限` 两侧数字，足够他判断要换多大的模型）。

**后果**：

- **会话可能停在错误上**：当历史真的超出模型输入上限且压缩始终失败时，本轮请求会
  撞 provider 的 context-length 错误而失败（带原因 + 重试入口），而不是"带着残缺
  历史继续跑"。这是**刻意的取舍**——可见的失败优于静默的数据损失。
- 用户侧的出路有三条：换更大上下文的模型后重试、手动精简会话、另起会话。
  三条都是显式动作，不会在用户不知情时发生。
- 压缩节点的失败形态必须与成功形态**视觉可分**（失败走错误条 + 重试，成功走弱化
  样式）——把"压缩没做成"渲染成一行淡淡的灰字，等于把需要决策的状态伪装成无事发生。
- 前端新增第三种重试粒度 `retry_compaction`：压缩节点是**根级**节点，若落到
  "回溯到父 Turn"的兜底分支，后端 `process_retry_turn` 会以 NotFound 拒绝。

---

## ADR-019: 跨栈契约**手工镜像 + 审计守卫**，不引入代码生成；G3（前端面板去语义）否决

**状态**：已接受（含一项否决）

**背景**：

前端 `tauri/src/schemas/` 与后端 `symbio_core` 之间是**手工镜像**：`chat_message.ts`
与 `schemas/session/chat_message.rs` 逐字段对应，`schemas/vdfs.ts` 与
`vdfs_provider.rs` / `plugins/vdfs/protocol.rs` 的常量逐字对应。第二轮前端机制化
复核把"跨栈重复"列为剩余最大的一块，并给出两条路：

- **生成侧**：引入 `ts-rs` / `schemars`，从 Rust 类型导出 TS；
- **检查侧**：扩展 `scripts/protocol-mirror-audit.mjs`，把已存在的镜像纳入守卫。

当时的实测结论是：`protocol-mirror-audit` **只守 3 个常量**，而前端实际持有
**31 个**后端协议词的副本（10 个 op、2 个会话状态、8 个 ext、4 个 action、
5 个 change、1 个 kind、1 个"新资源默认 ext"）。剩下 28 条无人看守——后端重命名
`vdfs/list` 或某个状态词时，前端会静默失效而**所有守卫仍绿**
（`docs/archive/architecture-health-check-2026-09.md` F-5 已记录）。

同一份复核还提出 **G3**：`components/appearance/Appearance.vue`（250 行）与
`About.vue`（75 行）由前端自持，被判为"违反不变量 4（前端零资源知识）"。

**决策**：

1. **不引入代码生成**。`schemas/` 继续手工镜像 Rust 契约。
2. **跨栈一致性交给审计，且审计要"自动发现"而不是"手工登记"**：
   - **A 组**：前端 `VDFS_*` 字面量常量 ↔ 后端同名常量，逐字比对。扫后端两个常量源
     与前端 `schemas/vdfs.ts`，**取同名交集**——新增常量即自动进入守卫，不必改脚本。
     名字不同的镜像登记在 `ALIASES`；前端自持（后端无对应）的常量必须登记在
     `LOCAL_ONLY` 并写明理由——**"没登记"会报错**，所以不存在静默的漏网。
   - **C 组**：后端**闭集的取值集合** ↔ 前端词表数组，要求集合相等（不比顺序）。
     后端表达闭集有**两种**写法，两种都收：
     - 带 `#[serde(rename_all = "…")]` 的**枚举** —— **转换规则按后端声明的取值
       自动分派**（当前支持 `snake_case` / `lowercase`，遇到别的取值**报错**而不是
       猜一个）；
     - 一组 `pub const PREFIX_*: &str`（**常量组**）—— 字面即线上取值，按前缀提取。

     共 **7 张**（2026-09-20）：`CHAT_ROLES` / `MESSAGE_TYPES` /
     `MESSAGE_STATUSES` / `RESUME_ACTIONS` / `OPTION_TYPES` /
     `SESSION_RISK_LEVELS` / `OPTION_PICKS`。
     > **后续修订（2026-09-23）**：`OPTION_TYPES` / `OPTION_PICKS` 随会话选项机制
     > 整体下线而撤除，同时 S1 新增 `DETAIL_PICKS`（详情方言的机制原生取值原语，
     > 即原来的 `OPTION_PICK_*` 换了家）。当前 **6 张**——数字以
     > `protocol-mirror-audit.mjs` 的 `ENUM_SETS` 长度为准，这里不追着改。
     ⚠️ 只认第一种写法会让第二种长期无人看守：`OPTION_PICK_*` 当初被 Rust 侧的
     `#[allow(dead_code)]` 登记成「消费方在前端」，而前端那份是**独立硬编码**的
     第二份抄本（无引用关系）——没有任何守卫比对两边。
     （该教训的产物 `DETAIL_PICKS` 登记至今仍在服役；`OPTION_*` 那套已删。）
     ⚠️ 初版把 `snake_case` **硬编码**成了检查项，于是
     `rename_all = "lowercase"` 的 `RiskLevel` 虽在前端有镜像却长期无人看守——
     「枚举类型对了、属性取值没覆盖到」是**守卫自己的漏**，不是登记的漏。
   - **D 组**：后端**结构体** ↔ 前端**接口**的**字段名**。只查一个方向——**前端
     持有的字段必须能在后端线格式里找到**（或登记在 `tsLocal` 作为"前端自持"）；
     反方向不查，前端不必镜像后端全部字段。只比字段名不比类型（类型映射正则读不出来，
     而"改字段名"本就是后端最常见的契约变更）。
   - **E 组**：后端文件顶部的 `// Corresponding Frontend: <路径>` 必须指向
     **真实存在**的文件。这条头是**人**在两边之间跳转的入口，腐烂得很安静
     （前端改名后没人被告知）。实测 8 条里 **7 条悬空**，其中 6 条指向从未在版本史里
     出现过的 `tauri/src/protocols/`（2026-09-20 已删）。故约定收紧为：
     **写了就必须指向真实文件；前端没有镜像就别写**——假指针比没有更糟。
3. **G3 否决**。`Appearance.vue` / `About.vue` 不违反不变量 4。

**理由**：

- **格式层已经单源，重复只在符号层**。两侧共用同一套契约名（`snake_case` 线格式），
  差别只是"Rust 标识符 vs TS 常量名"——这不是两份真相，是同一条真相的两种拼写。
  `serde` 就是那个单源点：后端把契约词写一次，序列化与投影都从它派生
  （`MessageStatus::as_str()` 与其序列化名的一致性由 Rust 单测
  `message_status_word_matches_serde` 逐变体锁死）。生成器要解决的问题，格式层
  已经解决了。
- **AST 无关，机器导出不可靠**。`chat_message.rs` 里 95 处 serde 标注，其中
  `untagged`（`MessageContent`）、`tag = "type"`（`ContentPart`）、
  `skip_serializing_if`（`ImageUrl.detail` / `ResumeRequest.args`）都会让导出的
  JSON Schema 与实际线格式不符。要生成就得先为这些特例写规则，成本不比手工镜像低；
  而**生成器错一次，错的是全部**，手工镜像错一处只错一处。
- **既有先例是"审计镜像"，不是"生成产物"**。`protocol-mirror-audit` 从第一轮起
  就在做这件事（`X-001..X-003`）。扩展它与既有机制同向；引入生成器则要同时改
  ADR、改 CI（生成步骤接进门禁）、改"契约层是手写的"这一前提——是架构级改动，
  而它换来的只是把最机械的一层自动化。
- **能自动化的与不能自动化的要分清**。能机器比对的只有**闭集取值**（枚举 / 常量）。
  `messageTypes.ts` 的文案与折叠策略、`vdfsCards.ts` 的三条约定、`vdfs-form.ts` 的
  `mergeDetailActions` / `detailPresetPatch`、`vdfs.ts` 的路径代数——这些是**业务
  判断**，生成器也造不出来。所以"引入生成器"最多消掉契约里最机械的那一层，
  却要付架构级代价。
- **G3 的前提与不变量 4 的原文不符**。`docs/design/vdfs-frontend.md:42-43` 写的是
  「前端只持有 `ext → 渲染器`、`子目录名 → 图标` 这类纯 UI 映射；**不得**硬编码
  资源类型清单、标签、路径模板、能力开关」——`ext → 渲染器` 是**明文允许**的。
  而 G3 把 `appearance` / `about` 说成"唯一 ext 即语义类型名的特例"，实测不成立：
  `registry/vdfsRenderers.ts:24-31` 里 `form` / `session` / `message` / `text`
  同样是"ext 即语义类型名"，它们都是**渲染分派键**，正是不变量 4 允许的那一类。

**后果**：

- 后端改一个契约词（op / ext / status / action / change）而前端没跟 ⇒ 审计立刻红，
  报出前后端两侧的文件与常量名。此前 28 个常量处于无人看守状态。
- 后端枚举加一个变体而前端词表没跟 ⇒ C 组红。`aborted` 曾因词表缺项让中止后的
  Turn 显示成已完成、重试入口不出现；这条路现在被堵死。
- **新增契约词的成本是"两侧各加一行"**，而不是"跑一次生成"。这是刻意的：契约改动
  本就该被两侧同时看见，生成器会把"改契约"变成"改契约 + 重新生成 + 复核 diff"。
- **结构体字段形状（struct ↔ interface）已由 D 组补上**——比对的是**字段名集合**，
  不是类型映射（类型读不出来，而"改字段名"本就是后端最常见的契约变更）。D 组上线
  当次就抓出 `ChatMessage` 的两个**死字段**（`agent_id` / `prompt`：后端从不下发、
  前端也零点访问）并删除。**类型声明里的死字段此前无人看守**：TS interface 的字段
  既不是"引用"也不是"定义"，`dead-code-audit` 看不见它们。
- **D 组的覆盖面按"前端逐字段镜像了它"这一判据推进**：首批 9 对（VDFS 契约 +
  `ChatMessage`），随后纳入**详情表单宿主方言**（`detail.rs` ↔ `vdfs-form.ts`，9 对：
  `DetailCondition` / `DetailOption` / `DetailField`(17 字段) / `DetailSection` /
  `DetailPreset` / `DetailPresetSpec` / `DetailBadge` / `DetailAction` /
  `DetailDefinition`），共 **18 对**。这批是"前端把整套形状抄了一遍"的典型——
  `DetailField` 一个字段不落。反过来，前端只挑几个字段
  用的响应结构**不登记**：它本就该按需取，多抄反而不必，塞进来只制造噪音。
  > **后续修订（2026-09-23）**：D 组曾再纳入**级联选项机制** 5 对
  > （`OptionDisplay` / `OptionAction` / `OptionNode` / `OptionsRequest` /
  > `OptionsResponse`，共 23 对）。那套机制随会话选项 schema 化整体下线，
  > 5 对登记与对应夹具一并撤除 ⇒ 回到 **18 对**。判据没变：登记的永远是
  > 「前端逐字段镜像了它」的形状，机制没了，形状也就没了。
- **类型映射仍无守卫**（`Option<u64>` ↔ `number` 这类）。它需要一张类型映射表，且
  泛型 / 嵌套会失控；收益也低于字段名——改类型通常伴随改字段名，那已经能被 D 组挡住。
- **G3 关闭**，不进入实施清单。`Appearance.vue` / `About.vue` 的规模是**呈现层
  体量**，不是"前端持有了不该持有的知识"；要压缩它们应从组件复用入手，与不变量 4
  无关。
- 审计脚本的**回归测试**必须与 `ENUM_SETS` / `STRUCT_SETS` 同步铺满：C 组或 D 组
  新增一条登记，测试夹具就要多铺一对——少铺一张，基线用例就会因"文件 / 枚举 /
  结构体不存在"变红。这是刻意的耦合：它保证夹具与脚本清单不会悄悄脱节。

---

## ADR-020: 执行期与传输层**分离**——`EventSink`（出）+ `AbortSignal`（入）取代 `PluginChannel` 的双职责

**状态**：已接受（第一批「双原语」与第二批「工具侧收敛」均已落地；`shell` / `agent_run`
的流式增量仍待接入）

> **当前状态**：**已实现（现行）**。原语在 `symbio/src/symbio_core/exec.rs`
> （`EventSink` / `AbortSignal` / `TranscriptWriter`），生产实现
> `session/orchestrator/sink.rs::TranscriptSink`；`ControlSignal` 协议面已删除；
> 消费循环改写为 `orchestrator/consume.rs` 的 `spawn + select!` 三臂。
> 机制细节见 [`symbio/src/plugins/session/docs/core-loop.md`](../symbio/src/plugins/session/docs/core-loop.md)
> §6（双原语）与 §7（工具侧收敛）。
> 门禁 32/32、`cargo test --lib` 811 passed、CLI 端到端四场景通过。

**背景**：

一次会话请求的流式链路曾有 **8 跳**，其中两跳是纯开销：

```text
parse_sse_stream → emit_append(serde #1) → PluginFrame → 消费循环
  → from_value::<NodeOp>(serde #2) → Transcript::apply → publish_frame(serde #3) → 前端
```

根因是 `PluginChannel` **一个类型承担了两种语义**：

| 职责 | 具体表现 |
|---|---|
| **出**（事件） | `serde_json::to_value(NodeOp)` → `PluginFrame::Data` → 通道 → 消费循环 `from_value::<NodeOp>` 再解回来 |
| **入**（中止） | Abort 帧 + `abort_flag` 的 100ms 轮询 + `cancel_token`，**三源并存** |

代价不只是两次 serde。由它派生出四类结构性冗余：

1. **同进程调用付进程外代价**。`provider.execute_turn` 与 `tool.execute` 的两端
   **始终同进程同 spawn**（`bound_provider.rs`），却按跨进程协议编解码。
2. **「静默」只能靠所有权手术**。上下文压缩需要「本次调用不产生可见事件」，
   历史做法是造哑 sender + 把 rx 与主通道 `mem::replace` 对调 + 起 drain task
   排空。`resume.rs` 重跑工具同理。**这是把「不说话」表达成了「换一根管子」**。
3. **中止的三种来源各有各的竞态**。投帧可能因对端已退出而丢失；轮询有 100ms
   延迟；`cancel_token` 与 `abort_flag` 可能不一致。`handle_abort` 因此必须
   准备 3s 兜底。
4. **消费循环被迫「按帧醒来」**。它必须持续 `recv` 才能感知中止与状态变化，
   于是「任务结束 / 看门狗 / 被接管」三种结局只能靠嵌套双层 `spawn` + `join` +
   keepalive sender 拼出来。

**决策**：

1. **执行期与传输层分离**，各用语义单一的原语：

   | 原语 | 方向 | 形状 | 用在哪 |
   |---|---|---|---|
   | `EventSink` | 出 | `Direct(Arc<dyn TranscriptWriter>)` \| `Null` | 执行期（LLM 单轮 / 工具调用） |
   | `AbortSignal` | 入 | `Arc<AtomicBool>` + `CancellationToken` 合一 | 执行期 |
   | `PluginChannel` | 双向 | `{tx, rx, cancel_token}` | **退回纯跨进程传输**（前端实时面） |

2. **`abort()` 是唯一置位入口**，置位即唤醒（`CancellationToken`）。中止**不再有帧、
   不再有轮询、不再有第二个来源**。
3. **`EventSink::Null` 是「本次调用不产生可见事件」的类型级表达**，取代哑通道 + drain task。
4. **删除 `ControlSignal` 协议面**（三源合一后无消费方；前端本无镜像）。
5. **消费循环退化为纯生命周期管理**：一次 `spawn` + `select!` 三臂
   （任务结束 / 1800s 看门狗 / 状态被接管），不再收帧、不再解码、不再分派。
6. **`AbortGuard::disarm` 注销登记时一并 `abort()`**，把历史上「通道被 drop ⇒
   对端读到关闭 ⇒ 视作中止」这条隐式语义**显式化**，并由测试锁定
   （`orchestrator.test.rs::disarm_aborts_the_signal`）。

**理由**：

- **进程内调用不该付进程外的代价**。这是判断的支点：既然两端同进程同 spawn，
  直接调 `TranscriptWriter::apply` 就是正确形状，`serde` 只是历史包袱。
- **「不说话」应当是类型选择，不是所有权交换**。`EventSink::Null` 让压缩路径
  「绝不产生可见事件」成为**编译器可检查**的性质，而哑通道方案里它只是一个
  运行期约定——任何人误用 `tx` 都能捅穿。
- **中止的复杂度来自「多源」，不是来自「中止」**。三源合一的收益不只是少两段代码，
  而是**消除了一整类竞态**：不再存在「帧丢了但标志位没置」这种状态组合。
- **`EventSink` 认识 `ChatMessage`，但不认识 `Transcript`**。转写抽象成
  `TranscriptWriter` 后，`symbio_core` 不必知道会话存储的存在，而「转写只有
  一个写入点」这条不变量仍由类型保证。

**后果**：

- **后端两跳 serde 消失**：`parse_sse_stream → sink.apply(message) → TranscriptSink
  → Transcript::apply`。日志里不再有每帧 `from_value` 的往返。
- **`ControlSignal` 从 8 处引用归零**；`PluginChannel` 不再承担执行期协议。
- **三处结构性冗余消失**：压缩的哑通道 hack、`resume` 的临时通道 + drain、
  消费循环的嵌套 spawn + keepalive sender。
- **`AbortGuard` 成为中止登记的唯一管理者**，`ActiveSessionState.ai_control_tx`
  → `abort_signal`。`handle_abort` 的 3s 兜底判据（登记变 `None`）与语义不变。
- **保住的语义**：转写唯一写入点；`Warn` 归会话级状态不进转写；「中止不是失败」
  （结局 `aborted` 而非 `completed`，在途根 Turn 定稿 `Aborted`，重试入口不消失）；
  工具经**自有** `PluginPayload::Session` 通道回传事件的能力完整保留。
- **未收敛的残留（下一批）**：`shell` 与 `agent_run` 的**流式增量**仍把
  `PluginPayload::Session` 当事件流用，由 `tool_executor` 解码后**再转发**进
  `sink`（含 user 角色过滤、`parent_id` 锚定、`user_prompt` 捕获）。它们的自造
  复杂度写在注释里：执行体必须 `spawn` 到后台，否则输出帧超过通道容量（64）时
  pump 阻塞在 send 而 executor 尚未开始消费 ⇒ **死锁**。接上出口后这段连同
  `PluginChannel::pair(64)`、`cancel_token` 克隆可一并删除。

**第二批（工具侧收敛，已落地）**：

7. **工具的执行期原语经 `ctx` 传递，不给 `Capability::execute` 加参数**。
   `execute(ctx)` 的入参是**请求信封**（`PATH` / `trace_id` / `payload` / 会话上下文），
   而出口是**执行期**的，与「这次调用从哪条路径来」无关——同一个 `shell` 既可能被
   编排层调用（有出口），也可能被 `route()` 直接调用（无出口 ⇒ 静默）。用 `ctx`
   承载（键 `EVENT_SINK` / `ABORT_SIGNAL`，`SymbioKey` 与 `CAPABILITY_VISITOR` 同款、
   `parse → None` 声明「进程内专用」）就不必为「有没有出口」造第二条调用路径。
8. **工具只声明意图与载荷，节点归编排层构造**。`ask_user` 与交互审批返回
   `Data{failure_kind, prompt}`，节点由 `process_tool_calls_async` 用
   `build_user_prompt_message` 构造（`id = result_msg_id`、`parent_id = tool_call_id`）。
   `PendingPrompt` **没有 id 字段**，因此「同一逻辑节点两个 id」在类型层面不成立。
9. **`failure_kind` 收口为共享闭集**（`symbio_core::capability::failure_kind`），
   判据只有 `is_pending()`。此前生产方与消费方各写各的字面量，编排层另有一处
   硬编码判定——加一个 pending kind 就会漏改它，表现是交互模式下本批剩余工具照跑。

**第三批（流式工具接上出口，已落地）**：

10. **`shell` / `agent_run` 的流式增量走出口，不走自建通道**。收口前它们是
    `PluginPayload::Session` 的最后两个消费者，`tool_executor` 里为此存在一整段
    「解帧 → 过滤 user 角色 → 把 `parent_id` 锚到 `tool_call_id` → `Assistant` 改
    `Tool` → 丢弃 `Reset`/`Warn` → 捕获冒泡审批节点」的二次分派（≈150 行）。
    这是**跨进程传输原语被当作进程内事件流**用。接上出口后翻译只剩
    `subagent::stream_relay_bridge` 一处，`execute_tool_async` 的返回值只有
    「工具结果」一种含义。
11. **`agent_run` 的结局具名化**：通道帧（最终文本哨兵帧 / 错误帧 / 节点冒泡）
    → `RelayOutcome { Done | Pending{text,prompt,failure_kind} | Failed }`。
12. **总时长上限 → 空闲上限**。`EventSinkProgress`（出口上的发射计数）使
    「有进展就不算挂死」可判定：不发事件的工具读数恒 0 ⇒ 退化为总时长口径
    （与历史一致）；会发事件的工具只在真的沉默 600s 后被杀。历史并存的两个
    魔法数（600s 总时长 + 180s 流式空闲）收成一个，长任务不再需要开特例。
    顺带修掉一处**语义回退**：`agent_run` 过去经通道回传、逃过了总时长上限，
    接上出口后被内联等待就会被误杀。

- **新增一条不可回退的约束**：`EventSink::Direct` 的写入者必须**只**经
  `TranscriptSink::apply` 落转写；任何绕过它的直接 `Transcript::apply` 调用都会
  破坏「唯一写入点」。
- **CLI 端到端成为本批的验收手段**（`--provider LMStudio`）：单条流式 / `--repl`
  多轮 / 工具调用（两轮）/ 流式工具四场景。中止路径 CLI 无入口（CLI 只在收到
  会话节点 `outcome == aborted` 时渲染，不主动发起），故由插件级端到端用例
  `handle_abort_signals_registered_turn_and_converges` 逐条锁定对外契约。

---

## ADR-021: 两个执行接口**同形**——`ExecEnv` 具名化，拆信封收口到一处

**状态**：已接受（第四批「签名统一」已落地）。**本 ADR 部分推翻 ADR-020 的决策 7**
（见「被推翻的决定」）。

**背景**：

ADR-020 的三批（E/F/G）把**数据面**收干净了——通道换成出口/信号，工具不再把
通道当事件流。但**签名（控制面）**没动，于是同一个「执行期」概念有两种到达方式：

| | `Capability::execute`（改前） | `ModelProvider::execute_turn`（改前） |
|---|---|---|
| 出口/中止怎么到 | 藏在 `ctx` 的两个无名键里 | **显式参数** `sink` / `abort` |
| 参数怎么到 | `ctx.payload::<Value>()?`（无类型，各工具自读） | 显式类型化参数 |
| 返回值 | `PluginPayload`（4 变体，工具只用 `Data`） | `TurnOutput`（类型化） |
| 信封 | `ctx` 把「路由信封」与「执行期上下文」混在一个键值袋 | 无 |

代价是**每个工具都得记住「我该读哪些键」**：参数、出口、中止、工作目录四件事
混在同一个袋子里，看不出哪两个是执行期必备、哪两个是可选上下文。改一处漏另一处
是必然的（本批就抓到 `subagent.rs` 这个残留：签名换了、体内还回读 `ctx.payload()`）。

**决策**：

1. **新增具名类型 `ExecEnv { sink, abort }`**（`symbio_core::exec`），表示
   「一次带中止的流式执行」的出/入两个方向。两个接口共用它，于是同形：

   ```text
   Capability::execute(args, env, ctx)      -> Result<Value, PluginError>
   ModelProvider::execute_turn(inputs, env) -> Result<TurnOutput, PluginError>
   ```

2. **差别只剩 `ctx`，且这是真实差异**：工具是**被路由、被注册**的（要转发
   `session/chat/send`、要解析 VDFS 挂载、要读会话身份），所以还需要信封；
   模型执行不被路由，也就没有信封。不强行抹平。
3. **`Capability::execute` 返回 `Result<Value, _>`**，不再经 `PluginPayload` 那层
   多态载荷——工具从来只用 `Data` 一个变体，其余三个（`Empty` / `Native` /
   `Session`）是**路由层**的形态，与工具无关。
4. **`invoke_capability(cap, ctx)` 是唯一「拆信封」的地方**：`args = payload ??
   Null`、`env = ExecEnv::from_request(ctx)`、结果装回 `PluginPayload`。
   `DefaultToolVisitor::invoke`、`LocalPlugin::route` 与 `WebPlugin::route` 的工具
   分支都必须经它；装饰器（`PrefixedCapability` / `SecureToolWrapper`）**不拆不装**，
   `(args, env, ctx)` 原样透传。
5. **`ExecEnv::from_request` 的缺席语义照搬 ADR-020**：没有 `EVENT_SINK` ⇒
   `Null` 出口，没有 `ABORT_SIGNAL` ⇒ 永不中止。因此 `route()` 直连调用仍然
   **自然静默**，「有没有出口」仍不需要第二条调用路径。

**理由**：

- **「统一」的判据是调用方能否只看签名就正确调用**。两个接口都变成「输入 + 环境」，
  执行期环境从一个类型取，不再一处显式参数、一处键值袋。
- **拆信封只该有一处**。多态载荷（`PluginPayload`）是路由层的形态，工具不该为它
  付代价；把它收进一个 helper 后，「工具怎么写」与「信封长什么样」重新解耦。
- **保留 `ctx` 是承认真实差异**，不是妥协。硬把 `ctx` 也塞进 `ExecEnv` 会把
  「这次调用从哪条路径来」与「这次调用要怎么跑」重新混成一团——正是本 ADR 要修的毛病。

**代价（逐条核实）**：

- **24 个 `impl Capability` 换签名**：2 个用 `env`（`shell` / `agent_run` 会发增量），
  22 个 `_env`（不发射；前缀即文档）。`ctx` 只在真正需要的工具里保留。
- **签名从 1 参变 3 参**：对「只读一个参数」的工具是净增两行。这是**刻意的**——
  执行期环境是接口的一部分，不该因为它今天没人用就从签名里消失（消失的表现是
  下次有人要发增量时又去 `ctx.get(EVENT_SINK)`）。
- **`#[allow(clippy::too_many_arguments)]` 的账**：`execute_turn` 参数 7 → 6，
  `bound_provider.rs` 与 `model_provider.rs` 两处 allow 删除（阈值 7，已不触发）。

**被推翻的决定**：

- **ADR-020 决策 7「工具的执行期原语经 `ctx` 传递，不给 `Capability::execute` 加参数」
  ——推翻。** 它当时要解决的问题是「不为『有没有出口』造第二条调用路径」，这个目标
  **仍然成立且已由 `ExecEnv::from_request` 的缺席降级保住**；但它选的手段（把出口
  藏在 `ctx` 里）代价是每个工具自己记键名，且与 `execute_turn` 的显式参数形态分裂。
  本 ADR 用「显式参数 + 缺席降级」同时满足两个目标，因此推翻其手段、保留其目标。
- ADR-020 决策 7 的其余内容（`EVENT_SINK` / `ABORT_SIGNAL` 两个 `SymbioKey`）**仍然
  有效**：它们仍是信封承载出口/中止的键，只是读侧从「每个工具各读一次」收口到
  `ExecEnv::from_request` 一处。

**后果**：

- `PluginPayload::new(&json!{...})` 这个包装在 24 个工具里逐处消失；
  `confirm_prompt_payload` / `execute_skill` 的返回从 `InvokeResponse<PluginPayload>`
  收成 `Result<Value, _>`。
- **CLI 端到端**（`--provider LMStudio`）7 个场景：普通流式对话 / 交互模式 `cmd`
  流式工具 / 交互模式 `ask_user`（编排层构造 `waiting_user_action`）/ 自动模式
  `ask_user`（`tool_unavailable` 继续）/ 参数校验失败（`Err` 经 `invoke_capability`
  收敛）/ `vdfs_read`（非流式工具）/ `agent_run`（子会话转播 + `RelayOutcome`）。
- **已知的 CLI 局限（非本批引入）**：CLI 把 `risk_level` 硬编码为 `medium`，而
  工具风险表默认全为 `medium`，故 `needs_approval` 分支**在 CLI 端不可达**；
  该分支的收口语义由 `ask_user`（同为 `is_pending` 闭集）经 CLI 覆盖，加上
  `tool_executor.test.rs` 的跨文件契约用例锁定。

---

## ADR-022: SSE 增量解析——**契约在 core，字段名在协议层**

**状态**：已接受（批次 I「SSE 增量解析归一」已落地）。

**背景**：

为了首字延迟，`parse_sse_stream` 在换行到达之前会先尝试从半截 JSON 里挤出正文。
这件事原先由 **core 内置的启发式解析器**（`try_parse_partial_sse_line`）代劳——
在整行里搜 `"content":"` / `"reasoning_content":"` / `"partial_json":"` /
`"text":"` / `"arguments":"` 五个字面量。三个后果：

1. **加协议要改 core**：新协议能不能增量，取决于 core 那张表里有没有它的字段名。
2. **两条路径两套转义**：core 的 `unescape_partial` 与协议解析器的 `serde_json`
   各实现一遍 JSON 转义，对 `\uXXXX` 的处理不同 ⇒ 「已发送前缀长度」记的是 A 的
   长度、完整行给的是 B 的文本，按前缀截断会**吃字**。这条最隐蔽：不报错，只是
   偶尔少一个字。
3. **每块重扫整行**：缓冲区每增长一次就把整行重新解析一遍 ⇒ 单行极长时 O(行长²)。

**决策**：

1. **契约拆成两个方法**（`symbio_core::sse`）：

   ```text
   SseLineParser::parse_line(line) -> Vec<ProtocolEvent>                     // 完整行
   SseLineParser::open_partial_line(head) -> Option<Box<dyn PartialLineExtractor>>
   PartialLineExtractor::push(bytes, out)                                    // 只吃新增字节
   ```

   `open_partial_line` **默认返回 `None`**，即「本行不做增量提取」——这是合法且正确
   的降级（代价只是首字延迟变大），因此不实现增量提取的协议一行代码都不用写。
2. **`open_partial_line` 每行只问一次**：答 `None` 就记下、本行不再重试。
   收口前那个 O(n²) 正是「每块都重问一次」造成的。
3. **UTF-8 边界对齐写进契约**（`utf8_chunk(buf, from) -> (&str, usize)`）：
   被切断的多字节字符**不消费**，留给下一次 `push`。SSE 分块由 TCP 决定，
   一个中文字符横跨两块是常态；用 `from_utf8_lossy` 会各替换出一个 U+FFFD，
   而完整行给出的是真字符，按前缀截断同样会吃字。
4. **字段名与转义规则全部留在协议层**。四个协议共用一个逐字节推进的 JSON 结构
   扫描器（`plugins/model/protocols/partial_json.rs`），协议只实现三个钩子
   （`begin_string` / `text` / `scalar`），用**键路径全等**判定「这个位置算什么」。
5. **`ModelProtocol` 以 `SseLineParser` 为父 trait**。行解析不是 model 插件的私有
   抽象——它是「core 定义、协议实现」的契约。于是 `BoundProvider::execute_turn`
   直接把协议实例交给 `parse_sse_stream`，core 与协议之间不再有闭包中转。
6. **core 只负责**：按 `\n` 切行 → `parse_line` → 按前缀截断去重（`LineProgress`，
   机制不变）→ 尾巴交给提取器。core 不再认识任何协议字段名。

**理由**：

- **「增量文本恰好是完整行文本的前缀」这条不变量，必须由同一套转义规则保证**。
  把它交给协议层，是因为只有协议层同时掌握「字段在哪」与「怎么解码」；
  让 core 猜字段名，就等于让两处各实现一遍解码。
- **降级必须是免费的**。`open_partial_line` 默认 `None` 让「不做增量」成为零成本
  选项，协议作者不必为了「安全」去写一个空实现。
- **判定用全等而非包含**：错配的代价是把别处的文本当增量吐出去（前端多出内容，
  且完整行按前缀截断会把正文吃掉）；不匹配的代价只是失去增量、退回「等换行」。
  **宁可漏，不可错。**

**代价（逐条核实）**：

- core 删约 115 行（`try_parse_partial_sse_line` / `unescape_partial` / `find_id` /
  `find_name` / `find_idx`）；协议层新增一个扫描器 + 四个 sink。
- **扫描器必须自己实现 JSON 字符串解码**（`\n` / `\uXXXX` / 代理对），并与
  `serde_json` 对齐。这是本决策的**主要风险点**，靠两层测试压住：
  `partial_json.test.rs` 直接比对 `serde_json` 的解码结果；四个协议各有一条
  「**逐字节切分喂进去，增量拼出的文本必须等于完整行解析出的文本**」的不变量测试。
- **对非法 JSON 转义比 `serde_json` 宽松**（原样输出而不是整行拒绝）。分歧只在
  非法输入上出现，此时完整行路径本就不产出事件——最多是多吐一段无人确认的文本。
- **下标未知就放弃本次增量**：`response.function_call_arguments.delta` 的
  `output_index`、`content_block_delta` 的 `index` 若尚未出现，本次不提取。
  用错下标会把参数接到别的工具调用上，比「等换行」糟糕得多。

**后果**：

- `PARTIAL_LINE_MIN_BYTES = 256` 与「增量路径不产出 `Finish` / `Usage` / `Error` /
  `ResponseId`」两条语义**不变**：半截 JSON 里这些字段的值不可信。
- `LineProgress` 前缀截断机制**不变**，只是「增量是完整行的前缀」这条不变量
  改由协议层用同一套转义规则保证。
- 新增 5 个测试文件、+35 个用例（`cargo test --lib` 817 → 852）。
- **CLI 端到端**：本地 mock SSE 构造「单行 809 字节、逐字节写出」的响应，
  四个协议五个场景 stdout **逐字节等于期望文本**；工具参数场景（419 字节 `cmd`
  参数逐字节切分）工具收到完整参数并正常执行；`--provider LMStudio` 真实模型
  冒烟 `EXIT=0`。

---

## ADR-023: `symbio_core` 的准入规则 = **依赖方数量**，不是「够不够底层」

**状态**：已接受（批次 J 期间由用户明确，并据此回退了已写下的两处）。

**背景**：

`symbio_core` 是**跨模块的系统架构**，不是通用工具箱。它的实际作用是给
**互相不可见**的插件提供唯一的共同可见处——插件 A 要跟插件 B 说上话，只能经由
core 里的契约，因为 A 看不见 B。

批次 J 我（AI）以「这是跨插件约定」为理由，往 core 里加了两样东西，都是错的：

1. **`symbio_core::tool_result`**——把工具结果的字段名读取器
   （`extract_result`）提为 `text_of`。**只有 `session` 一个模块依赖它**。
2. **`CapabilityVisitor::resolve_name` 默认方法**——名字解析。同样
   **只有 `session` 一个消费方**。

判错的根源是把「**约定**」与「**跨模块**」当成一回事。约定可以只在一个模块内
成立（`extract_result` 的判定顺序就是一个纯 session 内部约定）；而 core 里的
每一行都要能被**至少两个互不可见的模块**分别依赖，否则它进 core 只是让
「谁都能用」的错觉代替「谁在用」的事实。

**决策**：

1. **判定依据是依赖方数量**：只被一个模块依赖的内容一律**下沉回该模块**。
   不问「它够不够底层」，只问「除它之外，还有谁依赖」。
2. 内容进 core 时，**在模块文档里写下依赖方对照表**：谁依赖、依赖哪个函数、
   改它要同时改谁。`symbio_core/tool_name.rs` 是范本——它同时列出
   `to_wire`（被 `model` 插件依赖）与 `resolve`（被 `session` 插件依赖），
   并说明两者是**同一契约的两半**（改一个不改另一个 = 静默错位）。
3. **模块文档里写反面例子**：`tool_name.rs` 明确记下「工具结果字段名读取器只有
   一个消费方，留在原地」，让下一次想往 core 加东西的人先看到自己被拒的先例。

**理由**：

- core 的**体积即耦合面**。往里加一个单消费方的东西，收益是省一行 `use`，
  代价是让一个模块的内部实现细节获得「架构级」地位——之后想改它，要先说服
  所有读 core 的人它不是契约。
- 「够不够底层」是**主观且无法证伪**的判据，最终会变成「我觉得它挺通用」。
  「依赖方数量」是可核对的：`grep` 一遍就有答案。
- 批次 J 的两处回退不是形式主义：`tool_result` 若留在 core，`session` 将来换
  结果形状时会被 core 的公共 API 锁住；`resolve_name` 若留在 core trait，每个
  `CapabilityVisitor` 实现都要背一个只有 `session` 用的默认方法。

**代价（逐条核实）**：

- 同一份逻辑若真被第二个模块需要，**要等第二个消费者出现再上提**——期间可能
  先复制一份。这是**有意接受**的：复制是可见的债，过早抽象是不可见的债。
- 依赖方对照表要**手工维护**。它不生成、不校验，靠评审。缓解办法是把它写进
  模块文档首屏（而不是藏在函数注释里）。
- 「跨模块」本身仍需人工判断（两个模块是否真的互不可见）。本项目里插件之间
  确实互不可见，所以「插件 A 与插件 B 共同依赖」是清楚的判据；换成别的架构
  形态时这条要重新论证。

**后果**：

- `symbio_core::tool_name` 保留（**两个模块**依赖：`model` 出、`session` 入），
  且带依赖方对照表 + 反面例子。
- `symbio_core::tool_result` 删除；`extract_result` 退回
  `session/tool_executor.rs`，就地升级为「判定顺序即约定」+ 两个字段名常量
  + 10 个用例。
- `CapabilityVisitor::resolve_name` 撤回；解析逻辑内联进
  `session/tool_executor.rs`，只调用 core 的**纯函数** `tool_name::resolve`。
- 批次 J 新增 core 内容**仅** `tool_name` 一处。

---

## ADR-024: 会话选项并入**详情方言**——「选项行」是配置表单的字段，不是独立协议

**状态**：已接受（2026-09-23 实施完成，门禁全绿）。

**背景**：

「新增一类资源、前端零改动」靠一条统一链路维持：后端在列表 / 详情节点上挂
`schema`（`DetailDefinition`），前端用唯一渲染器 `DetailForm` 解释它。
model / agent / skill / mcp / setting 分区都走这条。

会话是**唯一例外**：它自带一套 `OptionNode` 协议——定义走专用端点
`session/options/list`（`parent` 懒加载）、当前值由贡献方在节点上算好
`value` / `value_label`、写回走专用端点 `session/update`（`action.payload` +
`bind` 点路径）、跨插件汇聚另走 `OptionVisitor` + `traverse(available_options)`，
前端另有一套 `useSessionOptions` + `ChatOptionBar` + `OptionFormDialog` +
`services/options.ts` + `schemas/options.ts`。

两套机制描述的是**同一件事**：一组「有当前值、可改、改完要落库」的字段。

**决策**：

1. **选项 = 会话配置表单的字段**；选项栏 = `DetailDefinition` 的**第二种渲染形态**
   （与 `DetailForm` / `DetailShell` 并列），不是新协议。
2. **定义随节点下发**：已落盘会话挂 `node.schema`，草稿态挂 `new_type.schema`
   （同一构造、两处投递；字段名见 [ADR-027](#adr-027-可新建的东西至多一种类型内挂可选导入入口)）。不再有「取定义」的专用端点。
3. **当前值随节点**：`node.attributes.metadata`（线上是摊平后的顶层 `metadata`，
   见下「踩坑」）。
4. **写回走 `vdfs/write(<根>/session/<id>, {"metadata": …})`**——
   `VdfsProvider::write` 的覆盖分支本就是 metadata 浅合并，不需要第二条写入路径。
5. 给共享方言补四件事，**都通用、非选项专用**：`DetailField.icon` /
   `DetailField.disabled_when`（文档已自认的不对称）/ `DetailField.pick`
   （原生取值闭集）/ `DetailField.form` + `widget="form"`（结构化子表单）。
6. **旧机制整体下线**：`OPTIONS_LIST` / `SESSION_STATE_ENDPOINT` / `OPTION_TYPES` /
   `OPTION_PICKS` / `schemas/options.rs` / `options/list` 路由 / `session/update`
   路由与 `schemas/session/session_update.rs`。`OptionVisitor` + `traverse` **保留**
   （汇聚机制不变），只把产物从 `OptionNode` 换成 `(order, DetailField)`。

**理由**：

- 会话选项与其它资源的「新建表单」在语义上**没有区别**（字段 + 默认值 + 候选 +
  条件可用性）。承认这一点后两条链路可以合成一条，收益是**前端零业务字段名、
  零业务枚举**。独立协议让每个字段名都要在前后端各写一遍，而没有任何守卫能发现
  这种漂移（ADR-019 记的正是这类「手工镜像」的代价）。
- 四件方言补充都是**通用能力**，不是为选项开的口子：任何 provider 的详情表单都用
  得上 `icon` / `disabled_when` / `pick` / `form`。把它们加进共享方言，比让选项
  单开一套私有字段更不容易腐坏。
- **`session/update` 的退役不需要扩协议**：`VdfsProvider::write` 的文档与
  `vdfs_service::entry::id_of` 的注释**早已**写明「**具名节点** + 不存在 ⇒ 就地创建，
  id 来自地址；只有**目录自身**才由 provider 生成名字」。会话 provider 是**唯一**
  违反者（无论有无名字都自己生成 id、把名字只当标题）。让它遵守既有规则即可，
  `--session <ID>` 语义与 CLI 会话 id 格式都不用动。
  > 这里差点做错：第一版方案是「给 `vdfs/write` 的 `create` 加 `id` 字段」——
  > 那是**真·扩协议**（与「id 是存储细节」冲突，且会让 8 个按固定 id 读落盘会话的
  > e2e 用例失效）。**给通用机制加字段之前，先看特例是不是在违反通用机制已有的
  > 成文规则。**

**代价（逐条核实，不粉饰）**：

- 级联选项从「任意深度」收窄为**一层**（现存实例全是一层）。
- `invoke` 型命令选项改由 `DetailAction` 承担（现存实例 0 个）。
- 懒加载取消：定义随节点全量下发。选项只有 6 行，量级无虞；将来字段数上量时
  这条要重新评估。
- `order` 号段留在后端收集层、不下发（只用于排序，不是数据）。
- 遗留：`/session <拼错的 ID>` 仍会**静默创建**一个空会话——`session/update`
  时代同样如此，非本次引入。

**后果**：

- 前端删除 `useSessionOptions` / `services/options.ts` / `schemas/options.ts` /
  `registry/optionIcons.ts`；实现收敛为 `useSessionOptionBar.ts` +
  `ChatOptionBar.vue` + `OptionFormDialog.vue`。
- `protocol-mirror-audit`：C 组撤 `OPTION_TYPES` / `OPTION_PICKS`，D 组撤选项 5 对
  （23 → 18 对）；S1 新增 `DETAIL_PICKS`。
- CLI `ensure_session` 改为一次 `vdfs/write`（一次调用即 upsert），启动期经
  `vdfs/root` 取回根地址；子智能体登记子会话改走进程内 VDFS 纯接口，
  不再绕路由。
- `session` 的静态路由臂从 4 条降到 3 条（`chat/send` · `chat/abort` · `stream`）。

**踩坑（写下来免得下次再踩）**：`VdfsNode.attributes` 是 `#[serde(flatten)]` 的，
线上（`vdfs/list` / `vdfs/stat` JSON、前端 `services/session.ts`）看到的是**摊平**
后的节点——`metadata` 是**顶层键**，没有 `attributes` 这一层。Rust 侧才读
`attributes`。写断言读 `node.attributes.metadata` 会得到 `undefined`。

**详细记录**：取舍与实施注记见
[`archive/session-options-unification.md`](./archive/session-options-unification.md)；
现行规范见 `symbio/src/plugins/session/docs/session-options.md`。

---

## ADR-025: 顺序是**节点属性**；`delta` 是 `updated` 的**传输形态**

**状态**：已接受（2026-09-23）。

**背景**：

VDFS 是**虚拟动态文件系统**（`d` = dynamic）——节点可能落盘、也可能只在内存里，
但一律以**文件系统的语境**访问。会话转写在这个语境里本来就有唯一自然形态：

```
<根>/session/<id>/message            一个文件夹
<根>/session/<id>/message/<mid>      一个文件（一条消息）
流式输出                          该文件的内容在增长
```

这套形态**数据面早已落地**：`transcript_of` 返回「落库转写 ∪ 本轮在途缓冲」，
`read(<id>/message/<mid>)` 拿到的是**已含增量**的正文，`list` 按 `ChatMessage.seq`
排序（`ordered()` 的注释写着「`seq` 是唯一权威顺序锚点」）。

**但通知面走偏了**，且偏了两次：

1. **S16–S19**：把消息挂到 VDFS 变更频道上（`appended` + `delta`、`truncated`、
   `renamed` + `to`、`node` / `content`）。S23–S25 拆掉，消息改走独立的
   `session/stream` 转写流（`symbio_core::transcript_stream`）。
2. **S22–S25 + 批次 E/G**：会话运行态也并入转写流，与消息共用 `seq`；`VdfsChange`
   收窄为 `{path, change}` 三个取值。

两次拆解的理由写在 `docs/design/vdfs.md` §9 与 `vdfs_provider.rs` 的「变更通知」小节：

- **频道没有流内序号**（丢帧不可检测）；
- **载荷全量/增量混合**（消费端必须猜「这次是追加还是替换」）。

**这两条是当时的正确判断，但它们被记成了能力性结论**，其中一句原文是：

> `appended` 的设想是对的（逐帧只带增量，否则 O(n²)），但它需要的是**一条有序流**
> （有流内序号、有背压恢复），而不是在资源变更频道上挂一个 `delta` 字段。

**这句话是错的**，而它是后来一切推导的前提。错的不是观察，是**归因**：它把
**数据的属性**（顺序）误当成了**传输的属性**。

**决策**：

1. **顺序是节点属性，不是投递属性。** `ChatMessage.seq` = 消息在该文件夹里的位置，
   由写入者分配、随节点下发；消费端按它排序。前端**已经**如此
   （`useChatConnection.ts` 的 `rootMessages.sort(a.seq ?? a.timestamp)`）。
   于是**到达顺序与显示顺序无关**：后生成的先到、两个并行工具的变更混着到，
   都正确显示——因为每个变更都指向一个明确的 `path`，节点自带位置。

2. **「追加」不是新取值，是 `updated` 上的一个 `delta` 字段。** 消息流的设计经验
   （`ChatMessage` 帧）是「**帧就是节点视图，没有操作枚举**」——`delta` 有 ⇒ 尾部
   追加、`content` 有 ⇒ 整条替换、`status = removed` ⇒ 就地移除，**语义由字段本身
   给出**（`ChatMessage.delta` 的文档写着「与 `content` 互斥，语义由字段本身给出」）。
   VDFS 变更沿用同一条思路：

   | `change` | `delta` | 消费端动作 |
   |---|---|---|
   | `created` | — | 插入该节点（回读拿全貌） |
   | `updated` | **有** | **尾部追加**，零回读 |
   | `updated` | — | 节点变了，回读（`stat` / `read`） |
   | `deleted` | — | 就地移除 |
   | `created` / `deleted` 带 `delta` | — | **协议违例**（与「`delta` / `content` 互斥」同源，报错丢弃） |

   **取值集合不变**（仍是 `created` / `updated` / `deleted`），只加一个可选字段。
   消费端不需要猜「这次是追加还是替换」——`delta` 的有无就是答案。任何「内容会
   增长」的 provider 都能用它（转写、日志、流水），机制里**没有任何会话特化**。

   ## 与消息流逐条同构

   | `ChatMessage` 帧（消息流） | `VdfsChange`（VDFS 变更） |
   |---|---|
   | `delta` 有 ⇒ 尾部追加 | `updated` + `delta` ⇒ 尾部追加 |
   | `content` 有 ⇒ 整条替换 | `updated` 无 `delta` ⇒ 回读（节点形态在 `read` 里） |
   | `status = removed` ⇒ 就地移除 | `deleted` ⇒ 就地移除 |
   | 未知 id 用帧内信息建占位 | `created` ⇒ 插入 |
   | **`delta` 是传输形态，`content` 是节点形态** | **`delta` 是传输形态，`read` 是节点形态** |
   | 同帧 `delta` + `content` = 违例 | `created` / `deleted` 带 `delta` = 违例 |
   | `seq` 是顺序锚点（**节点属性**） | 同（`ChatMessage.seq`，与投递无关） |

   > `Transcript::apply` 里那句注释是全部要害：**「图里只留累积后的 `content`：
   > `delta` 是传输形态，不是节点形态。」** VDFS 侧一字不改地照搬。

3. **不加快照载荷**（`node` / `content`）。理由是**独立于顺序**的：快照携带的是它
   **生成那一刻**的状态，迟应用会把运行态回退。这是**陈旧写入**问题，不是乱序问题；
   而回读（`stat` / `read`）永远最新且**幂等**，是正解。原文那句
   「快照的来源必须**有序或幂等**」——**只有「幂等」那一半是对的**。

4. **丢失靠幂等重读兜底，不靠序号。** 三层，**全部已实现**：
   - `event_bus::try_publish` 满通道 → 保留订阅 + 补送 resync 标记（`ea8a460`）；
   - 会话终态 → 300ms 宽限定时器 → 复查本地非终态节点 → 整份回读（`ea8a460`）；
   - `subscribeVdfsChanged` **订阅即登记**（登记先于加载，避免启动窗口漏变更）。

5. **`session/stream` 退役，`transcript_stream.rs` 删除。** 它存在的三条理由现在
   都不成立：顺序（不是投递属性）、背压（`event_bus` 已对等）、免回读（由
   `appended` + `delta` 直接提供）。

6. **判据不变**：一个变更取值（或载荷字段）必须有**生产性生产者**。批次 G 删除时
   确实**没有**（消息域已迁走）；现在消息域搬回来，**有了**。结论不同是因为输入
   不同，不是反复。

**理由**：

- **热路径用载荷、冷路径回读**——这正是消息流的经验，而原文那句「**热路径与冷路径的
  区别不足以支撑载荷**」把它说反了：消息流的高频帧（逐 token）**全部**带 `delta`，
  只有低频帧（创建 / 终态 / 压缩）才回读。VDFS 侧同理：`delta` 覆盖绝大多数帧量，
  状态迁移与创建各一次回读。**载荷该不该有，取决于它落在热路径还是冷路径上——恰恰
  是原句否定的那个判据。**
- **push 增量不是必需的**（消费端也可按游标拉取），但它是**更省的**：一条 5KB 的
  消息跑 10s，push ≈ 总增量 5KB，pull ≈ 全量 × 每秒次数。既然通道已有 resync 兜底，
  没有理由为了「可丢」而放弃它。
- **精确缺口检测不是必需的**：resync 给的是「你可能漏了，重读你的作用域」——粗粒度，
  但重读幂等，**足够**。
- **`transcript_stream` 的背压纠正已经上移到 `event_bus`**（`ea8a460` 明写「本次把
  同一条纠正补到本频道」），所以「一条机制」的成本已经从两处降到一处。
- 合并之后 **`event_bus` 的 `KIND_VDFS` 就是唯一实时通道**，`symbio_core` 里不再有
  第二个投递设施；新域（MCP 工具流、模型流）**不需要再造一个**。
- **顺序与顺序敏感无关**：`ChatMessage.seq` 是**节点属性**（`ordered()` 的注释写着
  「`seq` 是唯一权威顺序锚点」），前端 `sortTranscript` 按它排、缺失回退 `timestamp`。
  两个并行工具的变更**混着到**、后生成的**先到**，显示都正确——因为每个变更都指向
  一个明确的 `path`，而节点自带位置。**顺序从来不是投递层的职责。**

**代价（逐条核实，不粉饰）**：

- `VdfsChange` 加宽 → 跨栈形状（`protocol-mirror-audit` 的 C 组 / D 组镜像）、
  `map_paths`、`to_change_event` / `VdfsChangeEvent`、前端 `schemas/vdfs.ts`
  与 `useVdfs` 的 `appended` 快速路径（批次 G 刚删的，要加回）。
- **通知量回到「每 token 一条」**：`event_bus` 订阅通道容量 2048 < `session/stream`
  的 4096，需与后者对等或更高；否则 resync 频率上升（正确性不受影响，但会多出
  整份重读）。
- **`ChatMessage.seq` 建议提前到「消息创建时」分配**（现在只在落库时分配，在途消息
  只能靠 `timestamp` 兜底；两条并行在途消息在 `timestamp` 同毫秒时无权威顺序）。
  `Transcript.seq` 因此**不删、改语义**：从「帧序号」变成「位置序号分配器」。
- **反向风险**：`VdfsChange` 加宽后，下一个改动者可能按「零生产者」再删一次。
  必须在词汇表旁**同时**留下生产者清单与本节链接。
- 遗留：`VdfsMessageDetail` 的「本视图不随流式增长」可以修好了——它当时不增长是因为
  消息不产生 VDFS 变更，现在产生了。

**详细记录（实施设计）**：
[`archive/session-realtime-vdfs-watch.md`](./archive/session-realtime-vdfs-watch.md)
——含「现行消息流的六条设计」逐条出处、读与流的协调规则（`appendGuard` 加回）、
`Transcript.seq` 的语义变更（帧序号 → 位置序号分配器）、逐文件改动清单与同批约束。

**这次纠正的范围（不只本 ADR）**：把「顺序是投递属性」当成前提的**历史性理解错误**
落在多处文档，已一并纠正——`docs/design/vdfs.md` §9、`vdfs_provider.rs` 的「变更通知」、
`docs/reference/ROUTES.md`、`docs/design/http-api-transport.md` §5.3、
`docs/architecture/PROTOCOLS.md` / `DATA_FLOW.md`、`docs/design/vdfs-frontend.md`、
`session/docs/node-state-streaming.md`（§8 不变量 #10 / #20）、
`session/docs/vdfs-session-messages.md`（S26 续）、`session/docs/core-loop.md`、
`docs/archive/legacy-route-migration.md`、`cli/docs/architecture.md`、`cli/src/client.rs`
模块文档、`VdfsMessageDetail.vue` 头注释，以及两份评审文档的**后记**。
**archive 与评审类文档不改写正文，只加后记**——它们是「某一时刻的判断」，
后记才是本次要留下的东西。

**追记（S27，2026-09-23）：操作枚举整个退役，信封收敛为 `{path, data?}`。**

本 ADR 落地时 `delta` 以 `updated` 的可选字段回归、取值集合保持三个；同日对形状
的进一步质疑推翻了「取值集合不变」——信封上的 `created` / `updated` / `deleted`
描述的「资源层面发生了什么」，与载荷描述的「业务数据变成了什么」是同一件事的
两种说法，而消费端真正消费的只有后者。最终形态：

- **信封 = `{path, data?}`**：`data` 是该路径的业务载荷（消息节点上是
  `ChatMessage`、会话节点 `<sid>` 上是 `VdfsNode`），缺失 = 无载荷（回读收敛）。
  `delta` 回到它本来的位置——**`ChatMessage.delta` 字段本身**，不再是信封上的副本。
- **`path` 恒为被变更节点自身的地址**（S27 收口同日定稿）：集合项的地址形状统一为
  `<sid>/<集合段>/<项 id>`，消息的落点是 `<sid>/message/<mid>`，身份就是末段。
  会话是**容器**，其下是若干**并列的集合**（消息 / 子会话 / 记忆 / 工作目录，后续还会
  有任务列表、请求队列……），所以机制不认识任何一类集合——新增一类集合只需在
  `internal_dirs` 里声明一个段，信封与消费端都不动。
  初版发的是**目录**（`<sid>/message`）而把身份交给 `data.id`，两处代价在收口时暴露：
  ① `path` 的含义随帧类型漂移（资源信号是节点自身、消息是它所在的目录），消费端必须
  **反推地址**才能回读；② 「目录 + 载荷里的 id」无法推广到第二类集合。
- **删除的表达**：资源域 = 「载荷缺失 + 回读 `NotFound`」（删掉的节点本就没有
  视图可带，恰好不需要一个 `deleted` 类型）；消息域 = `ChatMessage.status =
  removed`（消息词汇本就有它）。
- **门面不再换信封**：`to_change_event` / `VdfsChangeEvent`（与 `VdfsChange`
  形状逐字相同的影子类型）合并删除，总线上的形状与 provider 侧逐字一致。
- **运行态随载荷带全量节点视图**（与 `stat` 同一构造点 `session_node`）：
  消费端零回读就地收敛；资源信号保持无载荷，消费端防抖重拉。

代价逐条核实：后端 `VdfsChange` / `map_paths` / 各 provider 的 notify 调用点、
前端 `schemas/vdfs.ts` 与四个消费端（`sessionTranscriptSync` / `sessionNodeSync` /
`sessions` / `useVdfs`）、协议镜像审计（D 组 `VdfsChange` 2 字段镜像）。
现行规范见 [`design/vdfs.md`](./design/vdfs.md) §9 的「变更通知」小节。

---

## ADR-026: 子智能体的完整会话**在 VDFS 空间内自驱动**——用户消息入 `inbox` 集合，进程内唤醒消费，不走 `route`

**背景**

- `agent_run` 是「在父会话下派生一次性子会话」的**工具**，产出归档在父会话的 `subsession` 集合。钉死这一点是为了不把它和「子智能体」混为一谈。
- 「子智能体」是一等**空间**：`agent/<id>/` 已装配为**含自身 `session` 插件实例的完整子树**，其存储目录就是自己的插件目录。也就是说独立空间结构上早已存在，缺的不是空间，而是**在没有调用方连接时驱动它跑一轮的入口**。
- `chat/send` 是请求-响应式的，它把「接收消息」与「跑一轮」绑在一起，并依赖 `route` 的调用方连接来观测输出。
- ADR-025 已确立**显示由节点状态驱动**：一轮的输出来自 VDFS 变更，本就不需要调用方连接——这是空间可以脱机自驱动的**前提**。

**决策**

1. 用户消息**只是 VDFS 写入**：向 `<sid>/inbox/` 写一条新条目即入队，条目本身就是一条待消费的 `user` 消息。
2. **运行由空间自身消费队列**：写入后在同一进程内唤醒该会话的收件箱消费者，消费者逐条取出并调用既有的一轮执行路径。
3. **机制统一到所有会话**，不特设子智能体分支——顶层与子智能体共用同一套 `inbox` 语义，子智能体只是「恰好没有人替它调 `chat/send`」的那种会话。
4. **FIFO、忙则排队**：会话正在跑时新消息只入队；消费者严格「跑完一轮再取下一条」，不并发、不抢占、不合并。

**理由**

- VDFS 已有空间与挂载遍历（`agent/<id>/session/<sid>` 本就可达）。把「发消息」表达为对空间的一次写入，就不必为子智能体复制一套 route 入口。
- 会话内部集合**按设计可扩展**：新增一类集合只需在声明处加一个段，信封与消费端都不动。`inbox` 正是「再加一类集合」，与 ADR-025 的集合模型同构。
- 「写即入队、忙则排队」用一条队列同时表达投递与背压，投递方无需等待，天然适配无连接的脱机空间。
- 不把 `chat/send` 的请求-响应语义套到子智能体上，避免「需要一个此刻并不存在的调用方」的自相矛盾。

**后果**

- `chat/send` 退化为「写 `inbox`」的**薄包装**，顶层调用方的现有体验不变（响应仍是 `accepted`，
  输出仍由 VDFS 变更推送），语义 owner 收敛到收件箱（机制见 `session/inbox.rs`）。
- 活动会话状态上新增队列；消费者是**每插件实例一个**、在构造点起，不是在写入点按会话起
  ——否则「第一次写入由谁消费」会取决于装配顺序，不可测。
- 忙碌不再由单次调用隐式携带：**一批**待办与「正在跑」各由队列和运行态表达，`resume` 分支
  仍然只把忙碌当拒绝条件（恢复必须落在当时那条消息上，排队会改变候选）。
- 取消一条**已排队**消息 = 删除对应 `inbox` 条目；**正在处理**（已出队）的那条已是转写里的
  消息，取消走既有 `chat/abort`——两种取消作用在不同对象上，因此是两个动作。
- 取向与从前相反：新消息**排队**而不是抢占当前轮次。换来的是「没有调用方的空间也能被
  可靠驱动」这一条最硬的保证（e2e `t15-subagent-inbox` 覆盖 FIFO / 忙则排队 / 可取消 /
  子空间自带指令与模型）。

---

## ADR-027: 可新建的东西**至多一种**——入口是类型下的可选分支，不是第二个类型

**状态**：已接受（2026-09-24 实施完成）。**第 2、3 条已被 ADR-029 取代**——
「整包导入」不再是一种入口形态，而是详情页上的一条动作；**第 4 条已被 ADR-030
取代**——`path` 不在节点上，落在 `VdfsItem` 条目上（同一条「使用方回填」的口径，
换了 owner）。

**背景**

`VdfsNode.new_types: Vec<VdfsNewType>` 原是一份**清单**：一个目录可以声明「可接受
的新建元素类型」若干种，前端拿到清单后让用户先选类型。它带来三处结构性的别扭：

- **一个 provider = 一棵子树 = 一种资源**，「本目录能建的东西」自然只有一类。
  清单形态下每个目录都要为「我到底能建几种」维持一份判据，而这份判据没有任何
  使用方真正需要——前端拿到清单后仍要按 `ext` 反查渲染器，两处知识必然漂移。
- **「几类东西」与「这类东西有几种造法」被压进了同一个轴**。skill / mcp 目录
  真正有两件事：① 表单新建一类配置；② 整包导入一个 zip。原实现把 ② 表达成
  **清单里的第二项**（`ext = zip` + `source = file`），于是「类型数」= 2，
  而用户看到的是「新建 → 选类型」——把一个**造法**的选择说成了**类别**的选择。
- **`VdfsNode.path` 的连带负担**（同一轮一并收敛，见下）。

**决策**

1. **`VdfsNode.new_type: Option<VdfsNewType>`**（`new_types` 清单退役）：
   至多一个，`None` = 该目录不可新建。trait 方法
   `VdfsProvider::new_types() -> Vec<_>` 随之收成
   `root_new_type() -> Option<VdfsNewType>`（仍 async，仍不进 `PluginMeta`——
   session 的表单 schema 需运行期汇流）。
2. **类型与入口分离**。类型 = 「一类东西」（至多一个）；入口 = 「怎么把它造出来」
   （至多两条）：**主入口**恒有，**整包导入**由类型内可选的
   `import: Option<VdfsNewImport>{ext, title, description?}` 声明。
   前端由 `vdfsNewEntries(newType)` 推导出 1~2 条入口，动作 id 固定为
   `new:type` / `new:import`。
3. **前端「选类型」这一档改造成「选入口」**：`useVdfsPrompt` 的判别式从
   `'type'` 改为 `'entry'`，载荷从「类型」改为「入口」；因为类型只剩一个，
   `ext` 已无法区分入口，动作 id 因此不再按 `ext` 派生。
4. **`VdfsNode.path` 只作**展示口径**：它是「使用方回填」的字段，provider 一律
   产出**树内相对地址**，由容器 / 门面补全为展示地址——provider **不得**据此做
   特判（原先存在按完整 path 判前缀的实现，随本轮一并清理）。

**理由**

- 「至多一个」让「能不能建」退化成一个 `Option`，判据只有一个；
  「能建什么」与「怎么建」各占一条轴，各自只有一个 owner。
- 类型数 = 1 而入口数仍可为 2 ⇒ **UI 与能力零损失**：skill / mcp 仍是
  「新建 → 选（表单新建 / 整包导入）」，只是这一步的语义从「选类型」正名为
  「选入口」。agent 包仍是单入口（主入口即选文件，不挂 `import`）。
- 收敛的是**声明**，不是行为：`vdfs/write { create: true }` 的两条目标形态
  （具名 / 目录自身）与整包导入的字节通道一个字都没改。

**后果**

- 后端：`VdfsNewType` 新增 `import` 字段；`VdfsNode.new_types` → `new_type`；
  五个 provider 的 `root_new_type()` 各自返回 `Some(...)`（agent 主入口即选文件、
  skill / mcp 挂 `import`、model / session 单入口）。
- 前端：`schemas/vdfs.ts` 新增 `VdfsNewImport` / `VdfsNewEntry` / `vdfsNewEntries`；
  `useVdfs` 的 `creatableTypes` → `creatableType` + `newEntries`；
  `useVdfsPrompt` 的 `'type'` 态 → `'entry'` 态。
- `vdfsScheme` 的挂载点识别从「遍历类型清单找 session」简化为
  `n.new_type?.ext === VDFS_EXT_SESSION`——判据变短是收敛的直接证据。
- 本 ADR 同时**改名** ADR-024 引入的 `new_types[].schema` 为 `new_type.schema`
  （同一构造、两处投递的决策不变，只换了标识符）。

---

## ADR-028: `VdfsRequest::Move` **不收**——移动是外层组合，不是核心原语

**状态**：已接受（2026-09-24 实施完成，整条链下线）。

**背景**

`VdfsRequest` 是 `VdfsProvider::dispatch` 的**操作载荷**，而 `path` 是第一个分发键：
**一个 provider = 一棵子树**。原实现里有一个例外——`Move { to }` 带**两个地址**
（`path` 是源、`to` 是目标）。它的代价在跨子树时才显形：

- **同一棵树内**移动 = 重命名，provider 能省事（物理盘直接 `rename`）；
- **跨虚拟挂载树**时 `from` / `to` 分属两棵子树，「移动」就不再是原语，而是
  `copy + delete`。trait 上的 `Move` 表达不出这件事，只能由某个 provider 假装
  自己同时拥有两端——`composite` 就不得不先解析 `to` 属于哪个子目录、再拒绝
  跨目录的情形，即**用错误表达能力的缺失**；
- 于是 `Move` 只有**恰好一个**实现者能真做（物理盘 `rename`），其余一律
  `NotImplemented`；而「用不了」的形态在 trait 上占着一个位置，还逼着每个
  实现方写一条拒绝臂。

**决策**

1. **`VdfsRequest::Move` 删除**，且**不应加回来**。删除后 `VdfsRequest` 只收拢
   操作载荷——**地址一律走 `path` 参数，载荷里没有任何地址字段**。
2. **`VdfsRequest::map_paths` 随之删除**（它存在的唯一理由是翻译 `Move.to`）；
   `VdfsChange::map_paths` **保留**——它是变更事件路径翻译的唯一入口
   （见 ADR-015），与请求无关。
3. **外层那条链一并下线**：协议操作 `vdfs/move`、LLM 工具 `vdfs_move`、
   前端「重命名」入口与提示态。要用移动由**外层组合**（`copy + delete`），
   当前外层**也不提供**——需要时再加，加在外层而不是核心 trait 上。

**理由**

- **删掉之后，一类错误在结构上不可能发生**：载荷里没有第二个地址，于是
  「跨半移动」（物理 ↔ 虚拟）与「跨子目录移动」都没有可表达的形式。
  原先那两条守卫（`UnifiedFs::same_half` 判定 + `composite` 的跨目录拒绝）与
  它们的用例一起消失——**消失是因为要守的形态没了**，而不是为了过门禁而放宽。
  这比「运行时判前缀再拒绝」是更强的保证。
- 拒绝面收窄不等于能力收窄：`vdfs/move` 的唯一真实用途是物理盘 `rename`
  （前端「重命名」）。而重命名在**配置型资源**上语义本就可疑——那些资源的
  地址**就是它的身份**（`model` / `mcp` / `skill` 皆如此），改名等于换一个对象。
  故「先不提供」是可接受的取舍，且方向正确：能力将来加在外层，核心层不受影响。
- `VdfsRequest` 与 `VdfsChange` 从此各自只有一条翻译入口、各自只翻译一种东西
  （请求：`path`；事件：全部路径字段）——两处 `map_paths` 只剩一处。

**后果**

- 后端：`vdfs/move` 常量与 `VdfsMoveRequest` / `VdfsMoveResponse` 删除；
  `VDFS_OPS` 14 → 13；`tools/move.rs` 整个文件删除，工具集 10 → 9；
  `UnifiedFs` 的 `dispatch` 退化为只做 `route(path)` 分流（无跨半判定）；
  `PhysicalFs::do_move`、`ToolVdfs::move_item`、`CompositeVdfs` 的跨目录拒绝、
  agent provider 的 `move_at` 全部删除。
- 前端：`VDFS_MOVE` / `VdfsMoveResponse` / `moveVdfs` 删除；`useVdfs` 的
  `mechanismActions` 只剩 `delete`（机制动作从两条收成一条）；
  `useVdfsPrompt` 的 `'rename'` 态与它特有的 `watch(selectedNode)` 联动删除。
- 基线：`rustTests` 930 → 926、`vitestTests` 727 → 722——**净减是预期的**，
  逐文件核对写在 `scripts/gate.d/_shared.mjs` 的对应注记里。
- `docs/CURRENT.md` 的协议操作表（13 个）与工具清单（9 个）由生成脚本自动跟随，
  不需要手改。

---

## ADR-029: 导入是**详情页动作**，不是类型入口——`VdfsNewType` 收成纯呈现定义

**状态**：已接受（2026-09-24 实施完成）。取代 ADR-027 的第 2、3 条。

**背景**

ADR-027 把「可新建类型」收敛为至多一个，同时把「整包导入」表达成**类型内的
第二种入口**（`VdfsNewType.import: Option<VdfsNewImport>`），前端据此先问
「选哪种方式」。这一版留下三处不对称：

1. **一对逆操作分在两个层级**。导出早已是 `vdfs/action` 上的 provider 自持动词
   （`VDFS_ACTION_EXPORT`）；导入却占着 `VdfsNewType` 的一个字段——「打包出去」
   与「解包进来」在协议上成了两件不同性质的事。
2. **一个呈现结构承担了操作语义**。`VdfsNewType` 的其余字段
   （`ext` / `title` / `icon` / `node_ext` / `schema`）全在回答「落成后长什么样」；
   `source` / `import` 回答的却是「怎么把它造出来」。
3. **`VdfsProvider` 上多出一个系统级接口**。为了把 `VdfsNewType` 交出去，trait 上
   加了 `root_new_type()`——而该 trait 的模块文档写着「只暴露纯接口：
   `VdfsProvider` 只有一个方法 `dispatch`」。更别扭的是**只有根**有这条通道：
   任何非根节点若想自述「我这里能新建什么」，只能再开一个方法——而
   `VdfsNode.new_type` 本来就长在节点上。

**决策**

1. **导入改走 `vdfs/action`**：新增 `VDFS_ACTION_IMPORT`（与 `VDFS_ACTION_EXPORT`
   互为逆向），载荷是入向的 `VdfsUnpack{filename, b64}`——与出向的 `VdfsPack`
   同形，只差出向独有的 `id`。
2. **`VdfsNewType` 收成纯呈现定义**：只剩
   `ext` / `title` / `description` / `icon` / `node_ext` / `schema`；
   `source` / `import` / `VdfsNewImport` / `VDFS_NEW_SOURCE_FILE` 全部撤除。
3. **新增 `DetailAction.pack`**：声明「本动作的载荷是一个本地文件」（值 = 包后缀）。
   前端因此**不认识「导入」这个动作**——它只认「这个动作的载荷是文件」这个形状，
   与「导出」侧认 `filename` + `b64` 的形状（`actionFileOf`）恰好对称。
4. **动作的适用态由 `when` 表达**：导入声明 `when: {is_existing: false}`，只在
   草稿（新建）态出现——同一份详情定义服务两种态，不靠第二份定义。
5. **前端撤除「选入口」这一档**：`useVdfsPrompt` 整体退役——新建 = 直接进详情页；
   导入 = 详情页上的一条动作，取文件由渲染器的原生文件选择器完成。

**理由**

- 「导入」与「删除」同级：都是「对某个地址做一件事」，占的是详情页的一条动作。
  删除没有 `VdfsProvider::root_delete()`，导入也不该有 `root_new_type()`。
- **形状判定优于动作判定**：`pack`（取文件）与 `actionFileOf`（给文件）是同一类
  声明（「这一步需要原生能力」），前端只翻译形状、不解释动词 ⇒ 新增同类动作零
  前端改动，且前端不再持有任何后端不认识的动词（`protocol-mirror-audit` 的
  `LOCAL_ONLY` 因此清空）。
- 撤除 `root_new_type()` 之后，`VdfsProvider` 回到「只有一个方法」——
  模块文档那句从假话变回真话。

**后果**

- 后端：`VdfsNewType` 瘦身（≈250 → ≈150 字节，`VdfsNode` 里的 `Option<Box<_>>`
  仍是 8 字节）；agent / mcp / skill 三个 provider 的 `write_at` 二进制分支改为
  **显式拒绝并指向 `import` 动作**（目录型资源的存储层原语语义不变），各自新增
  导入分支与详情动作声明；`VdfsPack` 旁新增入向的 `VdfsUnpack`。
- 前端：`useVdfsPrompt.ts` 与其 spec 删除；`DetailForm` 新增 `pack` 动作的原生
  文件选择器（`File` 原样上抛，载荷编码归机制层）；`useVdfs` 的 `createTypedFile`
  → `runPackAction`；`newFileNameOf` 与 `writeVdfsBinary` 删除（目标名由 provider
  侧的 `pack_name_of` 推导，二进制写不再有对外入口）。
- 守卫：`protocol-mirror-audit` 的 D 组 19 → 18 对（`VdfsNewImport` 撤除）、
  `LOCAL_ONLY` 清空；`DetailAction` 的 `pack` 两侧同步。
- 基线：`rustTests` 926 → 928（新增解包往返与草稿态动作用例）、
  `vitestTests` 722 → 707——**净减是预期的**（删掉的是「选入口」状态机的用例），
  逐文件核对写在 `scripts/gate.d/_shared.mjs` 的对应注记里。

---

## ADR-030: 地址属于**条目**，不属于节点——`VdfsItem` 拆出，写回执只给**名字**

**状态**：已接受（2026-09-24 实施完成）。

**背景**

`VdfsNode` 里有一个不属于它自己的字段：`path`。

1. **地址是关系，不是属性**。它回答的是「这个节点在**某一份列表**里的位置」，
   而同一个节点可以在不同列表里以不同地址出现——实证：设置页的一项指向插件自己
   那份配置文档 `<目录名>/PLUGIN.yml`（落在**另一个挂载点**里），而同一份文档在
   自己的目录里就叫 `PLUGIN.yml`。把 `path` 挂在节点上，等于宣称「一个节点只有
   一个地址」。
2. **于是每个 provider 都得知道自己在树里的位置**。`composite` 容器、虚拟层
   `host`、三个存储实现（`dir` / `memory` / `single_file`）各自在拼
   `<父地址>/<name>`——同一条推导写了五遍；拼不出来的地方就留空，再靠访问层
   （`fill_paths`）补一次。**两处都在补**，口径各自维护，必然漂移。
3. **`VdfsWriteResponse.path` 更含混**。具名写时它回传的是调用方刚给的地址
   （把已知的东西还回去）；匿名写（打在目录自身上的那一次）时它是 provider 生成
   的名字**混在地址里**——前端 `sessions.ts` 因此要 `split('/').pop()` 取末段，
   还得额外挡「空路径 == 虚拟根」这个哨兵。

**决策**

1. **`VdfsNode` 去掉 `path`**——它是纯自述（`name` 已是父内的路径段）。
2. **新增 `VdfsItem { path, #[serde(flatten)] node }`**：`vdfs/list` / `vdfs/tree`
   的每一项都是它。**线格式与「带 `path` 的节点」逐字节相同**，故不改变任何既有
   消费者读到的 JSON。
3. **`path` 的回填只有一处**：provider 不填时，分发层按 `<父地址>/<name>` 推导
   （`plugins/vdfs/host.rs` 的 `item_addr`）；只有「地址不是那个形状」时由拥有者
   自己填（设置页条目是唯一实例）。
4. **`VdfsContent` 去掉 `path`**——内容总是「请求的那个节点」的内容，地址在请求
   里已经有了。
5. **`VdfsWriteResponse.path` → `name: Option<String>`**：**仅匿名写有**，值是
   [`VdfsNode::name`] 口径的路径段，不是地址——地址由调用方拿它和自己请求的那个
   目录拼（它本来就知道请求的是哪个目录）。

**理由**

- **一条推导只写一处**：`<父地址>/<name>` 现在只在分发层出现。provider 不需要
  知道自己在树里的位置——此前「留空 + 访问层再补」的两段式，本质是同一条规则
  的两个副本。
- **回执只给调用方不知道的那一个**：具名写回传地址是冗余；匿名写唯一的新信息是
  provider 生成的**名字**。给名字而非地址，拼地址那一步本来就该由调用方做。
- **线格式不变** ⇒ 这次拆分对任何既有消费者（含 CLI 与 e2e）都是**透明的**：
  `{path, name, title, …}` 仍是同一个对象，只是 `path` 的 owner 从节点挪到了条目。

**后果**

- 后端：`VdfsNode` 14 → 13 字段；新增 `VdfsItem`（`path` + flatten 的节点）；
  `VdfsContent` 7 → 6；`VdfsWriteResponse.path: String` → `name: Option<String>`；
  `composite` / `host` / 三个存储实现里的拼接逻辑收敛到分发层的 `item_addr`
  一处（`list` / `tree` / `search` 共用）。
- 前端：`VdfsItem extends VdfsNode`；`VdfsListResponse.items: VdfsItem[]`，而
  目录自身的 `node` 是纯 `VdfsNode`（它的地址就是响应里的 `path`）；
  `emptyNode()` 不再带 `path`；`sessions.ts` 的 `vdfsBase(resp.path)` →
  `resp.name`，「空地址 == 虚拟根」那道哨兵随之消失。
- 守卫：`protocol-mirror-audit` 新增 `VdfsItem ↔ VdfsItem` 一对（两侧都 flatten，
  故只比 `path`——节点字段由 `VdfsNode ↔ VdfsNode` 覆盖），D 组 18 → **19** 对。
- 基线：`rustTests` 928、`vitestTests` 706 均**不变**（改的是形状归属，不是用例
  数量）。

---

> **维护原则**：每个架构决策必须记录在此，包括背景、决策、理由、后果。

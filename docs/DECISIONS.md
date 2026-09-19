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

> **当前状态**：**已实现（现行）**，但资源与配置**不再走 `{plugin}/{action}`**——统一经 `vdfs/*` + `.vdfs/…` 地址（见 ADR-010 / ADR-011）。

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

## ADR-005: OAB 协议宿主实现

**状态**：已接受

> **当前状态**：**已演进**。Agent 插件仍是 Bundle 宿主、导入仍走 `.vdfs/agent` 新建类型 `zip`；
> 但 OAB v1 的**约定目录协议**（`prompts/` `skills/` `mcps/` 由宿主硬编码解释）已被
> [`agent-dir/v2`](./design/agent-directory-spec.md) 取代——**Agent 就是一棵插件树**：
> 技能 / MCP 复用宿主既有的 `skill` / `mcp` 插件目录，人格改为根 `AGENTS.md`。
> v1 规范见 [archive/open-agent-bundle-spec.md](./archive/open-agent-bundle-spec.md)。

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

> **当前状态**：**未落地 / 已回退**。CU（认知单元）+ `prop` 驱动的认知层已从代码中移除——无 `seed_cus.jsonl`、无认知单元解析；`ids.rs` 也已无本 ADR 的常量残留（原「Agent 能力 id」区的 `CAPABILITY_AGENT_COGNITION` / `_CHAT` / `_IDENTITY` / `_CREATE` 四个悬空常量于 2026-09-17 随 OAB v1 装配实现一并清理，见 CHANGELOG 同日）。现存的只有 `CapabilityCategory::Metacognition` 一个分类枚举值。

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

> **当前状态**：**已实现（现行）**。资源存储 = `providers/vdfs_service/` 的 `SingleFileVdfs` / `DirVdfs` / `MemoryVdfs` 三型，磁盘布局未变。

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
   会漂移（实测漂移 6 处，见 CHANGELOG 2026-09-16 第一批）；
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
- 骨架化摘要变长（48 → 64 token 上限，列表类多列 7 个条目名）：
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

> **当前状态**：**已实现（现行）**。会话域的全部实时显示经 `kind = "vdfs"` 一条频道，
> 按**地址**分派（`schemas/vdfs.ts::sessionRouteOf`）；`services/sessionBusWatcher.ts`
> 与 `eventBus` 的防乱序缓冲已删除。详见
> [`symbio/src/plugins/session/docs/node-state-streaming.md`](../symbio/src/plugins/session/docs/node-state-streaming.md)。

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

- 会话节点 `.vdfs/session/<sid>` 承载 `status`（`working` / `active` / `failed`）+
  `attributes.outcome`（`completed` / `aborted` / `failed`）+ `attributes.error`；
- 消息节点 `.vdfs/session/<sid>/消息/<mid>` 承载 `status`
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
- **`kind = "session"` 未整体删除**：进程内消费者（`agent/host/subagent.rs` 的审批透传、
  文本累积、以 `Status idle` 判定子会话结束）仍依赖它。前端不再订阅。
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

## ADR-017: `session` 的规模债**暂不拆分**，等 `agentbundle` 落地后按新边界动刀

**状态**：已接受（**带触发条件的延后**）

**背景**：
`session` 目前 **14340 行实现代码，占 `symbio/src` 生产代码的 29.3%**（源自 2026-09 的
度量，其余 15 个插件合计 24426 行）。它是 16 个插件里唯一"一个插件 ≈ 一整个应用"的
存在——会话编排、`chat_loop` 状态机、消息存储、工具执行与审批、VDFS 适配、心跳等
都在其中。2026-09 的后端质量评估把它列为与 `agentbundle` / OAB 方向最相关的结构债。

**决策**：
**现在不拆。** 拆分时机定在 `agentbundle`（新的标准访问层）落地并替代旧 `agent` 模块
之后，届时按 `agentbundle` 划出的新边界**一次性重组**，而不是先按现状切几刀。

**理由**：
- **边界还没定，先切就得重切**：`session` 现在的边界是历史形成的"编排唯一入口"
  （见 [ADR-003](#adr-003-session-作为编排入口)），而 `agentbundle` 正在把它重新定义为
  "访问层 + 资源拥有者"。按旧边界拆 = 付两次成本（拆一次、重组一次），且中间态没有收益。
- **规模本身不是本轮发现的问题的成因**：这一轮修掉的缺陷——路径穿越、shell 白名单旁路、
  会话丢更新、网关无上限、死代码——**没有一个**源于 `session` 体量大，全部已就地修复。
  规模是"可维护性利息"，不是当下的故障源。
- **`session` 的测试是全仓最厚的**（7044 行，且**全部**在独立 `*.test.rs` 里，内联为 0）。
  此刻大动会同时搬代码与搬测试，把"重构"和"回归"混在一次提交里，反而降低可验证性。

**触发条件（命中任一即应重启此决策，由新 ADR 取代本 ADR）**：
1. `agentbundle` 的边界与所有权（"一个 scope = 一个 owner"）在代码里稳定下来；
2. 出现"改 A 必须同时改 B"的跨文件连锁，而其根因是 `session` 内部职责混杂；
3. 单次改动的 diff 因 `session` 体量而无法被人完整审阅。

**后果**：
- 本阶段不引入新的内部模块边界，`session` 继续作为单一插件存在；
- 在此期间对 `session` 的改动坚持"**就地最小化**"，不做顺手的大搬迁；
- 本 ADR 被触发条件命中时，应被**新 ADR 取代**，而不是直接删除——延后是有理由的，
  理由失效这件事本身也值得记录。

---

> **维护原则**：每个架构决策必须记录在此，包括背景、决策、理由、后果。

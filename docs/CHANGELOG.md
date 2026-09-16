# 功能更新记录

> **文档类型：Reference（参考）** — 按时间倒序的功能/修复事实记录。
> 2026-04-04 及更早的条目已归档至 [archive/CHANGELOG-history.md](./archive/CHANGELOG-history.md)。

> **重要：当前形态声明（2026-07-06 起）**
>
> 本仓库当前形态是 **Rust 核心库 + Tauri (Vue 3) 桌面前端 + E2E CLI**。
>
> - `tauri/` 目录是活跃维护中的桌面前端（一等公民），与核心库版本同步演进。
>
> - `symbio/src/bin/seed_agents.rs` 提供批量灌入种子 Agent 的 CLI（当前唯一二进制入口）。
>
> - 早期"Tauri 时代 = 临时形态"的说法已被否决：前端自 2026-06 多会话体系改造后恢复为**一等公民**。
>
> 下方按日期倒序记录**与代码同步**的功能/修复条目。
> 下方按日期倒序记录**与代码同步**的功能/修复条目，前端 UI 变更同样记录于此。

***

## 2026-09-16: 「当前时间（Unix 毫秒）」收敛为 `symbio_core::clock::now_ms`

同一语义此前在**三个层各写了一份，共 7 处**：

| 位置 | 原写法 |
|---|---|
| `symbio_core/turn.rs`（2 处内联） | `time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000` |
| `plugins/session/heartbeat.rs`（`pub(crate) fn now_ms`） | 同上 |
| `plugins/session/plugin.rs`（私有 `fn now_ms`） | `SystemTime::now().duration_since(UNIX_EPOCH)…unwrap_or(0)` |
| `providers/vdfs_service/memory.rs`（私有 `fn now_ms`） | 同上 |
| `plugins/session/chat_session.rs`（`pub(crate) fn now_millis`） | `time::OffsetDateTime` 版 |
| `handlers.rs` / `orchestrator/entry.rs` / `resume.rs` / `types.rs` / `heartbeat_tool.rs` | 内联表达式 |

两种写法对本项目时间范围等价，但**分散在层间意味着改口径时必然漏改**（`plugin.rs`
与 `vdfs_service/memory.rs` 的 `SystemTime` 版本还带 `unwrap_or(0)` 兜底，会静默产生
`1970-01-01`）。现统一为 [`symbio_core::clock::now_ms`](../symbio/src/symbio_core/clock.rs)：

```rust
pub fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}
```

- 模块名取 `clock` 而非 `time`——后者会与外部 crate `time` 同名，使 crate 内
  `time::OffsetDateTime` 的解析产生歧义（同 `plugin.rs` 不能用 `mod vdfs` 的陷阱）。
- 共 **11 个文件、17 处调用点**改为引用同一实现；3 处本地定义 + 4 处内联表达式删除。
- **有意保留**（语义不同，非「当前时间」）：`chat_loop/compress.rs:411`（单位是**秒**）、
  `vdfs/host.rs` / `vdfs/physical.rs` / `vdfs_service/entry.rs` 的测试临时目录名
  （用 `as_nanos()` 求**唯一性**，不是时间戳）、`to_unix` 一族与 `workdir.rs:154`
  （转换**任意** `SystemTime`，不是「此刻」）。

***

## 2026-09-16: session 插件模块拆分（S1–S4）—— 消除 2300 行超长文件

`chat_loop.rs` 达 2401 行、`orchestrator.rs` 1513 行、`plugin.rs` 1480 行——单文件承载
3~6 类不相干职责，靠"文件"这一层已无法表达边界。分四步拆分，**全程零行为变更**
（只搬家：不改任何判定 / 阈值 / 文案 / 执行顺序）：

| 步 | 前 | 后 |
|---|---:|---|
| **S1** | 21 个文件带内联 `#[cfg(test)] mod tests`（合计 3713 行） | 测试外置 ⇒ 生产 **16647 → 12419** 行 |
| **S2** | `chat_loop.rs` 2401 | **434** + `chat_loop/{state,inputs,turn,compress,io}.rs` |
| **S3** | `plugin.rs` 1480 | **523** + `plugin/nodes.rs` 501 + `plugin/vdfs_provider.rs` 512 |
| **S4** | `orchestrator.rs` 1513 | **320** + `orchestrator/{broadcast,consume,entry,failure}.rs` |

**保真纪律**：每步做「归一化代码行多重集比对」——把旧文件与「新父文件 + 各子模块」
归一化（去 `//!` / `use` / `mod` / 空行，抹平 `pub(crate)` 等可见性前缀，还原
`super::super::`）后比较行多重集，**差异必须逐条归因**。S2 归因 13 条（6 处路径加深 +
2 处 rustfmt 折行 + 5 行 re-export 脚手架）、S3 归因 21 条、**S4 归因 0 条**
（双向差集均为空）。

**可见性口径**（S2 新确立，S3/S4 沿用）：

- 模块内共享面 = 子模块 `pub(crate)` 条目 + 父模块 `pub(crate) use`；
  ⚠️ `pub use` 要求条目本身是 `pub`，否则 `E0364`/`E0365`。
- 跨模块访问的**字段**必须逐个标 `pub(crate)`——Rust 的**字段可见性不随结构体**。
- 子模块统一 `use super::*;`：子模块可看到父模块的**私有**条目，故父模块的 `use`
  清单即子模块的共享导入面；**只被子模块使用**的导入**不会**触发 `unused_imports`。
- 被搬移代码里的 `super::X::` 需**加深一层**（S2 5 处 / S3 19 处 / S4 12 处）。

**踩过的坑**：① 子模块**不能与作用域内的 `use` 同名**——`plugin.rs` 已有
`use crate::symbio_core::vdfs;`，故子模块取名 `vdfs_provider` 而非 `vdfs`（否则 `E0255`，
且模块内 `vdfs::X` 会解析到自己）；② **自由函数不能落进 `impl` 块**——S4 的
`subtree_of` 若随 `persist_failure` 一起塞进 `impl SessionPlugin`，报
`E0425: cannot find function`；③ 段间的"分节注释"要并入**下一个**块，否则会留在空档里。

评审、目标结构与逐步实施记录见
[`symbio/src/plugins/session/docs/module-layout.md`](../symbio/src/plugins/session/docs/module-layout.md)。
门禁：`cargo test --lib` **544 passed / 0 failed**（基线未减）；clippy `--all-targets -- -D warnings` 零告警。

---

## 2026-09-16: 测试文件扁平化（`X.rs` + `X.test.rs`）与测试归位

**问题**：此前"实现与测试分文件"用的是 `<module>/tests.rs`，于是每个被测模块都多出一个
**只放一个测试文件**的目录——全仓 32 个（`workdir/`、`compression/`、`chat_session/`……）。
目录本身不携带信息，却把"模块"与"目录"两个概念混在了一起。

**改法**：测试文件与实现文件**同级**，同名加 `.test` 后缀，靠 `#[path]` 属性定位：

```rust
// workdir.rs 末尾
#[cfg(test)]
#[path = "workdir.test.rs"]
mod tests;
```

`#[path]` 相对**声明它的文件所在目录**解析，故测试文件与实现同级；**模块路径不变**
（仍是 `workdir::tests`），`use super::*;` 的语义与内联 `mod tests { … }` 逐字一致
——这是一次纯文件搬迁，不涉及任何可见性或导入面调整。

- 32 处扁平化，移除 30 个只为放测试而建的目录；`chat_loop/`、`plugin/` 两个目录**保留**
  （另有真实子模块，目录本身有存在理由）。
- **例外**：模块文件是 `mod.rs` 的（`store/mod.rs`、`plugins/agent/host/mod.rs`），
  测试放同级 `tests.rs`，已是正确形态，不动。
- 验收：逐文件**行多重集指纹**比对（改名前后逐字相同）+ 断言 32 个实现文件都带上
  `#[path]` + `cargo check --tests` 0 错。

**测试归位（高内聚）**：S1 曾把测试搬出实现文件，但 `plugin/tests.rs` 一个文件同时服务
`plugin.rs` / `plugin/nodes.rs` / `plugin/vdfs_provider.rs` 三个实现——**测试一处、被测试
代码一处**，违反高内聚。现按"一个实现文件对应一个测试文件"归位：

| 实现文件 | 测试文件 | 用例 |
|---|---|---:|
| `plugin.rs` | `plugin.test.rs` | 6 |
| `plugin/nodes.rs` | `plugin/nodes.test.rs` | 15 |
| `plugin/vdfs_provider.rs` | `plugin/vdfs_provider.test.rs` | 6 |
| `chat_loop.rs` | `chat_loop.test.rs`（`gate_turn` 契约） | 9 |
| `chat_loop/state.rs` | `chat_loop/state.test.rs`（`StopSignal` + `TurnRequest`） | 9 |
| `chat_loop/inputs.rs` | `chat_loop/inputs.test.rs`（`resolve_system_prompt`） | 6 |

`chat_loop/gate_tests.rs` / `chat_loop/stop_signal_tests.rs` 已并入上表并删除；
用例数**零丢失**（`chat_loop` 24、`plugin` 27）。

**顺带清理**：删除 `symbio/src/plugins/skill/plugin/tests.rs`——自初始提交起就**没被任何
`mod` 声明引用**的孤立文件，它引用的 `SkillPlugin::classify_skill_source` /
`get_skill_detail` 在当前代码里**已不存在**（"技能来源分类"功能整体下线），
故那 9 个用例从未运行、也不可能编译。删除后 `skill/plugin/` 目录随之消失。

**约定入文档**：测试文件布局写进 [`CONTRIBUTING.md`](../CONTRIBUTING.md) §4「测试文件布局」；
`session/docs/module-layout.md` 增补 §4.4（扁平化）/ §4.5（归位）两节实施记录。

---

## 2026-09-16: session 文档下沉插件目录（高内聚）+ 实现与测试分文件

**文档下沉**：session 相关的 9 份文档从系统级目录迁入 [`symbio/src/plugins/session/docs/`](../symbio/src/plugins/session/docs/)
（新建 [索引](../symbio/src/plugins/session/docs/README.md)），落实 `docs/README.md` 早已写明的
「单模块文档放在该模块目录内」原则——此前该原则只在 `turn-tool-mechanisms.md` 上兑现过。

| 原位置 | 新位置 |
|---|---|
| `docs/design/session-core-loop.md` | `session/docs/core-loop.md` |
| `docs/design/session-perf.md` | `session/docs/perf.md` |
| `docs/design/context-compression-design.md` | `session/docs/context-compression-design.md` |
| `docs/design/heartbeat-mechanism.md` | `session/docs/heartbeat-mechanism.md` |
| `docs/design/vdfs-session-messages.md` | `session/docs/vdfs-session-messages.md` |
| `docs/design/cascading-options-mechanism.md` | `session/docs/cascading-options-mechanism.md` |
| `docs/architecture/session-mechanism-audit.md` | `session/docs/mechanism-audit.md` |
| `docs/architecture/session-complexity-audit.md` | `session/docs/complexity-audit.md` |
| `docs/architecture/session-module-layout.md` | `session/docs/module-layout.md` |

去掉冗余 `session-` 前缀（目录已表达归属）。共改写 16 处文档内相对链接 + 14 处外部引用
（`docs/design/vdfs.md`、`README.md`、`cli/docs/usage.md`、`local/README.md`、
`session/plugin.rs`、`session/store/mod.rs`、`tauri/src/schemas/session_meta.ts`、
`tauri/src/services/sessionBusWatcher.ts`、本文件历史条目）。
路径书写约定：**同插件内用插件相对 `docs/x.md`，跨模块用仓库根相对**。

**审计工具扩范围**：`scripts/doc-link-audit.mjs` 原只扫 `docs/`，文档下沉后新位置将不受保护。
扫描根扩为 `docs` / `symbio/src` / `tauri` / `cli` / `examples` + 根目录 `*.md`
（跳过 `node_modules` / `target` / `dist` 等）。覆盖面 172 → 223 条链接。
顺带暴露并修复 `CONTRIBUTING.md` / `CODE_OF_CONDUCT.md` 的 7 条**既有**坏链
（根目录文件却写 `../` 前缀，等同跳出仓库）。

**实现与测试分文件**：session 插件 21 个文件的 `#[cfg(test)] mod tests { … }` 内联块
移出为同目录 `<module>/tests.rs`（沿用 `store/tests.rs` / `chat_session/tests.rs` 既有约定），
生产代码从 16,647 行降到 12,419 行（测试 4,313 行独立）。同时产出
[模块分工评审](../symbio/src/plugins/session/docs/module-layout.md)（`chat_loop.rs` 2400 行
的超长文件问题、目标结构、S1–S4 执行顺序）。

---

## 2026-09-15: 节点隐藏属性（`hidden`）与设置页的插件配置清单

改造三之后，左侧导航多出 `local` / `web` / `gateway` 三个入口——它们的全部内容只有
一份配置文档，没有可浏览的用户资源；而设置页却只剩「外观 / 关于」两项，各插件的配置
文档散落在各自目录里，用户在设置页看不到它们。两件事都由**机制**解决：没有引入新协议
字段，也没有引入「挂载」这个状态。

- **隐藏属性是机制级的节点属性**（`VdfsNode::hidden`）：与文件系统的隐藏属性同义，
  文件 / 目录通用。语义只有两条——**列表里不出现**（父目录 `list` 的结果不含它，
  `children` 计数同理），**可达性不受影响**（按路径 `stat` / `read` / `write` /
  子树操作一概照常）。因此它既不是权限（那是访问位），也不代表节点不在系统里。
  过滤由**产出列表的一方**执行：容器对自己合成的子目录清单、以及任何子 provider
  交回来的 `list` 结果做同一条过滤，标了 `hidden` 的节点不因来自哪个 provider 而异。
- **provider 的根也不过是一个目录节点**：所以「它显示还是隐藏」由
  `VdfsProvider::root_hidden()` 回答——与 `root_access` / `root_status` /
  `root_new_types` 同构，容器在合成该目录节点时回填进 `VdfsNode::hidden`。没有新的
  索引表，也没有「挂载清单」这类中间物。当前标为隐藏的是内容仅一份配置文档的
  `web` / `local` / `gateway`（各自的 `.vdfs/<插件>/PLUGIN.yml` 照常可寻址）。
- **第三条收集通道 `ConfigurableVisitor`**：与能力 / 选项并列，共用同一次 `traverse`
  广播，各有自己的 ctx 键与降级行为。插件在 `traverse` 里调
  `announce_configurable(&ctx, &self.config_file)` 声明「我有一份配置文档」；容器用
  一个**共享**收集器收下（不是像 VDFS provider 那样逐子插件一个——声明自带目录名，
  不存在归属歧义），并把结果**写回请求 ctx**，同一次请求里稍后被委派的 provider
  （即设置插件）据此读到清单：无需反查插件目录、无需硬编码插件表、也不需要协议上的
  新字段。
- **设置清单 = 各插件交出来的配置条目 + 自有分区**（配置在前——那才是用户要在设置页
  动手的东西；`appearance` / `about` 垫后）：条目由 `entry_of(&ConfigFile)`
  生成——名字用**目录名**（列表内唯一，也是前端查图标的键），地址用**真实地址**
  `<目录名>/PLUGIN.yml`，标题 / `ext` / 表单定义取自 `ConfigFile::node()`（定义因此
  只有一份来源）。设置页只是「指路」：点开读写的还是拥有者那份文件，不代理读写、也
  不复制配置。`kind` 留空，由消费方（设置插件）按自己所在的场景填 `setting`。
  图标不进协议——前端按 `kind:<目录名>` 查 `registry/vdfsIcons.ts`（已补
  `setting:telegram`）。
- **词汇口径**：VDFS 下一切都是目录和文件，「挂载」只是动词、不是状态值；节点也不
  需要 `link`——地址由节点自己的 `path` 表达。二者都不进协议。

***

## 2026-09-15: 插件配置回到插件目录（`PLUGIN.yml`），父插件不再代管配置

上一版把配置做成了「一个普通节点」，但**存储仍由父插件代管**：写配置者推切片、
`home` 合并落盘到 `<homedir>/config.yaml` 的 `symbio.plugins.<名>`。于是「一个插件的
配置」横跨三处（home 的合并规则、composite 的分发规则、插件自己的读取），而配置
**却不在插件自己的目录里**——插件目录因此不能整体拷贝移植。

- **一个插件 = 一个目录**：`<homedir>/plugins/<插件>/` 里既有配置（`PLUGIN.yml`）
  也有该插件自己的数据 / 资源，因此整个目录可直接拷贝移植。系统级插件（`home`
  与容器 `composite`）的目录是**系统根本身**，配置在 `<homedir>/PLUGIN.yml`——
  这同时消掉一个自举环：若 `home` 住在 `plugins/home`，容器扫描插件根时会把它当
  普通插件再构造一次，而那个 `home` 又会构造容器。
- **`PLUGIN.yml` 规范**：一个 YAML 映射，身份字段 `plugin_provider`（工厂 id）/
  `plugin_name`（实例名）**不参与配置反序列化**（`PluginDir` 读写时自动剥离 / 补回），
  其余键即插件配置。加载判据 = 文件可解析、且 `plugin_provider` 指向已注册的工厂。
- **容器改为「目录驱动」**：`composite` 扫描**自己目录下的 `plugins/`**（父插件经
  ctx 键 `PLUGIN_DIR` 告知它的目录），逐目录构造并挂载。它**不内置任何插件清单**
  （通用容器，可以嵌套另一个容器）——「必需插件」由构造者经 ctx 键
  `REQUIRED_PLUGINS` 传入（`home` 传的是 `SYSTEM_PLUGINS`），容器只负责把缺失的
  目录 / 配置文件补出来（只补身份字段，缺省字段由插件自己的 `Default` 兜底）。
- **配置归插件，写自己的文件**：`ConfigFile::apply` = 校验 → 落内存 → 落自己的文件
  → 广播。`save_config` 路由、`ConfigSlice` 载荷、`home::merge_slice`、
  `composite` 的向上转发、`ConfigDoc` / `SEG_CONFIG` / `is_config_path` 全部删除；
  `ConfigDoc` 由 `symbio_core::plugin_dir` 的 `PluginDir` + `ConfigFile` 取代。
- **地址从保留段变成真实文件名**：`.vdfs/<插件>/PLUGIN.yml`（原来是
  `.vdfs/<插件>/配置`）。`ext = form` 显式声明覆盖由文件名推导出的 `yml`——
  呈现方式由声明决定，不由文件名猜。网关只读白名单随之改判 `…/PLUGIN.yml`。
- **一次性迁移**：`home` 首次启动读旧 `config.yaml` 的 `symbio.plugins.*`，逐项写
  各插件目录的 `PLUGIN.yml`（目标已存在则跳过），随后把旧文件改名
  `config.yaml.migrated` 留档——天然只生效一次。旧形态里把资源明细混在配置中的
  插件自己消化：`model` 把遗留 `providers` 搬成 `provider.json` 后把配置**归一**为
  只有跨条目状态；`mcp` 搬完 `servers` 后用 `PluginDir::remove_keys` 把遗留键摘掉。
- **顺带**：`model` 的 `parent` 通道（仅用于推切片）随之成为死代码并删除；
  修掉两处既有 clippy 报错（`&mut Vec` → `&mut [_]`、`SettingPlugin::default()`
  → `SettingPlugin`），本地 `cargo clippy -- -D warnings` 恢复全绿。

***

## 2026-09-15: VDFS 机制细节收敛（重复挂载名、`kind` 口径、会话单条定位）

- **重复挂载名不再静默丢一份**：`CompositeVdfs` 收集子目录时改为**先排序、再去重**
  （键 `(order, 目录名, 插件名)`），重名 `warn` 并点名双方。此前 `list("")` 会列出
  两个同名子目录、而路径解析只命中一个——后来者完全不可达，且「谁胜出」取决于
  `HashMap` 的枚举顺序（同 `order` 时完全随机）。
- **删除冗余的 `entry::dir_node`**：它只是 `VdfsNode::dir(..., VdfsAccess::LIST)`，
  且把构造器已写好的 `kind` 再复写一遍。三处挂载根（`single_file` / `memory` /
  `dir` 的 `stat("")`）直接调用构造器。
- **文档**：`vdfs.md` §3.2 明确 `kind` 只有一个词表（`dir` / `file` 只是构造器给的
  **缺省场景标签**，可被插件名等覆盖），目录性永远只由 `l` 位表达；并列出全部机制
  字段名为**保留字**（`attributes` 会 flatten 到顶层，场景字段不得与之同名）。
- **会话单条定位不再全量读**：`SessionStore` 新增 `load_session_checked`（未命中给
  `None`），`session_of` 由「`list_sessions()` 全量读并解析后 `find`」改为按 id 直取
  ——`stat` / `read` 单个会话的开销不再随会话总数增长。`load_session` 改为
  `load_session_checked` + 缺省空会话，语义不变。

***

## 2026-09-15: 插件配置地址化，`CONFIG_GET`/`CONFIG_SET` 协议废弃

插件配置过去是一条私有路由（`<插件>/config/get|set`），并由 `setting` 插件**代理 +
硬编码映射** 4 个插件分区——于是同一份配置有两个地址（`.vdfs/setting/session` 与
`session/config/get`），`setting` 必须认识每个插件名，字段定义与校验也寄居在它那里。

- **配置 = 一个普通节点**：`<挂载根>/配置`（`ext = form`、`rw`、`schema` = 该插件
  自己的详情定义）。读写用的就是 `vdfs/read` / `vdfs/write`，与任何其它资源同一条
  链路——因此前端与 LLM 用同一种方式改配置。定义与校验**回归配置的拥有者**，
  默认值从各自的 `Default` 读出（不再有第二份 schema 字面量）。
- **落盘靠推送**：写配置者把自己的切片推给宿主（`save_config` 载荷
  `ConfigSlice { plugin, config }`），`home` 按 `plugin_provider` 定位既有条目后
  **逐键合并**再原子落盘，**不再反向拉取**任何插件的配置。切片里没有
  `plugin_provider` 的插件（model / mcp）其配置**就是**它们的资源树，不另设配置文档。
- **`setting` 瘦身为无状态 provider**（794 → 约 250 行）：只剩 `appearance` /
  `about` 两个前端自持分区。**「设置」页不再聚合插件配置**——会话 / 本地 / 网络 /
  开放接口的设置现在在各自的挂载点下（`.vdfs/<插件>/配置`），Telegram 首次获得
  可编辑的配置入口。
- **前端**：`DetailForm` 的 `config` 绑定与 `DetailDefinition.load_path/save_path`
  删除（绑定模式只剩 upload / info / option）。

***

## 2026-09-15: 会话存储由「trait + 三后端」收为一个具体类型

`plugins/session/store` 原本是 `SessionStore` trait + `FileSessionStore` /
`SqliteSessionStore` / `InMemorySessionStore`，由配置项 `store_kind` 经
`create_store` 选型。两个前提经取证证伪（`docs/architecture/session-*-audit.md`
早已判定 `store_kind` 为「可配置但不可用」的死机制）：sqlite 前端零引用、默认恒为
`file`、不支持子会话清单、且仍需一个磁盘目录放压缩存档；memory 表达的不是「另一种
存储」而是「要不要持久化」。

- **删除** `sqlite.rs`（199 行）与 `memory.rs`，`store` 收为单文件实现 +
  一个 `tests.rs`；`SessionStore` 由 trait 变为具体类型，落盘 / 不落盘是构造选型
  （`SessionStore::new(base)` / `SessionStore::ephemeral()`），`async_trait`、
  `create_store` 工厂、`Arc<dyn SessionStore>` 一并消失。
- **删除** `StoreKind` 枚举与 `SessionConfig::store_kind` 字段（与既有
  `storage_dir` / `session_id` 同一处置：旧配置残留键由 serde 静默忽略，无迁移）。
  `rusqlite` / `tokio-rusqlite` 依赖随之移除。
- **寻址接入宿主层**：`paths::safe_id` 不再自带一份规则，委托
  `providers::vdfs_service::entry::safe_segment`；`session_storage_dir()` 委托
  `entry::category_dir(PLUGIN_SESSION)`。会话目录名与 VDFS 资源条目目录名从此
  同一份规则（顺带把 `.` / `..` / 控制字符防护带进会话侧——旧实现只替换 `/ \ :`）。
- **刻意不改用 `vdfs_service` 三型**：`DirVdfs` 的「条目内部可下钻」会把
  `session.json` / `messages/` / `tool_archives/` / `transcripts/` 变成对外地址，
  而会话要求 `<id>` 是叶子、内部只以人读语义段呈现；消息内联在 `session.json` 里，
  条目不是文件字节。理由与规范 §13.4「目录自管的类型自己落盘」同一条判据。
- **新增测试**：截断 JSON 自愈、原子写不留 `.tmp`、恶意 id 不得逃出存储根、
  临时与落盘两种驻留方式契约逐条一致。

`cargo test --lib` 495 passed；`clippy --all-targets` 零告警；`fmt --check` 干净。

***

## 2026-09-15: 废除 storage_service，资源存储收敛为 VdfsProvider 的三个集中实现

**两套并行的资源访问抽象合并为一套**：上一条目（S16）删掉了「差异集中在一张 trait」的
适配层，但落盘那一层仍是与 VDFS 并行的私有抽象——磁盘资源用 `EntityStore` 的
`list_entities` / `read_entity` / `write_entity` 表达，再由每个插件手翻成
`VdfsNode` / `VdfsContent`。本次把这一层也换成讲 VDFS 的话的实现（决策见
[DECISIONS.md](./DECISIONS.md) ADR-011，机制定位见
[design/vdfs.md](./design/vdfs.md) §11 与 §13.4）。

- **删除**：`providers/storage_service`（`StorageService` / `EntityStore` /
  `EntityStoreError` / `FileEntityStore` / `path_resolver::safe_id`，含一份从未参与
  编译的孤儿文件 `entity_store.rs`）；`symbio_core/entities.rs` 的 13 个存储原语自由
  函数与 `EntityError`；`symbio_core/providers/storage.rs`（trait + `categories` /
  `manifests` 常量，类别段名从此就是插件名 `PLUGIN_*`，主文件名写在各插件内部的
  `const MANIFEST`）；`symbio_core/schemas/entities.rs` 里的 `EntitySummary` /
  `EntityUploadResponse` / `EntityExport` 与 `ENTITY_MODEL`…`ENTITY_SETTING` 常量
  ——该文件收敛为纯 `DetailDefinition` 表单方言模块。
- **新增** `providers/vdfs_service/`：三个**本身就是 `impl VdfsProvider`** 的集中实现
  ——`SingleFileVdfs`（一个条目 = 一份主文件，条目内部不外露；消费者 `model`）、
  `DirVdfs`（一个条目 = 一个目录，可下钻，主文件承载内容；消费者 `skill` / `mcp`）、
  `MemoryVdfs`（条目只在进程内；消费者 `model` 的 VDFS 清单镜像）。共享的寻址与落盘
  原语在 `entry.rs`（`category_dir` / `safe_segment` / `entry_dir` / `split_rel` /
  `id_of` / `pack_name_of` / `Entry` + 读写删），整包 zip / base64 与导出载荷
  `VdfsPack { id, filename, b64 }` 在 `pack.rs`。**不是新抽象**：没有 trait、没有
  注册表、没有适配器，差异（呈现、写前校验、写后内存同步）仍在各插件的 `impl` 里由
  调用点以普通参数传入；也**不走** `create_object` 工厂（不存在第二种实现）。
- **磁盘布局一个字没改**：仍是 `<homedir>/plugins/<category>/<id>/<manifest>`，三型
  只是三种访问拓扑，换拓扑不动数据。因此对外**零变化**——地址、`ext`、`schema`、
  访问位、导出包的线上字段全部原样。
- **事件通道收敛为一条 `kind = "vdfs"`**：`event_bus.rs` 的 `KIND_ENTITY` 与
  `publish_entity_changed` / `publish_entity_status` / `try_publish_*` 全部删除。
  生命周期与运行时状态变化一律经 `vdfs::host::notify_change` 广播，由 provider 的
  `watch` 经门面补成展示地址后以 `VdfsChangeEvent` 下发。
  **明确代价**：`notify_change` 只报 `(kind, path, change)` 三元组、不带载荷（不为此
  扩展 core 协议），故 `created` / `updated` / `deleted` 由消费者**防抖重拉**收敛；
  带载荷的增益投递只存在于 provider 自己实现的 `watch` 里。
- **协议层零改动**：`symbio_core/vdfs_provider.rs`（纯接口）与 `plugins/vdfs/*`
  （协议 / 访问层 / 物理层）本次未修改——`vdfs_service` 是对该接口的实现，不是扩展。
- **前端连带改动**：`services/eventBus.ts` 的 `KIND_ENTITY` 与实体生命周期 / 状态
  分支退场，清单与状态角标一律订阅 `kind = 'vdfs'`；资源协议面不变。
- **文档**：删除废止 stub `design/entity-provider-mechanism.md`（内容并入 `vdfs.md`
  §13.4 与 ADR-010/011）；`archive/` 两份实体机制档案补终局说明并修失效引用；
  `vdfs-frontend.md` 新增 S17；`PROTOCOLS.md` 删除「统一实体管理」整节并新增工厂
  适用边界；`ROUTES.md` 删除 `{plugin}/entities/*` 各节；`DATA_FLOW.md` 资源链路
  补「落盘在哪一层」一跳；`CONFIGURATION.md` 删除代码中并不存在的 `storage.backend`
  配置项、改为如实描述各类数据的落盘位置；`SYSTEM_MAP.md` 与 model / home / agent /
  setting / composite 五个插件 README 同步现状口径。

## 2026-09-15: 废除实体提供者机制（各插件直连 VDFS）+ 修复 SKILL.md 保存即损坏

**VDFS 收敛终局**：删除 `EntityProvider` trait（20 个钩子）、`provider_registry()` 注册表与
`EntityVdfsAdapter`（1504 行）。每个资源插件**直接实现 `VdfsProvider`**，用现有的
`list` / `stat` / `read` / `write` / `delete` / `action` / `watch` 表达自身语义；
`vdfs_provider.rs` 未做任何修改。跨插件共享的只剩 `symbio_core/entities.rs` 的
**存储原语自由函数**（写盘 / 删除 / 导入 / 导出，无 trait 约束）。

- **对外无变化**：挂载名（`.vdfs/model` / `.vdfs/skill` / `.vdfs/agent` 等）、节点形状、
  `ext` / `schema` / 访问位全部保持原样，前端零改动。
- **本次补上直连实现**：`model` / `skill` / `agent`（`session` / `setting` / `mcp` 此前已直连）。
- **实时能力不丢失**：适配器的变更广播提到 `vdfs/host.rs`（`notify_change` / `watch_changes` /
  `unwatch_changes`），**按 kind 全局持有**——同一 provider 每次 `traverse` 都会新构造，
  按实例持有会让订阅与投递配不上对。
- **修复：SKILL.md 保存即损坏**。表单保存链路把 Markdown 当 JSON 值写入，落盘内容变成
  `"---\nname: ...\n"`（外层引号 + 换行被转义成字面两字符），而读取端 `parse_skill_md`
  以 `strip_prefix("---\n")` 起手 ⇒ 必然失败：**保存报成功、文件却是坏的**，下次加载解析不出
  frontmatter，详情表单也读不到字段值。新增纯文本写盘原语 `entities::write_entity_text`
  （Markdown 主文件专用；`write_entity_manifest` 专用于 JSON 主文件），并以「写进去的必须能被
  读回来」回归测试钉住。zip 整包导入路径不受影响。
- **已删除的失效代码**：`EntityStatusResponse`、`ENTITY_STATUS_CONNECTED` /
  `ENTITY_STATUS_FAILED`（随 `test_status` 钩子一起失去用途）。
- **文档**：`design/entity-provider-mechanism.md` 归档（原路径留废止 stub 指向 `vdfs.md` §13.4）；
  `vdfs.md` §13.4 / `DECISIONS.md` ADR-010 / `vdfs-frontend.md` S16 同步；新增
  `scripts/doc-link-audit.mjs`（站内相对链接审计，默认只报告、`--strict` 才拦截）。
- **验收**：`cargo check --lib --tests` 0 error / 0 warning；`cargo clippy --workspace --all-targets`
  零告警；`cargo test --workspace` **464 passed / 0 failed**；`dead-code-audit` 0 文件 / 0 行。

## 2026-09-13: Session 插件机制收敛（配置契约单一真源 + 会话/骨架化实现去重 + 存储后端补齐）

对 session 插件做了一轮机制审计并据其落地（取证记录见 `symbio/src/plugins/session/docs/mechanism-audit.md`，
早期复杂度审计 `symbio/src/plugins/session/docs/complexity-audit.md` 已归档为历史版本）。以下为**对外可见**的行为/配置变更：

- **`max_tool_rounds` 默认值 `15` → `0`（`0 = 不限制`）**：此前配置面声明"默认 15"与 README 宣称的
  "实质无上限"互相矛盾，且 `chat_loop` 另持一份 `unwrap_or(15)` 兜底。现默认值、schema 描述、
  请求视图三者同源，契约翻译唯一入口 `SessionConfig::model_chat_max_tool_rounds()`（`0 → None`）。
  行为影响：未显式配置过的会话不再在第 15 轮工具调用处熔断。
- **`max_messages` 去掉 `.max(500)` 硬下限**：设置面板中小于 500 的值此前静默失效，现按用户设定生效；
  `0` 表示不限制（与其余窗口类配置语义一致）。
- **`context_messages = 0` 不再越界 panic**：`prune_historical_tool_calls` 对 `keep_turns = 0` 早返回。
- **`tool_context_window` 与 `context_messages` 解耦**：工具结果骨架化/保留窗口只由
  `tool_context_window` 控制，不再被 `context_messages` 连带关闭（此前两者门控纠缠导致"只调一个
  参数却同时改变两条链路"）。
- **新增 `prune_tool_history`（默认 `true`，保持原行为）**：置 `false` 时存储严格保留完整原文，
  工具链裁剪完全交给请求视图（不落库）。
- **轮次淘汰不再物理删除工具归档文件**：FIFO 淘汰与存储期 prune 均只删消息节点；
  `tool_archives/` 的磁盘生命周期唯一归 L0 守卫的 `TOOL_ARCHIVE_KEEP` 滚动策略（消除"配置说保留
  归档、实际文件已被删"的矛盾）。
- **`SessionConfig.session_id` 字段删除**：全仓零读取的死字段（会话身份由目录名/请求注入决定）。
  旧 `session_config.json` 中残留的该键被 serde 静默忽略，无需迁移。
- **fade（老旧工具结果淡化）阈值迁入配置**：`fade_activate_rounds`（默认 40）/
  `fade_keep_recent_turns`（默认 12）取代 `chat_loop` 内的硬编码常量。
- **`store_kind` 新增 `memory` 且 sqlite 后端纳入回归**：`store_kind` 此前"可配置但 sqlite 无测试、
  memory 未接线"；现三种后端共享同一份 `SessionStore` 契约测试。
- **`resume` 与 `message` 互斥**：`session/chat` 同时提供两者时显式报错，不再出现"user 消息被静默
  落库、但请求实际走 resume 分支"的半生效状态。
- **内部结构收敛**（无外部行为变化）：会话引擎实现 `impl ChatSession for` 由 4 降到 1
  （`EphemeralChatSession` / `FallbackChatSession` 删除，临时与降级会话改为
  `PersistentChatSession` + `InMemorySessionStore`）；头尾切分/截断机制唯一化到 `session::text_split`；
  `config_schema()` 从 142 行降到 83 行且不含任何 `default` 字面量；`orchestrator.rs` 三分支重复的
  会话派生字段收敛为 `req_base`；`workdir` 候选路径列表提取为单一常量。
- **验收**：`cargo check --workspace` 0 error / 0 warning；`cargo clippy --lib --tests -- -D warnings`
  零告警；`cargo test --lib` **337 passed / 0 failed**（基线 278 → 313 → 337，新增 59 个用例）。
  关键缺陷均做了变异验证（把修复回退后对应测试确实 FAILED），非恒真断言。

### 同轮收尾：两项"需用户决定"的遗留实施项（2026-09-13 授权实施）

审计中明确标注"超出本次授权范围、需单独决策"的两项，经授权后已实施：

- **`load_history` 序列化行为归一**（复杂度审计 §8.2-P1⑤ 原刻意保留项）：`model_chat::Request` 中
  它是唯一**缺少** `skip_serializing_if` 的 `Option` 字段——同结构体内自相矛盾：`None` 时其他可选
  字段消失、它却输出 `"load_history":null`。现补齐属性，使序列化**键集恒定**（`None`/`Some(true)`/
  `Some(false)` 三态均可无损往返）。语义零变化（`None` 与 `Some(true)` 本就同为"加载历史"）；
  该结构体为 session→model 的**纯进程内**契约，无 TS 对应文件、不落库、`cli/` 与 `tauri/src-tauri/`
  均不引用，故无跨语言兼容风险。补 3 个 serde 契约测试，并用穷尽结构体字面量（新增字段即编译失败）
  锁定契约；变异测试确认有拦截力。
- **批次 D：`run_chat_loop` 拆分**（机制审计 §4-批次 D 原暂缓项）。前置条件为"watchdog 与
  `stop_session` 竞态"，经取证确认为**真实缺陷**，先修复再拆分：
  - **竞态修复**：消费循环的提前出口（watchdog 超时、业务 Error 帧）会跳过 `ai_control_tx = None`
    清理，留下指向已关闭通道的**陈旧 sender**；而 `handle_abort` 恰以 `ai_control_tx.is_none()`
    作为子任务退出判据 → abort 必然空等 3s 兜底、Abort 帧投递到死通道被静默丢弃。新增
    `AiControlGuard`（`Drop` 守卫，与既有 `WorkingGuard` 同型）：登记时快照 request_id，注销时
    仅当仍是本轮登记才清除——**任何出口（含 panic）都不可能跳过清理**，且不会误伤下一轮的新登记。
    配套 3 个回归测试（Drop 注销 / disarm 不重复注销 / 陈旧守卫不误伤新登记），变异测试双向验证。
  - **拆分**：`run_chat_loop` 640 行 → **404 行**（纯骨架：装载上下文 → 消费流 → 收尾分派），
    提取 `close_turn`（241 行：截断续写 / 主动压缩拦截 / 工具分发 / 父节点状态落库 / 停等判定，
    以 `TurnFlow::{NextTurn,Finish}` 回传循环决策）与 `run_context_compact`（压缩执行）。
    **拆函数不拆行为**：搬移段与拆分前逐行比对，241 行区间仅 11 处差异，全部为机械改写
    （借用形式 `&mut out` / `&channel`、出口 `continue`→`NextTurn`、`return Ok(())`→`Finish`），
    三条出口路径与拆分前逐一对应；`tool_rounds` 改传 `&mut`（否则计数不推进，已在编译期暴露并修正）。
    文档强调的不变式"终态唯一落库点在 orchestrator"未被搅浑——`finalize_assistant_turn` 调用点
    数量与位置与拆分前一致。
- **本轮验收**：`cargo test --lib` **343 passed / 0 failed**（337 基线 + 3 serde 契约 + 3 守卫回归）；
  `cargo clippy --lib --tests -- -D warnings` 零告警；`cli`、`tauri/src-tauri` 两 crate `cargo check` 通过。

***

## 2026-09-11: ModelProvider 纯 trait 化（ModelProtocol 完全内化进 model 插件 + session 压缩路径走 execute_turn）

- **核心 `ModelProvider` 重写为纯 object-safe trait**（`symbio_core/model_provider.rs`）：方法集 `provider_id()` / `api_protocol()` / `rate_limit_ms()` / `max_context_tokens()` / `effective_context_tokens()`（async，= min(用户设置, 服务探测)）/ `execute_turn()`（async，五态错误映射内聚于实现方）。Session 的模型契约收敛为 `Arc<dyn ModelProvider>` 单一形态；`FinishReason`/`Usage`/`ProtocolEvent` 保留 core，`TurnOutput`/`PluginChannel`/`PluginError` 等既有类型不动。
- **`ModelProtocol` 完全内化进 model 插件**：trait（钩子 get_api_url/get_headers/prepare_request/parse_response_line/ping/query_context_limit，全部收 `&ModelProviderConfig`）、`resolve_protocol_id` 别名表、`MODEL_PROTOCOL_*` 注册常量、`ReasoningConfig`（serde 形态冻结）全部迁入 `plugins/model/`；`symbio_core` 不再导出任何协议概念，`ids.rs` 四常量删除。协议实现文件为纯机械替换（签名 `&ModelProvider` → `&ModelProviderConfig`，参数名 provider → cfg；已验证协议体仅使用 config 同名字段）。
- **新增 `bound_provider.rs`**：`BoundProvider(cfg: ModelProviderConfig, protocol: Arc<dyn ModelProtocol>)` 实现 core trait——身份/限流/上下文参数读 cfg，`execute_turn` 五态机（Aborted/RetryWithoutContextId/Err/RateLimited/Ok→parse_sse_stream）与 `effective_context_tokens`（min(用户设置, query_context_limit 探测)）自旧 core 结构体固有方法原样迁入，语义不变。`model_providers.rs` 删除 `into_model_provider` 构造器，保留为纯持久化 schema。
- **`parse_sse_stream` 闭包化**（`symbio_core/turn.rs`）：泛型 `<P: ModelProtocol + ?Sized>` 参数改为 `parse_line: impl Fn(&str) -> Vec<ProtocolEvent>` 行解析闭包，core 转录机器不再依赖任何协议抽象；>256 字节部分行分支（`try_parse_partial_sse_line`）与 LineProgress 去重逻辑不动。
- **session 压缩路径收敛**：`run_compression_llm` 原手动拼装（prepare_request + execute_post_with_abort + PostResult 五态匹配 + parse_sse_stream）整体替换为单次 `provider.execute_turn(system_prompt, messages, &[], root_id, muted, abort_flag)`；`ChatOrchestrator.provider` 与 CapabilityVisitor 注册槽位改 `Arc<dyn ModelProvider>`；orchestrator 的 provider_id/rate_limit_ms/max_context_tokens 字段访问改方法调用。
- **验收**：cargo check 零警告、cargo test --lib 286 项全绿、clippy 零错误零警告（tools.rs 测试桩同步重写为实现新 trait 的 MockProvider）。

## 2026-09-11: ModelProvider 类型体系统一（合并 Entry/Config 为单一核心定义 + 按上下文单注册 + CapabilityVisitor 简化）

- **核心 `ModelProvider` 由 trait 改为具体结构体**（`symbio_core/model_provider.rs`）：自含身份（provider_id/protocol_id/system_prompt/rate_limit_ms）+ 全量模型参数（model/api_base/api_key/temperature/max_tokens/max_context_tokens/reserved_tokens/timeout_secs/api_protocol/store/reasoning，自 `ModelConfig` 迁入）+ `protocol: Arc<dyn ModelProtocol>` 协议适配器。原 `description` 字段删除（全仓无消费者）。
- **原协议 trait 更名 `ModelProtocol`**：纯钩子集（get_api_url/get_headers/prepare_request/parse_response_line/ping/query_context_limit），全部改收 `&ModelProvider`；`execute_turn` 从 trait 移除，改为 `ModelProvider` 固有方法（prepare_request → execute_post_with_abort → parse_sse_stream），另提供固有委托方法与计算参数 `effective_context_tokens()`（= min(max_context_tokens, query_context_limit)）。
- **删除 `symbio_core::schemas::model::model_config`**：`ModelConfig` 全部字段并入 `ModelProvider`；`ReasoningConfig` 迁入 `model_provider.rs`；`schemas/model/` 模块整体移除。默认 `max_context_tokens` 统一为 262_144（修复原 model_config 307_200 与 ModelProviderConfig 262_144 的分叉）。
- **model 插件按上下文注册唯一生效 Provider**：traverse 从"注册全部 enabled 条目"改为"解析链 ctx[PROVIDER_ID] > default_provider_id > 首个 enabled → 仅注册该 Provider"（含其系统提示词双键注册：id 键 + "default" 键）；`model_providers.rs` 保留为持久化 serde schema（JSON 兼容），`to_model_config` 替换为 `into_model_provider(protocol_id, protocol)` 构造器。
- **CapabilityVisitor 简化**：删除 `ModelProviderEntry` 与 `list_model_providers`；`register_model_provider(Arc<ModelProvider>)`（覆盖语义）+ `get_model_provider() -> Option<Arc<ModelProvider>>`（无 id 参数）；`DefaultToolVisitor` providers 改单槽。
- **session 消费收敛**：`run_chat_loop_task` 解析链（get_model_provider(id) → find is_default → first listed）收敛为单次 `get_model_provider()`；`ChatOrchestrator` 改持 `ModelProvider` + 构造时预计算 context_limit，6 处消费点切换（api_protocol 日志、nudge 阈值、execute_turn、自动压缩触发、压缩溢出守卫、压缩请求）。
- **顺手修复**（非重构引入、新工具链 lint）：gateway/plugin.rs 测试 `create_config` 与 schemas/options.rs 两处 struct/enum literal 省略字段警告、homedir.rs 文档注释 `>` 行首误读为 markdown 引用。
- **验收**：cargo check 零警告、cargo test --lib 286 项全绿（基线 228 → 新增 58，含重写的 tools.rs 单槽覆盖测试）、clippy 零错误零警告。

## 2026-09-11: 会话心跳任务（heartbeat 工具 + CLI 守护模式 + homedir 环境变量优先级修复 + 空闲基线语义修复）

- **会话心跳调度器**（`plugins/session/heartbeat.rs`）：每进程每 15s 扫描本 store 全部会话，对「已启用 + 空闲满阈值（interval + 每会话固定 jitter ≤30s）」的会话注入 `hb_<sid>_<毫秒>` 心跳消息（`meta.heartbeat=true`，提示词作为用户消息进入正常回合管线）。`is_working` 会话绝不触发（防重入）；触发即写内存锚点（防热循环）；锚点首见回退 `session.updated_at`（进程重启/多进程兜底 = 重启追赶）。
- **heartbeat 设置工具**（`plugins/session/heartbeat_tool.rs`）：agent 可调用 `heartbeat` 工具对本会话 `set`（interval ≥10s / prompt / include_history，部分更新）/ `get` / `cancel`（停用保留配置）；配置持久化于 `session.json` 的 `metadata.heartbeat`，写回刷新 `updated_at`。
- **CLI 心跳守护模式**（`cli/`：args/client/main）：`symbio-cli --heartbeat` 驻留进程，为本 homedir 下所有启用心跳的会话触发空闲心跳并渲染状态；与 `-m`/`--repl` 互斥，模式判定顺序 `--heartbeat` 优先。
- **homedir 优先级修复**（`symbio_core/homedir.rs`）：`HomedirRegistry` 优先级改为 **`SYMBIO_HOMEDIR` 环境变量 > bootstrap 文件 > 默认 `<cwd>/.symbio`**（修复前 bootstrap 优先，`--homedir` 被静默覆盖）；新增回归测试 `test_env_var_overrides_bootstrap`。
- **空闲基线语义修复**（`plugins/session/heartbeat.rs`）：空闲基线改为 `max(内存锚点, 磁盘 updated_at)` —— 修复前空闲时钟从「上次触发/上次消息接收」起算，回合结束后 14.1s 即重触发（违反「无活动之后满 interval」契约）；修复后空闲严格从活动结束（最后一次落盘）起算，E2E 43 次触发最小间隔 36.2s 全部合规。新增测试 `idle_baseline_prefers_latest_activity`（全套 286 项测试通过）。
- **文档**：新增 `symbio/src/plugins/session/docs/heartbeat-mechanism.md`（语义契约/调度细节/工具 API/E2E 摘要）；`cli/docs/usage.md` 补守护模式、`--heartbeat` 参数与 homedir 优先级链。

## 2026-09-09: L1 消息压缩豁免与批次保护（ToolCall 参数永久豁免 + 最近 N 条原文保护 + 头尾保留 + 删除死代码路由）

- **删除死代码路由 `session/compress`**：`invoke_compress`（handlers.rs）全仓无任何调用方（tauri 前端、examples、bin 均未引用），且与自动压缩路径保护语义不一致（无"保护最新一条"切分、无角色豁免，误用反而会压缩当前任务指令）。随路由一并删除：`SessionCompressRequest` schema（schemas/session/session_compress.rs）与 mod 声明、plugin.rs 路由分发、ROUTES.md 条目；`ChatSession::compress_messages` 默认实现的 doc 同步。压缩统一走自动路径（`compress_temporary_messages` → 批次覆写）。

- **L1 批次压缩增加"最近 N 条内容节点"原文保护**：`PersistentChatSession::compress_messages` 新增 `keep_recent` 语义（`SessionConfig::compress_keep_recent`，默认 3，serde 默认兼容旧配置）——从尾部倒数最近 N 个 Text/Reasoning 内容节点跳过压缩（ToolCall/ToolResult 不占名额），且最后一条消息永不压缩（与自动压缩 `messages[..len-1]` 保护语义对齐）。自动路径 `compress_temporary_messages` 无需改动即同等受益（保护在批次覆写内部读取配置）。

- **L1 单消息压缩 ToolCall 参数永久豁免**：`compress_message` 对 `msg_type == ToolCall` 直接返回 None 不压缩不写存档——工具调用参数被骨架化后，模型在请求视图里看到"自己上次执行了一个参数为存档占位符的 edit"，会误记自身行为。新增测试 `toolcall_args_never_compressed`。

- **L1 保留策略从"仅尾部"改为"头尾保留"**：对齐 L0 `split_head_tail` 策略——保留头部 1/4 行 + 其余尾部行（首行常含结论/路径/计划骨架，仅留尾部会挤出关键头部），单行超长退化按字符截断行首；压缩头文案同步为「保留开头 N 行与结尾 M 行内容」。新增测试 `long_line_under_token_cap_not_compressed`，`normal_messages_unaffected` 断言同步头尾格式。

- **配置新增**：`SessionConfig::compress_keep_recent: usize`（默认 3），`session/config/schema` 自动透出。

## 2026-09-09: 上下文压缩体系 P1-P2（存档迁移 + 取回协议统一 + JSON 语义摘要 + 工具输出瘦身 + 快照版本指纹）

- **P1-1 L0 工具结果存档迁移至会话目录**：`guard_tool_result` 的全文存档从系统临时目录迁至 `<homedir>/plugins/session/<safe_id>/tool_archives/`（`safe_id` 将 `/\:` 替换为 `_`；session_id 缺失或目录创建失败时回退临时目录）——工具结果语义上是会话资产，历史写入临时目录会被 OS 清理造成死链。文件名改为 `tool_{毫秒}_{token数}_{内容FNV指纹}.txt`（FNV-1a 64 取高 32 位 hex），杜绝旧实现"同秒同 token 数互相覆盖"的碰撞；每次写入 best-effort 清理旧档，按修改时间保留最新 `TOOL_ARCHIVE_KEEP=20` 个文件。`guard_tool_result` 签名增加 `session_id: Option<&str>`，调用点（tool_executor）从 ctx 的 `SESSION_ID` 取值传入；新增 `archive_into_dir`（目录注入，供测试）与 `resolve_archive_dir`。新增测试 `same_milli_same_tokens_do_not_collide` / `prune_keeps_only_latest_files` / `archive_prefers_session_dir_when_session_id_given`。

- **P1-2 三层压缩取回协议统一**：L0 占位与 L1 压缩头统一追加取回提示「取回：local/file_read 该路径，按 offset/limit 分段读取」——此前各层只给存档路径不给取回方法，模型需自行猜测。L3 fade 按设计不写存档（`archive_path` 为 none）不变；L0（`[... ...]`）与 L1（`<!-- -->`）前缀标识保持各异，供 `decompress_message` 识别防重复压缩。

- **P2-1 骨架化摘要 JSON 语义感知**：`context_window.rs` 的 `first_line_digest` 在首行以 `{`/`[` 开头时改走 `json_digest`：解析 JSON 后提取 count/total/total_count/size 等计数字段与 entries/results/items/data/files 首数组首项的关键字段（name/path/file/title/id/type/status），输出 `count=16,name=main.rs,...` 形式替代盲切片——长 JSON 结果被骨架化后仍保留行数与内容类型等语义线索。数组不占计数；数组兜底 `items=元素类型列表`、对象无计数字段兜底 `keys=键名列表`；整体仍受 `SKELETON_DIGEST_TOKEN_CAP=48` 截断。新增测试 `json_result_gets_semantic_digest`。

- **P2-2 目录列举结果瘦身**：`dir_list` 的 entries 从 `{name,type,size,modified}` 精简为 `{name,type}`——size/modified 对模型定位目录结构无增益且逐条挤占 token；目录优先排序与 MAX_ENTRIES 上限不变。glob（file_search）结果本就是纯相对路径数组，无需改动。

- **P2-3 快照协议版本与提示词指纹**：新增 `COMPRESSION_PROTOCOL_VERSION="v2"` 常量与 `compression_prompt_fingerprint()`（对 `get_compression_prompt()` 全文取 FNV-1a 64 高 32 位 hex）；被动压缩与主动 `run_context_compact` 两处快照 meta 注入 `protocol_version` / `prompt_fingerprint` 字段——离线审计快照时可确认由哪版协议与提示词产出，提示词后续演化不再造成快照溯源歧义。meta 增字段安全（`should_start_compression` 只读 post_tokens 做 ×1.15 迟滞）。

- **测试口径**：`cargo test --lib` **246 passed / 0 failed**（本轮新增 5：guard 3 + context_window 2）；`cargo clippy --all-targets` 0 警告 0 错误。

***



- **骨架化参数定位锚点（链路保持）**：窗口外 ToolCall 骨架化时参数占位符保留关键定位参数回声（`path`/`command`/`url`/`pattern`/`file_paths`/`query`，单条约 10 Token，新增 `anchor_of_args`）——否则"读过某文件第 N 行"这类结果摘要因缺失文件路径而无法回溯，历史逻辑链路断裂。新增测试 `skeletonized_call_and_result_keep_anchor_param` / `skeletonized_without_anchor_falls_back_to_generic`。

- **单行长内容压缩失效修复（bug）**：单行大 JSON/URL/base64 原先绕过两层防线——L0 守卫 `split_head_tail` 首行永远整行保留（超预算不生效）；L1 脱水仅按行数触发（行数=1 不触发）。修复：L0 head/tail 超预算时按字符截断兜底；L1 触发条件增加"单行超长 token 超预算"（`message_archive.rs`，保留内容再按字符截断）。新增测试 `single_long_line_message_is_token_capped` / `single_long_line_is_char_truncated`。

- **策略保留优先级修复（bug）**：`context_window.rs` 的 `is_stale` 原逻辑全局窗口判定优先于工具级保留策略，导致 LastOnly 工具（todo_write）的最新调用滚出全局窗口（15 个 ToolCall）后被骨架化，模型丢失任务清单引发重写。修复后**策略保留优先于全局窗口**：声明 `LastOnly`/`LastN` 的工具其最近 N 次调用即使滚出全局窗口也完整保留；未声明策略的工具仅受全局窗口约束。新增测试 `last_only_latest_call_survives_beyond_global_window`。

- **骨架化"整条丢弃"改为"保留一行摘要"**：新增 `SKELETON_DIGEST_TOKEN_CAP=48` 与辅助函数 `is_failed_result`（结构化优先：`meta.success` 存在即直接采信短路返回，避免"0 failed tests"文本误判）、`error_digest`（failure_kind + 工具名 + 首行原因）、`first_line_digest`、`truncate_tokens`（CJK 友好，字符预算 = token×2）。占位符：失败 → `[System Info: Tool result failed: {kind} ({tool}): {cause}. Full output skeletonized.]`；成功 → `[System Info: Tool result received successfully. Output skeletonized. Summary: {首行}]`。新增测试 `skeletonized_failure_keeps_error_digest` / `skeletonized_success_keeps_first_line_digest` / `failure_detection_prefers_structured_meta`。

- **todo_write 结果瘦身**：返回值从 `{success, count, todos(全量), markdown(全量渲染), message}` 改为 `{success, count, message}`——全量清单对当轮是重复（输入参数刚写过），历史由 LastOnly 策略保证最新一次完整保留。确认 tauri 前端无专用渲染依赖，瘦身安全。

- **压缩提示词强化**：`compression.rs` `get_compression_prompt()` 追加高信噪比规则：跨区块去重（同一事实只出现一次）、只留结论丢过程度量（行数/字节数/读取范围/报错转储）、错误只留"结论+原因"一行、可低成本核实的疑问先核实再入 open_questions、有 todo 清单时 in_progress 引用不复述。

- **Clippy 清零**：修复 rust 1.93 新 lint 全部 22 个警告（needless_borrow ×12 / doc_lazy_continuation ×3 / empty_line_after_doc_comments / bool_assert_comparison / field_reassign_with_default / needless_range_loop / question_mark / single_match / too_many_arguments 加 `#[allow]`），新代码零警告，`cargo clippy --all-targets` 0 警告。

- **测试口径**：`cargo test --lib` 实际执行 **236 passed / 0 failed**（此前 README 的 239 为 ripgrep 统计误差，以 cargo 执行为准）。

***

## 2026-09-07: 文档体系重构（文档下沉原则落地）

- **确立"文档下沉"原则**：单模块文档放模块目录内（`README.md` + 可选 `docs/`），系统级文档只保留跨模块核心逻辑并引用模块文档；每篇职责一句话见 [README.md](./README.md) 的"模块文档地图"。

- **模块文档全覆盖**：14 个插件 `symbio/src/plugins/*/README.md` 全部就位（新增 agent / local / web / gateway / home / composite / setting / hook / event_bus / skill 十篇；重写 model——Phase E 后 model 为无状态单轮 LLM 网关 `execute_turn`，旧"工具执行/审批流分发器"描述作废）；前端新增 [tauri/README.md](../tauri/README.md) + [tauri/docs/FRONTEND.md](../tauri/docs/FRONTEND.md)。

- **历史实施日志归档**：`symbio/docs/model-session-refactor.md`、`turn-tool-mechanisms.md` 原文移入 [archive/implementation-logs/](./archive/implementation-logs/)（加状态横幅）；机制现行版精简下沉为 [session/docs/turn-tool-mechanisms.md](../symbio/src/plugins/session/docs/turn-tool-mechanisms.md)；`symbio/docs/` 目录清空。

- **去重**：[session/docs/context-compression-design.md](../symbio/src/plugins/session/docs/context-compression-design.md) 瘦身为 L0-L6 分层总览 + 取舍原则 + 不变量，各层阈值与实现细节归 [session/README.md](../symbio/src/plugins/session/README.md)；[OVERVIEW.md](./architecture/OVERVIEW.md) 删除 agent 模块内部细节章节，"插件不各自维护文档"的旧约定改写为下沉原则。

- **口径修正**：单元测试数以实际统计为准修正为 **239**（原 README 355 / 重构日志 227 均不准）；README/SYSTEM_MAP 同步 model 与 session 职责新表述。

***

- **只读模式不再放行网关自身配置**：`is_readonly_allowed` 从未匹配 `gateway/config/get`
  （白名单只有精确 `config/get` 与前缀 `config/get*`），而 `config.rs` 的单测却断言它放行，
  该测试一直失败。按"网关配置含 `inbound_token`，只读模式下放行等于把令牌读走"的判定，
  确认**拒绝**为正确语义：测试改为断言拒绝，并在函数文档与
  [CONFIGURATION.md](./reference/CONFIGURATION.md) 写明理由。
  **验证**：`cargo test --lib gateway::config` 3 passed / 0 failed。

- **新增设计稿** [session/docs/context-compression-design.md](../symbio/src/plugins/session/docs/context-compression-design.md)：
  针对"长对话上下文超限导致会话中断"，盘点现有四层压缩（轮次窗口 / 工具结果窗口 /
  单条消息存档 / 自动摘要），定位 7 条根因（其中 `finish_reason` 全链路未解析、
  token 估算误用 UTF-8 字节数、`max_tokens` 默认值与模型能力脱钩为 P0），
  给出"四层 token 预算模型"与 P0~P3 落地计划。

- **P0 已实现**（同日，设计稿状态同步更新）：
  - `symbio_core/tokenizer.rs`：纯 Rust 启发式 tokenizer（按字符类别加权、`chars()` 口径
    修复中文低估 2 倍的字节估算 bug）+ `CalibratedTokenizer` 用 provider `usage` 滑动校正
    （反馈必须用原始估算 `count_raw`，否则自反馈收敛到 √(真实比值)）。
  - `ProtocolEvent` 新增 `Finish` / `Usage`；**四个协议全部解析**（openai_chat /
    openai_responses / anthropic / gemini），`TurnOutput` 贯通。
  - `chat_loop`：主循环改 `loop` **移除 `max_tool_rounds` 硬性上限**（默认无限轮次，
    仅显式配置时作软上限并明确提示）；`finish=Length` 纯文本截断→自动续写（≤3 次），
    工具参数截断→明确报错；截断 Turn 落 `meta.finish_reason="length"`。
  - L0 `tool_result_guard.rs`：超 8192 token 的工具结果存档 + head/tail 摘要；
    `fade_aged_tool_results` 老化淡化：超 40 轮后仅压缩早于最近 12 个 user turn 的
    工具结果（可经存档取回），**绝不触碰 assistant 文本/推理**（保护思维链）。
  **验证**：`cargo check --lib` 0 警告 0 错误；tokenizer / tool_result_guard 单测 7 passed。

## 2026-09-04: 统一资源管理器动态 provider 注册——六类资源 + 导航/设置全部注册驱动

在 `ResourceProvider` trait 收敛之后，把"哪几种资源、怎么导航、怎么展示"彻底交给注册表：

- **注册表扩展为六类**：`provider_registry()` 新增 `setting`（`supports_upload=false`、
  `compact_list=true`、`nav=settings`）。`ResourceProviderInfo` 与下发 `ProviderInfo` 新增
  `compact_list`（列表简洁模式）与 `nav`（左侧主导航归属 `resources`/`settings`，空串不进导航）。

- **左侧导航全部动态驱动**：`MainLayout` 资源区 `v-for="p in resourceNav"`、设置区
  `v-for="p in settingsNav"`——"设置"入口由注册表动态生成，删掉手写按钮；session 不进导航
  （走独立会话主入口）。新增 provider 只需在后端登记一条 `nav`，导航自动出现。

- **设置页 = 统一资源实例**：删除 `components/SettingsPage.vue`，`/settings` 复用
  `ResourceManagerView`（types='setting'）。5 个分区（appearance/session/local/web/about）经
  setting 插件 `resources/list` 下发，前端按 `kind:ext` 复合键注册各自 editor（
  `tauri/src/components/settings/`），未命中回退 kind 级 / 通用兜底——与文件系统"扩展名决定编辑器"同构。

- **列表尊重服务器返回顺序**：`buildMixedItems` 删除前端 name 排序，改为按 activeTypes 顺序 +
  各类型 `resources/list` 原序（设置分区严格按后端声明顺序展示，不再被中文名重排）。

- **editor 契约清理**：5 个设置表单 `defineOptions({ inheritAttrs:false })` 阻断未消费 props
  落根 DOM；动态 `<component>` 加 `:key` 避免切换资源/类型时表单状态残留。

- **死代码清理**：删除 `schemas/resources.ts` 无引用的 `ResourceType` / `ResourceUploadRequest` /
  `ResourceDeleteRequest` 导出（请求体统一走对象字面量）。

- **验证**：cargo clippy 零告警、363 测试通过；vue-tsc 零错误、vitest 61/61、vite build 成功。

## 2026-09-03: 资源管理页体验修复——表单注册表、路由刷新、主题令牌化

修复统一资源协议落地后的四个回归问题：

- **Model 表单退步修复（详情差异化机制）**：明确"列表统一、详情差异化"架构——
  `ResourceManagerView` 新增 `FORM_COMPONENTS` 注册表按类型注入专属表单组件，未注册类型走
  通用兜底（zip 面板 / JSON 编辑器 / 只读详情）。恢复 `constants/modelProviders.ts`，新建
  `ModelProviderForm.vue`（恢复提供商预设、模型候选、API Key 显隐、启用开关、高级设置折叠、
  校验连接、跳过校验保存、设为默认等全部旧表单能力，样式迁移到新设计令牌）。

- **后端支持表单控制标记**：model 插件 `validate_manifest` 支持 manifest 携带
  `skip_validation`（跳过连接校验，不落盘）与 `is_default`（设为默认，随 manifest 落盘）；
  `on_uploaded` / `load_from_storage` 读取 `is_default` 标记（显式标记 > 现有指向 > 首个可用），
  "设为默认"重启后不再丢失。

- **页面切换列表不刷新修复**：四个资源路由共用 `ResourceManagerView`，Vue Router 复用组件
  实例导致 `onMounted`/事件订阅不再执行、列表残留上一类型数据。`MainLayout` 的 `RouterView`
  加 `:key="route.path"` 强制重建。

- **布局重构**：zip 上传创建面板改为居中卡片式；mcp/skill/agent 详情区新增统一工具栏
  （测试连接/删除按钮同一行右对齐），替代原先散乱堆叠的操作行。

- **主题令牌化**：`ResourceManagerView` / `ResourceShell` / `ResourceDetailPanel` /
  `ModelProviderForm` 全部样式迁移到 `--surface-* / --text-* / --border-* / --accent / 语义色`
  设计令牌，替换硬编码色值与 `rgba(0,0,0,…)`（深色下不可见）——输入框深色下白底、
  hover 叠加失效、状态徽标颜色不适配等深浅色主题问题一并修复。

- **验证**：cargo clippy 零告警、362 测试通过；vue-tsc 零错误、vitest 35/35、vite build 成功。

## 2026-09-03: ResourceProvider trait 化——资源协议公共流程收敛核心层

在统一资源协议之上再做机制收敛：`resources/*` 五操作的公共流程（列表包装、zip/manifest 上传、
幂等删除、状态事件推送）由 `symbio_core::resources::dispatch` 统一承载，各插件只实现
`ResourceProvider` trait 的差异化钩子：

- **核心**：新增 `ResourceProvider` trait（`kind` / `category` / `manifest_file` +
  `list_items` / `summarize` / `validate_manifest` / `on_uploaded` / `on_deleted` / `test_status`
  钩子，多数带默认实现）与 `dispatch` 统一分发入口；插件 route 顶部一行接入，非资源路径返回
  `None` 继续 match。新增 `ResourceGetRequest` schema 与 dispatch 单测（7 例）。

- **五插件接入**：skill / agent / mcp / model / session 全部改为实现 trait 钩子，删除各自手写的
  `resources_*_list/upload/delete/status` 方法。纯目录资源（mcp/skill）只实现 `summarize` 等
  轻钩子；model/session 重写 `list_items` 接管独立数据源；agent 在 `on_uploaded`/`on_deleted`
  中失效索引缓存（顺带修复上传/删除后列表不刷新的潜伏 bug）；mcp 的 `test_status` 连接测试结果
  经 `dispatch` 统一推送 resource 事件总线。

- **前端死代码清理**：删除无引用的 `ModelProvidersSettings.vue` / `ModelProviderCard.vue` /
  `constants/modelProviders.ts`（model 表单已由 `ResourceManagerView` 通用 JSON 表单承载）；
  删除 `services/resources.ts` 死导出 `resourcePath`/`toSummary` 与 `services/modelProviders.ts`
  死导出 `generateUniqueProviderId`（及其 spec）。

- **验证**：cargo clippy 零告警、362 测试通过；vue-tsc 零错误、vitest 35/35、vite build 成功。

## 2026-09-03: 统一资源协议落地（model/mcp/agent/skill/session 五类资源）

围绕"机制统一、最小差异化"重构前端与前后端协议，一份页面实例化多类资源：

- **后端统一协议**：新增 `symbio_core::schemas::resources`（`ResourceSummary` / `ResourceCapabilities` /
  `ResourcesListResponse` / `ResourceUploadRequest` / `ResourceDeleteRequest` / `ResourceStatusRequest`）与
  `symbio_core::resources`（`decode_zip_b64` / `parse_zip` / `strip_common_root` / `extract_zip_to_entity`）。
  model / mcp / agent / skill / session 统一暴露 `resources/list|upload|delete|status`；zip 资源（mcp/skill/agent）
  以文件名即目录名解压到 `~/.symbio/plugins/<category>/<id>/`。

- **能力开关驱动差异**：`capabilities_for(kind)` 成为后端单一真相源，前端 `ResourceManagerView` 按
  `zip_upload` / `independent_form` / `realtime_status` / `mutable` / `test_connection` / `read_only` 驱动 UI。

- **前端统一**：新增 `services/resources.ts` 与 `schemas/resources.ts`；`ResourceManagerView` 一份页面实例化
  model/mcp/skill/agent；删除遗留 AgentView/McpView/SkillView/ModelProvidersView 及 agents/mcpServers/skills 等
  旧 service 与孤儿 schema。

- **session 并入体系**：会话管理列表改用 `worker/session/resources/list` 统一契约（含 is\_working/message\_count/
  metadata 扩展），前端 `listSessions` 由 `ResourceSummary` 映射；`resources/status` 提供实时工作状态轮询。

- **model 收尾（消除双协议）**：chat 侧 `listModelProviders` 改从统一 `resources/list`（`worker/model`）读取——
  列表项 `extra` 展开完整 `config` 与 `is_default`，与资源管理页共用同一入口；`resources/upload` 对齐旧
  `providers/set` 补齐保存前校验 + 落盘；删除遗留 `providers/list|get|set|delete|set_default|test` 六个路由及
  对应前后端 CRUD schema/service（`modelProviders.ts` 仅保留映射 `listModelProviders` 与纯工具函数）。

- **清理**：删除后端孤儿 schema（session\_list/skill\_get/skill\_list/mcp\_servers）与 agent 旧 list/get/delete 路由、
  mcp 旧 `servers/*` 路由、skill 旧 list/get 路由。

## 2026-09-02: 停止按钮失效 + 工具命令白名单误报修复

- **修复停止按钮失效**：`sessions` store 的 `refreshList` 只把后端 `session/list` 返回的权威 `is_working` 合并进 `list`（卡片圆点），未回填 `sessionStatuses`——而停止按钮的 `isLoading` 只读后者。页面重载/视图挂载后，运行中的会话按钮显示为禁用的「发送」，点击无反应。现在 `refreshList` 把后端运行态回填到 `sessionStatuses`（仅 false→true 升级；true→false 收敛仍由事件流 idle/Abort/Error 负责，避免快照竞争误降级）

- **修复工具调用「命令不在允许列表中」误报**：`SecurityPolicy` 默认白名单补充常用命令（flutter/dart、node/npx/pnpm/yarn/bun、powershell/pwsh/cmd、pip、cp/mv/touch/ln/tar、head/diff/sed/awk/which、taskkill/ipconfig/netstat 等）；新增 `normalize_base_command` 归一化——去掉路径前缀与 `.exe/.cmd/.bat` 扩展名（`npm.cmd run build`、`python.exe` 此前均无法匹配白名单，`rm.exe` 的风险等级也会被误判为 Low）；`rm` 等高风险命令加入白名单仅消除误报文案，仍受 `block_high_risk_commands` 策略阻止（附 4 个单元测试）

## 2026-09-02: 工具失败语义收敛（recoverable 标记：信息性 vs 待恢复）

- **问题**：工具失败无条件显示重试按钮并自动展开，但 auto 模式下会话仍在运行——错误结果已喂给 LLM 继续处理，重试无意义（resume 在会话忙碌时也会被后端拒绝）；`retry`/`supply` 实际只为 **interactive 模式**（失败即暂停等用户恢复）设计

- **后端**：`chat_loop` 因工具失败退出循环（needs\_user\_action）时，给触发的 Failed ToolCall 打 `meta.recoverable=true` 并广播+持久化——服务端唯一真相，区分「信息性失败」与「待恢复失败」

- **前端**：重试/补充参数按钮仅对 `recoverable` 失败显示；可恢复失败自动展开（需要操作入口），运行中失败保持单行红标（可手动展开看错误，无按钮）

- **设计取向**：长期看 `supply` 应与 ask\_user/user\_prompt 结构化提问统一（参数修复走表单而非 JSON 补丁），保留但视觉降级

## 2026-09-02: 会话流响应体验重设计（Turn 响应分组 + 两级重试分派）

对齐 Claude / Codex 类智能体 UI，前后端配合（协议改动最小化）：

- **后端**：`session/orchestrator.rs::persist_failure` 失败终态（含 Turn）以 `StreamEvent::Update` 广播（时序在 Error 事件之前）——实时画面改信服务端，前端删除客户端启发式错误标记（刷新前后不再不一致），Error 事件仅承担 transport 级 ephemeral 兜底

- **前端 MessageNode.vue**：

  - 根级助手 Turn 改为**响应分组透明容器**，三形态：① Turn 无子节点且运行中 → 三点脉动「正在思考…」骨架；② 子节点（思考/正文/工具）出现后容器完全隐藏、直接纵排；③ Turn 失败 → 组级错误条 + 重试（`retry_turn`：删响应子树重建）

  - 思考节点**始终单行**：流式中「思考中…」呼吸动效，完成后「思考」+ 摘要预览，点开看全文

  - 工具调用**始终单行**（名称 + 状态标签，失败红标 + 悬停 ↻ 就地重试），例外：内含待审批子节点 / 可补充参数时自动展开

  - 工具卡片展开后**三段式**：请求（参数 JSON）/ 过程（子会话实时流，无子会话的工具整段隐藏）/ 结果（工具返回 + 审批提问），各自独立响应流

  - 正文（助手文本）始终展开；工具结果失败仅在结果段呈现错误文本，重试入口在工具行

- **重试两级分派**（`ModelChatPanel.handleRetry` 按 `msg.type` 路由）：工具失败 → `retry`（仅重执行该工具）；其余失败 → `retry_turn`（整轮响应重建）——两个粒度不再混淆

## 2026-09-02: 会话请求修复（temperature 序列化精度）+ 测试/构建卫生

- **修复 LLM 请求 400（temperature 参数非法）**：`ModelConfig.temperature` / `ModelProviderConfig.temperature` 由 `f32` 改为 `f64`——`f32 0.7` 经 serde\_json 升位 f64 后序列化为 `0.699999988079071`，被限制 2 位小数的模型 API 拒绝；f64 round-trip 精确，默认值恒为 `0.7`

- 修复 `time.spec.ts`「今天」用例硬编码日期随时间腐化（改用 `Date.now()` 相对构造）

- `tauri/package.json` 声明 `"type": "module"` + `vite.config.ts` 改用 `import.meta.dirname`，消除 Vite configLoader native 模式的 ESM/CommonJS 混用警告

## 2026-09-01: 全量依赖升级（Rust / Node / 前端 / CI）

**Rust**（保留精确版本，symbio/Cargo.toml）：thiserror 2.0.20、dirs 6、notify 8.2、fastembed 6.0.2、dashmap 6.2.1、which 8.0.6、reqwest 0.13.4、time 0.3.55、rusqlite 0.37 + tokio-rusqlite 0.7 配套锁定等。代码适配：

- tokio-rusqlite 0.7 移除 `Error::Other`，`call` 闭包直接返回 `Result<R, E>`，10 处调用点改写并显式标注 `rusqlite::Error`

- time 0.3.55 deprecated `format_description::parse`，改用 `parse_borrowed::<2>`（chat.rs / system\_prompt.rs）

- tauri/src-tauri 旧 Cargo.lock 与 rusqlite 0.37 冲突（links = "sqlite3"），重新生成

**前端**（tauri/package.json）：vite 8、@vitejs/plugin-vue 6、vitest 4、pinia 4、vue-router 5、marked 18、katex 0.18、mermaid 11、milkdown 7.22、@tauri-apps/\* 2.11。配置适配：

- vite 8（rolldown 内核）不支持对象形式 `manualChunks`，改为函数形式（vite.config.ts）

- vite 8 不再内置 esbuild，`minify: 'esbuild'` 改为 `'oxc'`

- typescript 保持 5.9：TS 7（native）与 vue-tsc 3.3 不兼容（实测 ERR\_PACKAGE\_PATH\_NOT\_EXPORTED）

**CI**：node 20→22（20 已 EOL）、checkout v5 / setup-node v5 / cache v4（消除 Node 20 deprecation 警告），release.yml 同步升级。

**验证**（2026-09-01）：

- `cargo fmt --check` / `cargo clippy -D warnings`：0 error

- `cargo test --workspace`：355 passed

- tauri/src-tauri `cargo check`：通过

- `npx vue-tsc --noEmit` / `npm test`（18 passed）/ `npm run build`：通过

***

## 2026-09-01: 连接测试 ping 请求修复 + CI 三处门禁修复

**一、模型连接测试报 400（`max_tokens must be greater than 2`）**

- 根因：`handle_ping` 探活请求硬编码 `"max_tokens": 1`，GLM 的 OpenAI 兼容网关要求 `max_tokens > 2`。

- 修复：三个协议（`openai_chat` / `anthropic_messages` / `gemini_api`）的 ping 请求统一调大到 16。

**二、GitHub CI 持续失败（三个 job 各一处根因）**

| job             | 根因                                                                                     | 修复                                                                                                                       |
| --------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| rust-checks     | CI 与本地 rustfmt 版本漂移导致 `fmt --check` 失败；rustfmt.toml 含 5 个 nightly-only 选项被 stable 静默忽略 | `rust-toolchain.toml` 锁定 `channel = "1.93.1"`；CI 改用 `actions-rust-lang/setup-rust-toolchain@v1` 读取该文件；清理 nightly-only 选项 |
| frontend-checks | `setup-node` 的 `cache: 'npm'` 在仓库根目录找不到 lock 文件（实际在 `tauri/`）                          | 增加 `cache-dependency-path: tauri/package-lock.json`                                                                      |
| security-check  | `cargo audit` 报 RUSTSEC-2025-0068：`serde_yml` 不维护且有 soundness 问题                       | 全量替换为维护中的 fork `serde_yaml_ng`（API 兼容，9 文件 20 处）                                                                         |

**验证**（2026-09-01）：

- `cargo fmt --all -- --check`：0 diff

- `cargo clippy --workspace --all-targets -- -D warnings`：0 error

- `cargo test --workspace`：355 passed / 0 failed

- `bash scripts/grep_audit.sh`：0 errors

- `npx vue-tsc --noEmit`：0 错误；`npm test`：18 passed

- Cargo.lock 已确认无 `serde_yml` 残留

***

## 2026-09-01: Model Provider"测试连接"路由修复

**Bug**：在 Model Provider 添加新模型时点击"测试连接"，报错
`Composite: 路径 'model_providers/test' 无法识别或子插件未挂载`。

**根因**：前端 `ModelProvidersView.vue` 的 `handleTest` 硬编码调用了 `model_providers/test`，
但该路径从未存在——Model Provider 管理路由的正确前缀是 `worker/model/providers/*`
（见 `tauri/src/services/modelProviders.ts` 的 `MODEL_PROVIDERS_PATH` 常量），
且后端此前**没有**独立的"测试连接"路由（只有 `providers/set` 会在保存时顺带校验）。

**修复（前后端联动）**：

| 端          | 改动                                                                                                                                                   |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| 后端 schema  | `symbio_core/schemas/model/model_providers.rs` 新增 `model_providers_test` 模块（Request 含 `provider` + `skip_validation`，Response 空）                     |
| 后端路由       | `plugins/model/plugin.rs` 新增 `providers/test`——复用 `validate_provider`（无副作用校验），**不写注册表、不落盘**，因此未保存的草稿配置也能直接测试；失败返回 `ValidationError("连接测试失败: {err}")` |
| 前端 schema  | `tauri/src/schemas/model_providers.ts` 新增 `ModelProvidersTest` namespace                                                                             |
| 前端 service | `tauri/src/services/modelProviders.ts` 新增 `testModelProvider()`（走 `worker/model/providers/test`）                                                     |
| 前端视图       | `ModelProvidersView.vue` 的 `handleTest` 改用 `testModelProvider`，移除 `model_providers/test` 硬编码与不再使用的 `callPlugin` 导入                                   |

**验证**（2026-09-01）：

- `cargo test --lib`：355 passed / 0 failed

- `cargo clippy --all-targets -- -D warnings`：0 error

- `cargo fmt --all -- --check`：0 diff

- `npx vue-tsc --noEmit`：0 错误

- `npm test`（vitest）：18 passed

**附带收益**：`providers/test` 作为无副作用路由，也是后续在设置表单中"实时校验"（输入即测）的稳定后端锚点。

***

## 2026-09-01: 全库质量门禁回归修复 + 前端测试基建

**背景**：项目级质量审计发现三处"门禁失真"：

1. `cargo clippy --lib` 实际存在 **12 个 warning**（文档声称 0）；
2. `cargo fmt --check` 存在 **83 处历史偏差**（CI 的 `--check` 门禁实际从未在当前 toolchain 下通过）；
3. 前端**零测试**（CI frontend job 仅有 vue-tsc + 空 sanity check）。

**变更**：

### 一、Rust 侧（业务行为零变化）

| 修复                                                            | 位置                                                                                                       |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `redundant_closure` ×3                                        | `symbio_core/event_bus.rs`（×2）、`providers/embedding/fastembed.rs`                                        |
| `collapsible_else_if`                                         | `plugins/agent/handlers/system_prompt.rs`                                                                |
| `unnecessary_if_let`（`.flatten()`）                            | `plugins/local/codebase_search.rs`                                                                       |
| `manual_div_ceil`                                             | `plugins/local/file_read.rs`                                                                             |
| `let_and_return`                                              | `plugins/mcp/http.rs`                                                                                    |
| `doc_overindented_list_items`                                 | `plugins/model/message_builder.rs`                                                                       |
| `unnecessary_cast`                                            | `plugins/session/heartbeat.rs`                                                                           |
| `ptr_arg`（`&mut Vec` → `&mut [T]`）                            | `plugins/skill/plugin.rs`                                                                                |
| `derivable_impls`（`#[derive(Default)]` + `#[default]`）        | `symbio_core/schemas/mcp/mcp_config.rs`                                                                  |
| `doc_lazy_continuation`                                       | `symbio_core/schemas/session/session_chat.rs`                                                            |
| `field_reassign_with_default` ×4（**测试代码**，`--all-targets` 门禁） | `plugins/mcp/http/tests.rs`（Default 赋值改结构体初始化语法）                                                         |
| 死代码清理                                                         | `plugins/agent/core/mod.rs::query_relation_names`（零调用）；`plugins/agent/capabilities/mod.rs` 3 个 v10 预留死常量 |
| `cargo fmt --all`                                             | 全库 83 处历史偏差统一，fmt 门禁恢复有效                                                                                 |

### 二、前端测试基建（从 0 到 1）

| 项                                | 说明                                                                            |
| -------------------------------- | ----------------------------------------------------------------------------- |
| 引入 `vitest@2.1.9`（devDependency） | 与 vite 6 对齐的稳定版本                                                              |
| 新增 `tauri/vitest.config.ts`      | `@` alias 与 vite.config 对齐；node 环境（纯逻辑层）                                      |
| 新增 18 个单元测试                      | `src/utils/__tests__/time.spec.ts`（6）+ `message.spec.ts`（12，多模态文本提取 / 消息键稳定性） |
| `package.json`                   | 新增 `test` / `test:watch` script                                               |
| CI（`.github/workflows/ci.yml`）   | frontend-checks job 新增 `npm test` 门禁                                          |

**验证**（2026-09-01）：

- `cargo test --lib`：355 passed / 0 failed

- `cargo clippy --lib -- -D warnings`：0 warning

- `cargo clippy --all-targets -- -D warnings`（CI 同款，含测试代码）：0 error（另修复 `plugins/mcp/http/tests.rs` 4 处 `field_reassign_with_default`）

- `cargo fmt --all -- --check`：0 diff

- `npm test`：18 passed（vitest）

- `npx vue-tsc --noEmit`：0 错误

**设计决策记录**：审计中评估的"认知内核与 Agent 插件壳解耦"（CognitionService trait 化 / crate 拆分）经确认**不采纳**——认知与智能体是一一对应的共生关系，`agent` 插件的"认知中心"内聚形态是设计使然。详见 `symbio/src/plugins/agent/docs/CHANGELOG.md` 同日条目。

***

## 2026-07-06: 项目级文档系统性同步 + 历史文档清理

**背景**：用户两轮反馈：

1. "项目文档与代码实现存在系统性脱节"——`docs/archive/proj/` 下历史规划文件、`docs/archive/design_docs/` 中早期提案、以及 `docs/README.md` 中插件文档列表与实际不符。
2. "继续，注意删除历史过期文档信息或者文档，确保所有文档保持最新"——既然能通过新方案覆盖过期文档，应**直接删除**而非保留横幅标注。

**变更**：

### 一、新增权威改进方案

1. **新增** **`docs/archive/proj/IMPROVEMENT_PLAN_2026.md`**（**权威改进方案**）

   - 基于 2026-07 当前代码状态的项目级下一阶段改进计划

   - 10 个改进方向（P0-P3）：文档脱节修复 / Plugin Channel 跨进程 / MCP 成熟化 / Skill 实战化 / E2E CI 化 / 可观测性 / HNSW ANN / 前后端类型同步 / 外部插件 / 移动端

   - 季度路线图（2026 Q3 / Q3-Q4 / 2027 Q1-Q2 / 2027+）

   - 取代 `PLAN.yml` / `TASK_INDEX.md` 作为项目要做的事的**唯一权威来源**

### 二、`docs/README.md` 修复

- §3 `design_docs/`：删除 `ARCHITECTURE_IMPROVEMENT.md` / `MODEL_CHAT_REDESIGN.md` / `COMPARISON_WITH_QWEN_CODE.md` 链接（已删除）

- §5 agent 插件文档列表：移除不存在的 `PROMPT_ARCHITECTURE.md` / `OPERATIONS.md` / `CODE_ANALYSIS_REPORT.md` 引用

- §6 "早期与产品向文档"：移除 `docs/archive/proj/` 引用（已清理为只剩 `IMPROVEMENT_PLAN_2026.md`）

- 添加"§6.1 现行改进方案"小节，链接到新的 `IMPROVEMENT_PLAN_2026.md`

### 三、删除过期历史文档（用户要求"删除"而非"加横幅"）

| 删除文件                                                     | 原因                                                                                                            |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `docs/archive/design_docs/ARCHITECTURE_IMPROVEMENT.md`   | Skill/Subagent/Hook 提案已**全部落地**（plugins/skill/, agent\_create+agent\_chat 组合, plugins/hook/）                  |
| `docs/archive/design_docs/MODEL_CHAT_REDESIGN.md`        | 扁平消息树设计已落地（`chat_message.rs` + Tauri `MessageNode.vue`）                                                       |
| `docs/archive/design_docs/COMPARISON_WITH_QWEN_CODE.md`  | 报告差异**多数已通过新增/重写插件弥合**，继续保留会持续误导                                                                              |
| `docs/archive/proj/PLAN.yml`                             | 2026-03 早期规划，与当前代码严重脱节                                                                                        |
| `docs/archive/proj/TASK_INDEX.md`                        | 2026-03 早期任务索引，24 个任务多数不适用                                                                                    |
| `docs/archive/proj/tasks/T001-project-infrastructure.md` | 早期任务                                                                                                          |
| `docs/archive/proj/tasks/T002-markdown-editor.md`        | 早期任务                                                                                                          |
| `docs/archive/proj/tasks/T004-docker-environment.md`     | 早期任务                                                                                                          |
| `docs/archive/proj/tasks/T010-rnaseq-template.md`        | 早期任务                                                                                                          |
| `docs/archive/proj/MODEL_CHAT_IMPLEMENTATION_PLAN.md`    | Phase 1-4 已落地，Phase 5 部分落地                                                                                    |
| `docs/archive/proj/MODEL_CHAT_IMPROVEMENT_PLAN.md`       | 已落地且**内容存在事实性错误**（声称的 `mcp/` / `hooks/` / `subagent/` / `workflow/` / `hmemory/` / `checkpoint/` 目录**实际不存在**） |

### 四、其他修复

- `docs/archive/design_docs/HISTORY_AND_REVIEWS.md`：

  - 修复"当前形态"错误描述（原称"纯 Rust 核心库 + E2E CLI"，但前端已于 2026-06 恢复为 Tauri）

  - 新增"2026-06 — Tauri 桌面前端恢复（当前形态）"里程碑小节

- `docs/archive/proj/IMPROVEMENT_PLAN_2026.md`：

  - 移除 §5 中指向已删除文件的引用

  - §2.1 改为"✅ 已完成"，列出全部已完成的删除项

**影响**：

- ✅ `docs/` 中无任何**事实性误导**文档

- ✅ `docs/archive/design_docs/` 仅保留 `HISTORY_AND_REVIEWS.md`（关键里程碑回顾）

- ✅ `docs/archive/proj/` 仅保留 `IMPROVEMENT_PLAN_2026.md`（项目级改进方案）

- ✅ "项目要做的事"有了单一权威来源（`IMPROVEMENT_PLAN_2026.md`）

- ✅ 关键决策轨迹仍在 `HISTORY_AND_REVIEWS.md` 中可追溯

**未改动**：

- 业务插件自包含文档（`symbio/src/plugins/agent/docs/`）保持不变

- `docs/explanation/*` / `docs/reference/*` / `docs/how-to/*` 权威文档保持不变

- `docs/CHANGELOG.md` 本文件**追加**本节记录

- `docs/ideas/*` 创意文档已整体移入 `docs/archive/ideas/`（属于产品方向探索，不在主文档树清理范围）

***

## 2026-06-15: 文档系统性更新

**背景**：上一轮代码与文档脱节（Tauri / Vue 引用遍布 `docs/`，但代码已剥离前端），用户要求按当前代码系统性更新项目文档。

**变更**：

1. **根** **`README.md`** **全面重写**

   - 移除 Tauri / Vue 全部引用；

   - 明确项目当前形态为"纯 Rust 核心库 + E2E CLI"；

   - 新增插件清单、能力路由示例、CLI 用法、快速开始、最小工作流。

2. **`docs/README.md`** **文档中心索引重建**

   - 重新组织为"核心架构设计 / 开发构建 / 设计草案 / 插件自包含 / 历史参考"五段；

   - 标注哪些文档"权威"、哪些"仅作历史参考"。

3. **`docs/explanation/*`** **与** **`docs/reference/*`** **三份文档全部更新**

   - `ARCHITECTURE.md`：补充分形路由树示意、插件清单、内核模块表；

   - `OPERATION_MECHANISM.md`：移除 Vue EventHandler / useChatEventHandler 等前端细节；

   - `API_DESIGN.md`：聚焦 V3.0 上下文注入版的 `Plugin` Trait 与 `PluginPayload` 4 态。

4. **`docs/how-to/*`** **三份文档全部更新**

   - `DEVELOPMENT_GUIDE.md`：聚焦机制化、Trait 抽象、Agent 子系统规范；

   - `BUILD_GUIDE.md`：移除 `pnpm tauri dev` 等前端命令，补 `cargo` 命令与排错；

   - `PLUGIN_DEVELOPMENT_GUIDE.md`：以 `weather` 插件为例演示完整链路。

5. **`docs/archive/design_docs/HISTORY_AND_REVIEWS.md`** **重写**

   - 按 v0.1.x / v0.1.5+ / v8 / v9 / v9.1 五段回顾关键里程碑；

   - 总结"机制化 vs 硬编码 / 文档代码同源 / identity 本质 / 前端剥离"四条经验。

6. **未改动文件**：

   - 业务插件自包含文档（`symbio/src/plugins/agent/docs/`）保持不变；

   - 本文件下方历史记录按"历史参考"原样保留。

***

## 2026-07-06: MCP 插件重构——对齐系统工具机制 + 清理误删

**背景**：用户两次反馈纠正早期对 MCP 插件的错误理解：

1. "前端并不负责任何 MCP 的调用，前端只是配置"——纠正了之前把 MCP 客户端实现归到前端的错误方向。
2. "call\_tool / discover / list\_tools 不是被调用的，系统的工具有现成机制（参考 web 插件等），所以 mcp 插件不会主动被调用的"——纠正了"为 MCP 单独设计一套调用 API"的过度设计。

**结论**：

- **后端**承担 MCP **配置管理**（CRUD）+ **客户端 transport**（stdio / http）

- **前端**仅做配置 UI（CRUD）

- MCP 工具通过 **系统统一的** **`Capability`** **trait +** **`traverse`** **+** **`tool_visitor`** **机制**集成到 agent——与 `web` 插件完全对齐

**变更**：

### 一、恢复 + 完善后端 MCP 客户端

1. **恢复** **`mcp/stdio.rs`** **+** **`mcp/http.rs`** **+** **`mcp/types.rs`**（误删纠正）

   - `mcp/stdio.rs`：stdio transport（每次调用临时 spawn 子进程 + kill）

   - `mcp/http.rs`：http transport（每次调用新建短连接）

   - `mcp/types.rs`：JSON-RPC 2.0 协议层类型（`JsonRpcRequest` / `JsonRpcResponse` / `McpTool` / `McpToolCallResponse` / `McpInitializeResponse` 等）

2. **新建** **`mcp/manager.rs`** —— 无状态 transport 路由器

   - `discover_tools(name, config)`：按 `transport_type` 路由到 stdio / http + 应用 `include_tools` / `exclude_tools` 过滤

   - `call_tool(name, config, tool_name, args)`：同上

   - **不维护**"激活集合"等运行时状态——是否可见由 `McpConfig.servers[name].enabled` 决定

3. **新建** **`mcp/capability.rs`** —— `McpToolCapability`

   - 把单个 MCP 工具包装为标准 `Capability`（`meta()` + `execute(ctx)`）

   - 命名规则：`mcp.<server_name>.<tool_name>` 三段式

   - 分类：`CapabilityCategory::Mcp`（新增变体）

### 二、集成系统工具机制

1. **改造** **`McpPlugin::traverse`**（参考 `WebPlugin::traverse`）

   - 每次 `parent.traverse(TRAVERSE_AVAILABLE_TOOLS)` 时遍历 `McpConfig.servers` 中 `enabled=true` 的项

   - 对每个 server 调 `McpManager::discover_tools` 动态发现工具

   - 把每个工具构造为 `McpToolCapability` 注册到 `ctx.get(CAPABILITY_VISITOR)`

   - agent 通过 `tool_visitor.invoke("mcp.<server>.<tool>", ctx)` 调用（与 `web_search` 等一致）

### 三、配置层统一 + 持久化

1. **升级** **`mcp_config::McpServerConfig`** 为完整版

   - 新增 `transport_type`（Stdio / Http / Sse）

   - 新增 `url`（http/sse 必填）

   - 新增 `include_tools` / `exclude_tools`（白/黑名单过滤）

   - 持久化路径不变：`~/.symbio/plugins/mcps/<name>/server.json`

2. **更新** **`servers/set`** **校验**：按 `transport_type` 校验必填字段（stdio → command；http/sse → url）

### 四、清理过度抽象

1. **删除 5 个多余 schema**：

   - `mcp_call_tool` / `mcp_discover` / `mcp_list_tools` / `mcp_register` / `mcp_unregister`

   - 这些功能通过 `Capability` trait + `tool_visitor` 机制实现，不再需要单独 schema

2. **删除** **`McpManager`** **的过度抽象**：

   - 移除 `register` / `unregister` / `is_active` / `active_servers` 集合

   - 移除 `tools_to_capabilities` / `list_capabilities` / `shared_manager` / `register_result_message` 等辅助

   - 移除 `types::stdio_command_args_env` 等内部辅助

### 五、Skill 插件清理

1. **删除未使用的** **`load_budget`** **/** **`estimate_tokens`** **方法**（之前为了"预留"留下但实际未使用）

### 六、文档同步

1. **`docs/archive/proj/IMPROVEMENT_PLAN_2026.md`** **§2.3** 重写：反映"前端只做配置 + 后端实现 transport + 系统工具机制集成"的正确方向
2. **`mcp_servers.rs`** **注释** 修正：删除"前端 tauri 端处理"的错误描述

**验证**：

- `cargo check`：✅ 零错误零警告

- `cargo test --lib`：✅ 233 tests passed

***

# VDFS 机制评估与收敛计划

状态：评估（结论 + 待办）；落地进度见文末「进度」。
范围：`symbio_core/vdfs_provider.rs`（纯接口）、`plugins/vdfs/*`（访问层 / 门面 / 物理层）、
`plugins/composite/vdfs.rs`（容器）、`providers/vdfs_service/*`（集中实现）、
以及消费方（各资源插件 + 前端）。
关联规范：[vdfs.md](vdfs.md)（机制）、[vdfs-frontend.md](vdfs-frontend.md)（前端）。

---

## 1. 结论

机制本身**成立且简洁**：一个纯接口（`VdfsProvider`）+ 一条地址规则（`.vdfs` 前缀分流）
+ 一组访问位（`r`/`w`/`l`/`t`）+ 一次能力广播，四点撑起全部资源访问，
「新增一类资源」确实只需实现一个 trait（§4）。这与它宣称的目标一致，值得保留。

问题不在机制，而在**两处历史遗留的旁路**，它们各自绕开了机制、又各自复制了一份机制已经解决的东西：

| # | 旁路 | 性质 | 现状 |
|---|---|---|---|
| A | 插件配置走 `CONFIG_GET`/`CONFIG_SET`（第二套资源协议），并由 `setting` 插件**代理 + 硬编码映射** | 与不变量 1（「所有资源只用 `vdfs/*`」）冲突；跨插件耦合；定义与校验寄生在 `setting` | **已消除**（§6） |
| B | 会话存储自造落盘原语，且**寻址有两套**（VDFS 语义路径 vs 磁盘目录函数） | 与 `providers/vdfs_service/entry.rs` 重复；子会话靠全盘扫描定位 | **寻址已合一**（`19e1c87`）；落盘原语为**有意不合并**，见 §5 |

另有若干 P2/P3 细节（见 §3）。两项改造的方案见 §5 / §6。

---

## 2. 机制为什么是简洁的（保留项，勿在改造中破坏）

1. **centerpiece 是纯接口**：`vdfs_provider.rs` 零 `use crate::`，可原样抽出成独立 crate；
   线路信封（`vdfs/*`、`VDFS_OPS`）只存在于 `plugins/vdfs/protocol.rs`。
   「core = 纯 trait，协议适配在插件」与 `model_provider` 同构。
2. **地址规则只有一条**（`UnifiedFs::half_of`）：规范化后是否以 `.vdfs` 打头。
   展示口径 ↔ 树内口径的映射只在门面进出两处各做一次，provider 只见自身子树相对路径。
3. **能力判据只有一条**（`VdfsAccess`）：`is_dir()` 就是 `access.list`，机制与消费者
   都不按 `kind` 特判 —— 这是 VDFS 通用的根基。
4. **provider 不含位置概念**：挂载名由注册方给出（约定 = 插件名），同一实现可挂任意位置；
   因此「换使用方策略」不动 provider。
5. **组合操作只写一次**（`edit_via` / `search_via` / `tree`）：trait 只含原子操作，
   「读改写」「递归过滤」留在访问层，不逼每个实现方重复实现。
6. **变更词汇封闭且载荷按类型可选**：`appended` 只带增量（热路径窄），
   `created`/`updated` 可带节点视图 + 内容快照（冷路径免回读）；
   `map_paths` 一次覆盖事件内全部路径，避免转发层漏翻译（这是正确的取舍）。
7. **两条链路同一个 `UnifiedFs`**：前端协议入口与 LLM 工具取到同一个虚拟根，
   「前端能做的」与「LLM 能做的」恒等，不存在第二套实现。
8. **一次广播、多条消费链路**：工具 / 模型服务 / 提示词 / VDFS 注册共用一次
   `traverse(TRAVERSE_AVAILABLE_TOOLS)`，未引入第二条收集通道。

## 3. 需要优化之处

### P1-1 插件配置是「第二套协议 + 一层代理」（对应 §1 的 A）—— ✅ 已消除（§6）

- `setting/plugin.rs` 的 `SETTING_SECTIONS` **硬编码** 6 个分区与各自的 `prefix`
  （`:132`），`read`/`write` 经 `route_config` 转发到目标插件的
  `<prefix>/config/get|set`（`:614`、`:681`、`:701`）。
- 于是同一份配置有**两个地址**：`.vdfs/setting/session`（面向用户）
  与 `session/config/get`（面向数据），后者不属于 VDFS 地址空间。
- `setting` 因此**必须认识** session/local/web/gateway 四个插件名与它们的配置路由
  ——这正是规范要避免的横向耦合。
- 定义（`session_detail_definition` 等）与校验（`validate_section`）也寄生在 `setting`，
  而字段的真源在各自的 `*Config` 类型里：**定义与数据分居两处**。

### P1-2 `CONFIG_GET`/`CONFIG_SET` 越权承担了「持久化聚合」—— ✅ 已消除（§6）

`home.save_config()` 的实现是「向 `worker` 要 `config/get` → composite 逐个插件聚合
→ 合并进 `symbio.plugins.*` → 写 config.yaml」（`home/plugin.rs:435`、`:523`；
`composite/composite.rs:187`）。

- 于是**读配置**这一个动作同时服务两个毫不相干的用途：给 UI 显示、给宿主落盘。
- 每个插件的 `config/get` 分支是为此存在的；`config/set` 分支在前端已无调用方
  （见 P1-3），成了死路由。
- 「配置协议」与「持久化协议」应当分开：**写配置者自报切片**，宿主只负责落盘
  （`save_config` 本来就是这样的路由，只是它现在还靠 `config/get` 反向拉取）。

### P1-3 前端 `config` 绑定已是死代码 —— ✅ 已删除（S6）

`VdfsFormDetail` 把定义适配为 `binding: 'option'` 并清空 `load_path`/`save_path`
（`components/vdfs/VdfsFormDetail.vue:101-107`），因此 `DetailForm` 的 `config`
分支（`components/vdfs/DetailForm.vue:548-580`，`onMounted` 拉取 + `saveConfig`）
**没有任何调用方**；`DetailDefinition.load_path`/`save_path` 只剩测试在断言。

→ 应删：机制里留一条「插件路由通道」的旁路，只会诱导后续实现再次绕开 VDFS。

### P1-4 会话存储是旁路，且寻址有两套（对应 §1 的 B）—— 寻址已合一，余项见 §5

- `session/store/mod.rs` 自造原子写、目录清单、删除（`:154`、`:214`、`:189`），
  与 `providers/vdfs_service/entry.rs`（`write_entry` / `list_entry_ids` /
  `remove_entry`）**同一件事两份实现**。
- 寻址两套：`plugin.rs::parse_session_path`（VDFS 语义路径）与
  `store/mod.rs::dir_for/file_for/sub_dir_for`（磁盘目录函数）。
  后果之一：**子会话只能靠 `find_nested_dir` 全盘扫描定位**（`:331`），
  而 VDFS 侧明明有 `<id>/子会话/<sub>` 这条直达地址。
- 后果之二：`SessionPlugin::session_of(id)` 为了「存在性校验」调用 `list_sessions()`
  ——**按 id 取一个会话却读取并解析全部会话文件**（`plugin.rs:1276`）。
  —— ✅ 已修（S7）：store 新增 `load_session_checked`（未命中给 `None`），
  `session_of` 改为按 id 直取；详见 §5 #4。

### P2（机制细节）

1. **重复挂载名静默丢一份** —— ✅ 已修。`CompositeVdfs::children_of` 改为
   **先排序、再去重**：收集时带上注册者（`(插件名, 目录名, provider)`），按
   `(order, 目录名, 插件名)` 排序后按目录名去重，重名 `warn` 并**点名双方**
   （`目录名「x」被 a 与 b 重复注册，保留 a（b 不可达）`）。两个改进：
   ① 重名不再静默——它必然让后来者完全不可达（`resolve` 只命中一个）；
   ② 胜出者与展示顺序都不再依赖 `HashMap` 的枚举顺序（原实现同 `order` 的
   先后完全随机，连「谁胜出」都是随机的）。测试 `duplicate_dir_names_keep_the_first_only`
   锁定该行为。
2. **`children_of` 每次操作都全量广播**（有意不缓存，保证与子插件集合一致），
   但 `vdfs/tree` 的递归 `list` 会退化成 O(节点数 × 子插件数)。
   → 可考虑**单次调用内**复用一次收集结果（调用级缓存），保留「不跨调用缓存」的既定语义。
3. **`VdfsNode.attributes` flatten 与机制字段同处一个命名空间**
   （`config_type` / `meta_tags` / `message_count` 与 `path` / `kind` / `ext` 平级）。
   机制新增字段名可能与场景字段撞车且无编译期保护。
   → 文档明确保留字，或场景字段统一加前缀（后者是破坏性变更，建议前者）。
   —— ✅ 已修（取「明确保留字」）：`vdfs.md` §3.2 列出全部机制字段名并声明为保留字，
   同时给出「新增机制字段须同步本节」的维护约定。
4. **`kind` 的语义漂移** —— ✅ 已修，但**原判断有误**，记录更正：`kind` 不存在
   「基础类型 + 场景类型」两套口径。`dir` / `file` 本来就是 `VdfsNode::dir` / `file`
   给出的**缺省场景标签**（`vdfs.md` §3.2 原文即把它们列为合法取值），插件名只是
   覆盖它——一个词表，不是漂移。真正的缺陷是**冗余**：`entry.rs::dir_node` 在
   `VdfsNode::dir` 已写好 `kind` 之后再复写一遍，等于给同一字段留了第二个来源。
   处置：删除 `dir_node`，三处挂载根（`single_file` / `memory` / `dir` 的 `stat("")`）
   直接用 `VdfsNode::dir("", label, VdfsAccess::LIST)`；`vdfs.md` §3.2 补一句明确
   「`kind` 只有一个词表，目录性永远只由 `l` 位表达」。

### P3（文档与一致性）

1. `symbio_core/vdfs_provider.rs` 的模块文档仍写「`composite` 容器是一个 **root 级
   provider**」（`:27`），与 `vdfs.md` §2.5「没有根级 provider 的概念」自相矛盾。
   —— ✅ 已修（S1）。
2. `symbio_core/paths.rs:18` 的注释「Config 插件」在协议废弃后应删。
   —— ✅ 已删（S2，`CONFIG_GET` / `CONFIG_SET` 常量一并删除）。
3. 本仓 `docs/design/vdfs.md` §13.4 已同步到「实体机制已删」，无需再改。

---

## 4. 检验标准（改造后应满足）

1. **一个协议**：`.vdfs` 之外不存在第二条资源读写协议；
   `grep -r "CONFIG_GET\|CONFIG_SET"` 归零。
2. **配置可寻址**：每个有配置的插件都能用 `vdfs/read` / `vdfs/write` 读写自己的配置，
   定义与校验由**配置的拥有者**产出。
3. **零横向耦合**：任何插件不出现另一个插件的名字、配置路由或类型。
4. **零冗余落盘原语**：会话存储的落盘走 `vdfs_service` 的同一套原语。
5. **单一定位**：一个会话的地址只有一种表达（`<id>` / `<id>/消息/<mid>` /
   `<id>/子会话/<sub>` / `<id>/工作目录/<rel>`），磁盘布局不再是寻址的一部分。
6. 门禁：`cargo check --tests` / `cargo test` / `vue-tsc` / vitest / 两个审计脚本
   与改造前基线一致或更好。

---

## 5. 改造一：会话存储的寻址与 VDFS 的关系（对应 P1-4）

**原则：存储层不再是「插件旁边的私有存储」，而是 VDFS 的一部分。**

> **落地结果（`19e1c87`，与本方案的差异已记录）**
>
> 对外已满足原则：`.vdfs/session` 只有一个入口——`SessionPlugin` 的
> `impl VdfsProvider`（清单 / `消息` / `子会话` / `工作目录`），存储是它下面的真相源。
>
> 对内**没有**让 `SessionStore` 自己实现 `VdfsProvider`，也没有复用
> `vdfs_service` 的三种集中实现——这是**有意的**，理由记在
> `plugins/session/store/mod.rs` 模块文档里：
>
> - `DirVdfs` 的定义是「条目内部可下钻浏览」，套上之后 `session.json` /
>   `messages/` / `tool_archives/` / `transcripts/` / `sessions/` 会原样成为对外
>   地址——**把物理布局当公共契约**；而会话要求 `<id>` 是叶子、内部只以人读语义段
>   呈现（与 agent bundle 的 `提示词` / `技能` / `MCP` 同一口径，vdfs.md §13.4）。
> - 条目也不是文件字节：消息**内联**在 `session.json` 里，`<id>/消息/<mid>` 是从
>   整份 `Session` 派生的视图，`append` / `replace` / `update` 的 seq 分配与剔孤儿
>   是会话专有的写入语义。
>
> 真正共用的是**寻址**：类别根取宿主层 `entry::category_dir`、id→段名取
> `entry::safe_segment`（经 `paths::safe_id`），因此会话目录名与 VDFS 资源条目
> 目录名永远是同一份规则（顺带把 `.` / `..` / 控制字符防护带进会话侧）。
> 判据与 vdfs.md §13.4 一致：**目录自管的类型自己落盘，不经 `vdfs_service`**。
>
> **因此本节的第 1 项按「更优形态」结案，第 2 项已完成；第 3 / 4 项未做，见下。**

1. ~~`SessionStore` 实现 `VdfsProvider`~~ —— 见上：**不复用集中实现**是定论，
   不再追求「存储层本身是一个 provider」。
   - ~~落盘原语复用 `entry.rs`~~ —— 只有**寻址**共用（`category_dir` /
     `safe_segment`），落盘原语保留在 store（拓扑相反，见上）。
   - **子会话直达**：`<id>/子会话/<sub>` 是 VDFS 侧地址；但 `find_nested_dir`
     的全盘扫描仍是 store 的定位手段，未收敛。
   - 临时会话（`ephemeral`）是**构造选型**（`SessionStore::ephemeral()`），不是
     第二个 trait 实现。
2. ✅ **寻址规则单一**：`paths::safe_id` 不再自带规则，委托 `entry::safe_segment`；
   `session_storage_dir()` 委托 `entry::category_dir(PLUGIN_SESSION)`。
3. ⬜ `SessionPlugin` 的 `impl VdfsProvider` 仍是**厚实现**（各分支在插件内直接
   调 store），未抽成「薄组合 + 存储 provider 承接」。**有意不抽**：本节开头已论证
   存储层不复用集中实现，再包一层 provider 只是把同样的分支换个地方写。
4. ✅ P1-4 已修：`session_of` 不再 `list_sessions()` 全量读后 `find`——store 新增
   `load_session_checked`（未命中给 `None`，而非 `load_session` 的空会话），
   `session_of` 直接按 id 取。**副作用**：`stat` / `read` 单个会话从「读并解析全部
   会话文件」降为「读并解析这一个」。`load_session` 变为 `load_session_checked`
   + 缺省空会话，语义不变。

## 6. 改造二：配置地址化，废弃 `CONFIG_GET`/`CONFIG_SET`（对应 P1-1/P1-2/P1-3）

共同部分（两个方案都要做）：

1. ✅ **删除协议**：`paths.rs` 的 `CONFIG_GET`/`CONFIG_SET`、各插件 route 里的两个分支、
   `composite` 的配置聚合、`home.collect_plugin_configs`、前端 `DetailForm` 的
   `config` 绑定与 `DetailDefinition.load_path/save_path`。
2. ✅ **配置文档 = 一个 VDFS 节点**：`ext = form`、`access = rw`、
   `schema` = 该插件的详情定义；`read` 返回当前配置 JSON，`write` = 校验 → 落内存 →
   落盘。共享实现是 `symbio_core::plugin_dir::ConfigFile`，各插件只是组合它 ——
   定义与校验**回归配置的拥有者**，不再寄生在 `setting`。
3. ✅ **校验归定义**：`DetailDefinition::validate(&Value)` 下沉到
   `symbio_core::schemas::detail`（定义与校验同源），`setting` 里的
   `validate_section` 随之删除。
4. ✅ **持久化 = 写自己的文件**：**没有宿主中转**。配置的地址就是插件目录里的那个
   文件（§6.3），写配置者直接落自己的 `PLUGIN.yml`。
   原先设计的「写配置者推切片（`save_config` 载荷 `{plugin, config}`）→ `home` 合并落盘」
   已**作废**——它仍然让父插件知道「有哪些插件、各自配置长什么样」，与
   「父插件不代管配置」的目标相反。

### 6.1 地址落在哪（**已定：方案甲**）

- **方案甲（配置归插件，已采纳）**：`.vdfs/<插件>/PLUGIN.yml`；`setting` 只保留前端自持的
  `appearance` / `about`。零新机制；代价是「设置」页不再聚合插件配置。
- **方案乙（设置即容器，未采纳）**：`.vdfs/setting/<插件>` 保留；各插件把配置文档
  **登记**给设置容器（同一广播的第二个登记位），`setting` 变成一个不认识任何插件的
  容器，硬编码的分区→插件映射整体消失。UX 不变；代价是 `CapabilityVisitor` 多一个
  登记位，且容器实现需从 `plugins/composite/vdfs.rs` 下沉到 `providers/vdfs_service/`
  复用。

**定论**：取甲——不新增机制（乙把「拓扑」从一处变成两处：容器合成 + 设置登记），
与「一个协议、一处注册」的收敛方向一致。UX 上「设置」页从「聚合各插件配置」
改为「本应用自身的设置（外观 / 关于）」，插件配置回到各自挂载点下。

### 6.2 配置文档的地址形状

- 挂载根**恒为目录**（前端导航与 `cwd` 语义如此），配置文档是根下的一个**文件**，
  文件名就是磁盘上的真实文件名 `PLUGIN.yml`：`.vdfs/<插件>/PLUGIN.yml`。
  **地址即文件路径**——不另造保留段名，前端拿到的那条就是插件目录里那一份。
- 已有资源树的插件（session）在自身 `list` / `stat` / `read` / `write`
  里按 `path == PLUGIN_FILE` 分流；只有配置、没有资源树的插件
  （local / web / gateway / telegram）直接把 `ConfigFile` 作为挂载内容。
- **不设配置文档的插件**：`model` / `mcp` 的配置**就是**它们的资源树
  （`.vdfs/model/<id>` / `.vdfs/mcp/<name>`，各自带详情定义与节点动作）。
  再挂一个 `PLUGIN.yml` 会是第二份真相，因此不挂；`model` 只把
  `default_provider_id` 写进自己的目录（读宽写窄，见 §6.3）。
- **不再有「保留段」问题**：`PLUGIN.yml` 是**文件**，而资源 id 是**目录名**，
  天然不冲突（`list_entry_ids` 只列子目录）。

### 6.3 改造三：配置回到插件目录（`PLUGIN.yml`）

把「配置」从「宿主代管的聚合文档」改为「插件目录里的一个文件」。目标不只是地址
好看，而是**解耦**：父插件不再需要知道任何子插件的配置形状，插件目录成为一个可整体
拷贝 / 移植的单元。

**规范**

- 一个插件 = 一个目录 `<homedir>/<插件>/`：里面既有配置 `PLUGIN.yml`，
  也有该插件自己的数据 / 资源（如 `model/<id>/provider.json`）。整目录可直接拷走。
- `PLUGIN.yml` 是 YAML 映射，其中两个**身份字段**：
  - `plugin_provider` —— 工厂 id，装配时据此 `has_creator` 判据；
  - `plugin_name` —— 实例名（缺省 = 目录名）。
  二者**不参与配置反序列化**，由 `PluginDir` 读写时自动剥离 / 补回。
  其余键即插件自己的配置，形状由插件自己定。
- **加载判据**：`PLUGIN.yml` 存在 + 可解析 + `plugin_provider` 指向已注册工厂。

**系统级插件：目录就是系统根**

- `home` 与它构造的容器 `composite` 的目录是**系统根** `<homedir>` 本身
  （`PluginDir::system`），配置在 `<homedir>/PLUGIN.yml`；容器只把系统根当**锚点**，
  它管辖的插件根是 `PluginDir::plugins_root()` = `<系统根>/plugins`。
  `composite` 可以理解为「`home` 动态加载自己的内置替身」——同目录、无自身配置。
- ⚠️ **`plugins/` 里不能有 `home`**：否则容器扫描插件根时会把它当普通插件再构造
  一次，那个 home 又构造容器 → 无限递归。自举环靠「系统级插件不住在 `plugins/` 下」消掉。

**职责划分**

| 角色 | 负责 | 不负责 |
| --- | --- | --- |
| 容器（`composite`） | 补必需插件目录、扫描插件根、按判据构造、把**自身目录**经 `PLUGIN_DIR` 告知被构造者 | 配置的形状 / 落盘 |
| 插件 | 读 / 写**自己目录**里的 `PLUGIN.yml` | 别人的配置 |
| 父插件（`home`） | 只做「必需插件清单」这一条**策略**（经 `REQUIRED_PLUGINS` 随构造传入） | 任何子插件的配置 |

- `REQUIRED_PLUGINS` 由**构造者**传入而非容器内置：`composite` 只是一个通用容器，
  它甚至可以自己嵌套另一个 `composite`；「缺省要加载哪些插件」是 `home` 的策略
  （`home::SYSTEM_PLUGINS`），容器不持有清单。
- 两个 ctx 键都是既有 `SymbioKey` 机制，**零新机制**：
  `PLUGIN_DIR`（`Value = PluginDir`）、`REQUIRED_PLUGINS`（`Value = Vec<String>`）。

**一次性迁移**

- 旧 `config.yaml` 的 `symbio.plugins.*` 逐项写成各插件 `PLUGIN.yml`；目标已存在则**跳过**
  （不覆盖用户已改过的配置）；旧文件改名 `config.yaml.migrated` 留档。
- `model` / `mcp` 的旧形态把资源明细混在配置里（`providers` / `servers`）：
  - `model` 走**读宽写窄**——读 `ModelProvidersConfig`（兼容旧形态），写
    `ModelConfig { default_provider_id }`，迁移完 `persist()` 一次把文件归一；
  - `mcp` 走 `PluginDir::remove_keys(&["servers", "_storage"])` 摘掉遗留键。

**下线清单**

`providers/vdfs_service/config.rs`（`ConfigDoc` / `SEG_CONFIG` / `is_config_path`）、
`schemas/common.rs::ConfigSlice`、`paths.rs::SAVE_CONFIG`、`home::save_config` /
`merge_slice` / `prune_migrated_plugin_data`、`composite` 的向上转发与 `get_parent`、
`model::persist_to_parent` 与它的 `parent` 通道。

**教训**

- 「保留段」这类约定是**地址污染**：一旦插件根下多出一个非资源项，所有「按场景把挂载根
  清单映射成业务列表」的前端代码都得按 `ext` 过滤（`listSessions()` 曾因此多出一条
  id 为 `配置` 的伪会话）。改成**真实文件名**后这条隐患自动消失——文件与目录本就不同名空间。
- 装配期的 `ensure_manifest` 必须**只补身份字段、绝不覆盖**已有文件；它同时是
  「必需插件即便从未配置过也要有一个可编辑的 `PLUGIN.yml`」的保证。

---

## 7. 进度

- [x] 评估（本文档）
- [x] 改造一：会话存储的寻址与 VDFS 的关系（寻址已合一 `19e1c87`；#1/#2/#4 结案，
      #3 有意不做）
- [x] 改造二：配置地址化，废弃 `CONFIG_GET`/`CONFIG_SET`（S1 / S2 / S4 / S5 / S6）
- [x] 改造三：配置回到插件目录（`PLUGIN.yml`），父插件不再代管配置（S8 / S9，见 §6.3）
- [x] P2-1 / P2-3 / P2-4 顺带收敛
- [ ] P2-2（调用级缓存，收益未证实，暂缓）

### 已落地

- **S1 core 契约**：`DetailDefinition::validate`（定义自带校验，含
  `DetailCondition::holds` / `DetailField::check`）下沉到
  `symbio_core::schemas::detail`；`setting::validate_section` 与三个自由函数删除
  （-195 行）；`DetailDefinition.load_path` / `save_path` 删除（P1-3 的机制侧旁路）；
  `vdfs_provider.rs` 模块文档的「root 级 provider」改为「目录合成」（P3-1）。

- **S2 配置文档实现**（机制已在 S8 被取代）：`providers/vdfs_service/config.rs` 新增
  `ConfigDoc`（节点形状 / 定义校验 / 切片推送；**值 + 一组函数，不是 trait**）+
  `SEG_CONFIG` / `is_config_path`；`paths.rs` 删 `CONFIG_GET`/`CONFIG_SET`、
  增 `SAVE_CONFIG`；`schemas/common.rs` 新增 `ConfigSlice { plugin, config }`。
  —— 该文件的「配置文档」抽象与切片推送在 S8 中**整体删除**，只留下「校验归定义」
  与「定义归拥有者」两条结论（`DetailDefinition::validate` 保留在 `symbio_core`）。

- **S4 四个插件接入**（local / web / gateway / session）：删除各自的
  `CONFIG_GET`/`CONFIG_SET` 分支；`config_definition()` **回归配置的拥有者**
  （默认值从各自的 `Default` 读出，不再有第二份字面量）；`local` / `web` /
  `gateway` 直接以 `ConfigDoc` 为挂载内容，`session` 在自身资源树里分流配置段。
  网关的只读白名单同步收紧：拒绝对配置文档的 `vdfs/read`（配置含凭据）。
  —— 地址名在 S8 改为 `PLUGIN.yml`。

- **S5 持久化改推送 + 其余插件**（推送部分已在 S8 作废）：`composite` 删配置聚合分支
  （只向上转发 `SAVE_CONFIG`，不解释）；`home` 的 `save_config` 改为收切片 →
  按 `plugin_provider` 定位既有键 → 逐键合并 → 原子落盘（新增 `merge_slice` /
  `flush`，删除 `collect_plugin_configs`）；`model` / `mcp` / `telegram`
  删两个协议分支——`model` 的 `persist_to_parent` 改为推
  `ConfigSlice { plugin: "model", config: { default_provider_id } }`，
  `mcp` / `telegram` 顺带删掉已无用的 `parent` 字段与访问器，
  `telegram` 新增配置文档（原配置无任何地址）。

- **S6 setting 瘦身 + 前端清理**：`setting` 变为**无状态 provider**（794 → ~250 行），
  `SETTING_SECTIONS` 从 6 项减为 `appearance` / `about` 两项，`route` 恒 `NotFound`，
  `read`/`write` 对分区恒 `Forbidden`；前端删 `DetailForm` 的 `config` 绑定
  （`configSaving` / `load_path` / `save_path` / `saveConfig`）、
  `schemas/vdfs-form.ts` 的 `load_path` / `save_path`、`VdfsFormDetail` 的通道适配
  与 `DetailFormConfig.spec.ts`。

- **S7 机制细节收敛（P2-1 / P2-3 / P2-4 + §5 #4）**：`CompositeVdfs::children_of`
  重名记账 + 告警（先排序再去重，胜出者确定）、排序加目录名 / 插件名兜底；
  删除冗余的 `entry::dir_node`（三处挂载根直接用 `VdfsNode::dir`）；
  `vdfs.md` §3.2 明确「`kind` 单一词表」与「机制字段保留字」；
  `SessionStore::load_session_checked`（存在性判据）替换 `session_of` 的全量清单 `find`。

- **S8 配置回到插件目录（改造三，§6.3）**：新增 `symbio_core::plugin_dir`
  （`PLUGIN.yml` 规范 + `PluginDir`：`of` / `at` / `system` / `plugins_root` /
  `read_manifest` / `load` / `save` / `ensure_manifest` / `remove_keys`，身份字段
  读写自动剥离 / 补回）；新增 `plugin_dir::ConfigFile`（`node` / `read` / `apply` =
  校验 → 落内存 → 落自己的文件 → 广播），地址用**真实文件名** `PLUGIN.yml`；
  新增 ctx 键 `PLUGIN_DIR` + `dir_from_ctx` 入口；`local` / `web` / `gateway` /
  `telegram` / `session` 五个插件从 `ConfigDoc` 迁到 `ConfigFile`；
  `model` / `mcp` 改为读写自己目录（`model` 读宽写窄 + `persist()` 归一，
  `mcp` 用 `remove_keys` 摘 `servers` / `_storage`）；`home` 做一次性迁移
  （旧 `config.yaml` 的 `symbio.plugins.*` → 各插件 `PLUGIN.yml`，已存在则跳过，
  旧文件改名 `config.yaml.migrated`）；**删除** `providers/vdfs_service/config.rs`
  （`ConfigDoc` / `SEG_CONFIG` / `is_config_path`）、`ConfigSlice`、`SAVE_CONFIG`、
  `home::save_config` / `merge_slice` / `prune_migrated_plugin_data`、
  `composite` 的向上转发与 `get_parent`、`model::persist_to_parent`。

- **S9 系统级插件 + 清单归构造者**（两条架构纠正）：`home` 与容器 `composite` 的目录
  改为**系统根**（`PluginDir::system`），`plugins/` 下不再有 `home`——消掉
  「容器扫到 home 再构造一个 home」的自举环；容器经 `PluginDir::plugins_root()` 定位
  自己管辖的插件根；容器**不再内置**必需插件清单，改由 `home` 经新增的 ctx 键
  `REQUIRED_PLUGINS` 随构造传入（`home::SYSTEM_PLUGINS`），使 `composite` 成为可嵌套的
  通用容器。

- **S10 隐藏属性 + 设置页的插件配置清单**（改造三的两个收尾，见 §6.3 与 vdfs.md §3.2 / §13.1）：
  ① **隐藏属性回归机制**——`VdfsNode::hidden`（文件 / 目录通用：列表里不出现、可达性不受
  影响）+ `VdfsProvider::root_hidden()`（与 `root_access` / `root_status` / `root_new_types`
  同构，容器合成该目录节点时回填）；过滤由**产出列表的一方**执行（容器对自己合成的子目录
  清单与任何子 provider 交回来的 `list` 结果做同一条过滤）。`web` / `local` / `gateway`
  三个「只有一份配置文档」的目录据此从左侧导航隐去。**未引入**「挂载索引 / 挂载清单」这类
  中间物——provider 的根本来就是一个目录节点。② **第三条收集通道 `ConfigurableVisitor`**
  （ctx 键 `CONFIG_VISITOR`）：与能力 / 选项并列，共用同一次 `traverse` 广播；插件侧唯一
  入口 `announce_configurable(&ctx, &self.config_file)`，条目形状由 `entry_of(&ConfigFile)`
  统一构造（名字 = 目录名、地址 = 真实地址 `<目录名>/PLUGIN.yml`、标题 / `ext` / 表单定义
  取自 `ConfigFile::node()`）；容器用**共享**收集器（声明自带目录名，无归属歧义）并
  **写回请求 ctx**，设置插件据此列清单——不反查插件目录、不硬编码插件表、协议零新增字段。
  设置插件的 `list` 因此 = **各插件交出来的配置条目 + 自有分区**（配置在前，
  `appearance` / `about` 垫后；不代理读写，条目地址就是拥有者那份文件）。前端只补
  `setting:telegram` 图标（图标本就不进协议，按 `kind:<目录名>` 查 UI 映射表）。
  **词汇口径**：`mount` / `link` 都不进协议——VDFS 下一切都是目录和文件，地址由节点
  自己的 `path` 表达。

- **门禁**：`cargo check --tests` 零错误零告警；`cargo clippy --workspace --all-targets
  -- -D warnings` 通过；`cargo test --lib` 全绿（另顺带修掉两处既有 clippy 报错：
  `SettingPlugin::default()` × 4、`sort_by_updated_desc` 的 `&mut Vec`）；
  `vue-tsc --noEmit` 通过；`vitest run` 全绿。

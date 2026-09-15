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

| # | 旁路 | 性质 |
|---|---|---|
| A | 插件配置走 `CONFIG_GET`/`CONFIG_SET`（第二套资源协议），并由 `setting` 插件**代理 + 硬编码映射** | 与不变量 1（「所有资源只用 `vdfs/*`」）冲突；跨插件耦合；定义与校验寄生在 `setting` |
| B | 会话存储自造落盘原语，且**寻址有两套**（VDFS 语义路径 vs 磁盘目录函数） | 与 `providers/vdfs_service/entry.rs` 重复；子会话靠全盘扫描定位 |

另有若干 P2/P3 细节（见 §3）。两项改造的方案见 §5 / §6。

---

## 2. 机制为什么是简洁的（保留项，勿在改造中破坏）

1. **centerpiece 是纯接口**：`vdfs_provider.rs` 零 `use crate::`，可原样抽出成独立 crate；
   线路信封（`vdfs/*`、`VFDS_OPS`）只存在于 `plugins/vdfs/protocol.rs`。
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

### P1-1 插件配置是「第二套协议 + 一层代理」（对应 §1 的 A）

- `setting/plugin.rs` 的 `SETTING_SECTIONS` **硬编码** 6 个分区与各自的 `prefix`
  （`:132`），`read`/`write` 经 `route_config` 转发到目标插件的
  `<prefix>/config/get|set`（`:614`、`:681`、`:701`）。
- 于是同一份配置有**两个地址**：`.vdfs/setting/session`（面向用户）
  与 `session/config/get`（面向数据），后者不属于 VDFS 地址空间。
- `setting` 因此**必须认识** session/local/web/gateway 四个插件名与它们的配置路由
  ——这正是规范要避免的横向耦合。
- 定义（`session_detail_definition` 等）与校验（`validate_section`）也寄生在 `setting`，
  而字段的真源在各自的 `*Config` 类型里：**定义与数据分居两处**。

### P1-2 `CONFIG_GET`/`CONFIG_SET` 越权承担了「持久化聚合」

`home.save_config()` 的实现是「向 `worker` 要 `config/get` → composite 逐个插件聚合
→ 合并进 `symbio.plugins.*` → 写 config.yaml」（`home/plugin.rs:435`、`:523`；
`composite/composite.rs:187`）。

- 于是**读配置**这一个动作同时服务两个毫不相干的用途：给 UI 显示、给宿主落盘。
- 每个插件的 `config/get` 分支是为此存在的；`config/set` 分支在前端已无调用方
  （见 P1-3），成了死路由。
- 「配置协议」与「持久化协议」应当分开：**写配置者自报切片**，宿主只负责落盘
  （`save_config` 本来就是这样的路由，只是它现在还靠 `config/get` 反向拉取）。

### P1-3 前端 `config` 绑定已是死代码

`VdfsFormDetail` 把定义适配为 `binding: 'option'` 并清空 `load_path`/`save_path`
（`components/vdfs/VdfsFormDetail.vue:101-107`），因此 `DetailForm` 的 `config`
分支（`components/vdfs/DetailForm.vue:548-580`，`onMounted` 拉取 + `saveConfig`）
**没有任何调用方**；`DetailDefinition.load_path`/`save_path` 只剩测试在断言。

→ 应删：机制里留一条「插件路由通道」的旁路，只会诱导后续实现再次绕开 VDFS。

### P1-4 会话存储是旁路，且寻址有两套（对应 §1 的 B）

- `session/store/mod.rs` 自造原子写、目录清单、删除（`:154`、`:214`、`:189`），
  与 `providers/vdfs_service/entry.rs`（`write_entry` / `list_entry_ids` /
  `remove_entry`）**同一件事两份实现**。
- 寻址两套：`plugin.rs::parse_session_path`（VDFS 语义路径）与
  `store/mod.rs::dir_for/file_for/sub_dir_for`（磁盘目录函数）。
  后果之一：**子会话只能靠 `find_nested_dir` 全盘扫描定位**（`:331`），
  而 VDFS 侧明明有 `<id>/子会话/<sub>` 这条直达地址。
- 后果之二：`SessionPlugin::session_of(id)` 为了「存在性校验」调用 `list_sessions()`
  ——**按 id 取一个会话却读取并解析全部会话文件**（`plugin.rs:1276`）。

### P2（机制细节）

1. **重复挂载名静默丢一份**：`CompositeVdfs::resolve` 用 `find` 取首个同名项
   （`composite/vdfs.rs:163`）；两个插件注册同一目录名时，后来者无声消失。
   → 至少 `warn`，或直接报错。
2. **`children_of` 每次操作都全量广播**（有意不缓存，保证与子插件集合一致），
   但 `vdfs/tree` 的递归 `list` 会退化成 O(节点数 × 子插件数)。
   → 可考虑**单次调用内**复用一次收集结果（调用级缓存），保留「不跨调用缓存」的既定语义。
3. **`VdfsNode.attributes` flatten 与机制字段同处一个命名空间**
   （`config_type` / `meta_tags` / `message_count` 与 `path` / `kind` / `ext` 平级）。
   机制新增字段名可能与场景字段撞车且无编译期保护。
   → 文档明确保留字，或场景字段统一加前缀（后者是破坏性变更，建议前者）。
4. **`kind` 的语义漂移**：机制声明「`kind` 只承载场景语义、不参与判定」，
   但 `entry.rs::dir_node` 又把它写成 `VFDS_KIND_DIR`；`session`/`setting` 则写插件名。
   → 统一口径：`kind` 一律是场景标签，目录性只由 `l` 位表达（`dir_node` 改为不写 `kind`）。

### P3（文档与一致性）

1. `symbio_core/vdfs_provider.rs` 的模块文档仍写「`composite` 容器是一个 **root 级
   provider**」（`:27`），与 `vdfs.md` §2.5「没有根级 provider 的概念」自相矛盾。
2. `symbio_core/paths.rs:18` 的注释「Config 插件」在协议废弃后应删。
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

## 5. 改造一：会话存储改用 `VdfsProvider`（对应 P1-4）

**原则：存储层不再是「插件旁边的私有存储」，而是 VDFS 的一部分。**

1. `SessionStore` 实现 `VdfsProvider`，地址空间 = 会话存储半边：
   `<id>`（会话文档）、`<id>/消息/<mid>`（转写列表项）、`<id>/子会话/<sub>`。
   - 落盘原语复用 `providers/vdfs_service/entry.rs`（`safe_segment` / 原子写 /
     目录清单 / 删除），删除 `store/mod.rs` 里的重复实现；
   - 子会话由 `<id>/子会话/<sub>` 直达，`find_nested_dir` 的全盘扫描不再参与 VDFS 寻址；
   - 临时会话（`ephemeral`）同一 trait，只是不落盘。
2. **寻址规则单一**：`parse_session_path` 归存储 provider，插件**复用**它做分流
   （不再各写一份）。
3. `SessionPlugin` 的 `impl VdfsProvider` 收敛为薄组合：
   - `工作目录` 分支 → `workdir` 模块（会话的工作区是**另一个文件系统**，不属于会话存储）；
   - 其余分支 → 存储 provider 原样承接；
   - **运行时叠加**（在途消息、工作状态）留在插件（那是会话引擎的状态，不是存储）；
   - 变更广播留在插件（它是唯一带**载荷**的 provider，见 vdfs.md §9）。
4. 顺带修 P1-4 的 `session_of`：按 id 直接取，不再 `list_sessions()` 全量读。

## 6. 改造二：配置地址化，废弃 `CONFIG_GET`/`CONFIG_SET`（对应 P1-1/P1-2/P1-3）

共同部分（两个方案都要做）：

1. **删除协议**：`paths.rs` 的 `CONFIG_GET`/`CONFIG_SET`、各插件 route 里的两个分支、
   `composite` 的配置聚合、`home.collect_plugin_configs`、前端 `DetailForm` 的
   `config` 绑定与 `DetailDefinition.load_path/save_path`。
2. **配置文档 = 一个 VDFS 节点**：`ext = form`、`access = rw`、
   `schema` = 该插件的详情定义；`read` 返回当前配置 JSON，`write` = 校验 → 落内存 →
   触发持久化。共享实现放 `providers/vdfs_service/config.rs`（`ConfigVdfs`），
   各插件只是组合它 —— 定义与校验**回归配置的拥有者**，不再寄生在 `setting`。
3. **校验归定义**：`DetailDefinition::validate(&Value)` 下沉到
   `symbio_core::schemas::detail`（定义与校验同源），`setting` 里的
   `validate_section` 随之删除。
4. **持久化改为推送**：写配置者把**自己的切片**交给宿主
   （`save_config` 载荷 = `{plugin, config}`），`home` 只负责合并落盘，
   不再反向拉取。切片按 `plugin_provider` 定位既有键，兼容实例改名。

### 6.1 地址落在哪（**已定：方案甲**）

- **方案甲（配置归插件，已采纳）**：`.vdfs/<插件>/配置`；`setting` 只保留前端自持的
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

- 挂载根**恒为目录**（前端导航与 `cwd` 语义如此），配置文档是根下的一个**保留子段**
  `配置`：`.vdfs/<插件>/配置`。
- 已有资源树的插件（session / model / mcp）在自身 `list` / `stat` / `read` / `write`
  里把 `配置` 段交给 `ConfigVdfs`；只有配置、没有资源树的插件
  （local / web / gateway / telegram）直接把 `ConfigVdfs` 注册为挂载点。
- `配置` 是**保留段**：插件自身资源 id 不得占用（会话 id 是 UUID，天然不冲突；
  model / mcp 的 id 由用户命名派生，理论上可撞名——由「保留段优先」的判定顺序
  兜底，并在规范中写明）。

---

## 7. 进度

- [x] 评估（本文档）
- [ ] 改造一：会话存储上 `VdfsProvider`
- [ ] 改造二：配置地址化，废弃 `CONFIG_GET`/`CONFIG_SET`
- [ ] P2-1 / P2-4 顺带收敛

### 已落地

- **S1 core 契约**：`DetailDefinition::validate`（定义自带校验，含
  `DetailCondition::holds` / `DetailField::check`）下沉到
  `symbio_core::schemas::detail`；`setting::validate_section` 与三个自由函数删除
  （-195 行）；`DetailDefinition.load_path` / `save_path` 删除（P1-3 的机制侧旁路）；
  `vdfs_provider.rs` 模块文档的「root 级 provider」改为「目录合成」（P3-1）。

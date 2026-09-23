# VDFS 前端迁移路线（档案）

> **归档文档**。本文原为 [`docs/design/vdfs-frontend.md`](../design/vdfs-frontend.md) 的
> §7「迁移路线」与 §7.1「落地进度」，2026-09-23 原样归档。S1–S17 每一步的改动、
> 验收与提交号都在这里。
>
> **本文不再更新**：迁移已全部落地——`entities/*` 自 S11 起无任何路由，资源存储只有
> `providers/vdfs_service` 的三个 `VdfsProvider` 实现。文中以 `EntityProvider` /
> `EntityVdfsAdapter` / `EntityStore` / `StorageService` 为主语的条目记录的是迁移期的
> 中间形态，这些类型**已全部删除**，只对理解「当时为什么这么走」有意义。
>
> 现行文档：[vdfs-frontend.md](../design/vdfs-frontend.md)（前端页面规范）、
> [vdfs.md](../design/vdfs.md)（机制规范，§13.4 记存储层形态）。
> 关联档案：[vdfs-review.md](./vdfs-review.md)（更早的机制评估）、
> [entity-provider-mechanism.md](./entity-provider-mechanism.md)（已废除的实体机制）。

---

## 迁移路线（自顶向下，逐步推进）

原则：**先立页面规范（顶），再逐个迁移 provider（底）**；每步独立可验证、
可提交，绝不半途并存两套页面机制。

| 阶段 | 内容 | 产出 |
|---|---|---|
| **S1 页面规范地基** | 地址模型 `<根>`（§3）+ 导航元数据（§4.1）+ 可新建类型（§5）+ 添加/类型选择器 | 前端页面机制就位；后端 `new_types` 机制就位 |
| **S2 设置迁移** | `setting` 已是 provider；把设置入口从 `/settings`（entities）切到 `/vdfs/setting` | 设置页走 VDFS；`entities/setting` 退场 |
| **S3 会话迁移** | 新增 `session` provider：根 = 会话清单（`new_types = [会话]`）、节点 = 会话（`ext = session`）；read/write/delete 转发既有会话协议 | 会话页走 VDFS；聊天工作区作为 `session` 渲染器 |
| **S4 其余迁移** | 用一个通用 `EntityVdfsAdapter` 把既有 `EntityProvider` 接成挂载点（`model` / `skill` / `mcp` 可写、`agent` 只读）；外壳左栏切到 `<根>` 根 | 全部资源在 `<根>` 下可见可管；统一实体页按类型逐个退场 |
| **S5 下线旧协议** | 移除 `entities/*` 路由与前端实体页（`/entities/*` 仅留保兼容重定向），导航完全由 `<根>` 驱动 | 一个协议、一个页面 |
| **S6 会话内部重建** | 会话内部结构（子会话 / 工作目录树）改由 VDFS 同名目录承载，原容器实体页可替代 | S5 的阻塞解除 |
| **S7 容器子实体重建** | 通用适配器按 `container_kinds` 支持 `<id>/<子类别>/<条目>`；agent 目录内部（提示词 / 技能 / MCP）上 VDFS | 容器页最后一处不可替代能力消失 |
| **S8 会话清单上 VDFS** | 会话节点自带 `message_count` / `metadata` / `meta_tags`；`listSessions()` 改走 `vdfs/list`，`services/entities.ts` 删除 | 前端 `entities/*` 调用点归零 |
| **S9 导航可见性** | ~~机制层新增 `VdfsProvider::nav_visible()`~~ **已整体撤销**：物理层从 `plugins/local` 迁入 `plugins/vdfs`，本地文件不再是子目录，左栏自然回到「六类资源」，无需可见性标记（**2026-09-15 复核**：改造三之后 `web`/`local`/`gateway` 必须留在树里，改用机制级隐藏属性 `VdfsNode::hidden` + `root_hidden()`，见下） | 左栏 = 六类资源，无按名硬编码（**结果达成，机制未引入**；后来的隐藏属性是「存在但不列」这一独立问题的机制解） |
| **S10 节点动作** | 新增 `vdfs/action` 操作 + `VdfsProvider::action()`（默认 `NotImplemented`）；适配器把 `test` 接到 `EntityProvider::test_status`；前端把「测试连接」接回 | S5 后丢失的连通性自检回归 |
| **S11 下线 `entities/*` 协议** | 6 个插件不再路由 `entities/*`，`entities::dispatch` 与其请求/响应、zip 工具一并删除；网关只读白名单改列 `vdfs/*` 读操作 | 对外只剩 VDFS 一个资源协议 |
| **S12 整包导入** | `VdfsNewType.source`（`file`）+ `EntityProvider::import_zip` 钩子；适配器的二进制 `write` 承接导入（agent 走 `AgentDirStore::import`），agent 补上 `delete_item` | S5/S11 后丢失的 zip 导入回归，且**不新增协议操作** |
| **S12 清理** | 注册表去掉 `prefix` / `provider_name` / `compact_list` / `status_indicator` 与 `EntityCapabilities`（改由 `supports_import` 表达）；删协议时代的请求/响应与 `get_item`；`agent` 的旧协议路由（已被 VDFS 取代）只保留导出 | 历史冗余与被替换代码清空 |
| **S13 整包导出** | `VDFS_ACTION_EXPORT` 节点动作 + `EntityProvider::export_zip` 钩子（默认 `zip_dir`、agent 走 `AgentDirStore::export`）；结果按「文件载荷」（`filename` + `b64`）回传；`agent` 的旧协议路由整体下线 | 导入/导出在 VDFS 内闭环；`agent` 插件零自有路由 |
| **S16 收敛终局（废除实体机制）** | 删 `EntityProvider` trait / `provider_registry()` / `EntityVdfsAdapter`；`model` / `skill` / `agent` 各补一份 `impl VdfsProvider`（与已有的 `session` / `setting` / `mcp` 同构）；`entities.rs` 降为存储原语自由函数 | 后端只剩 VDFS 一套机制；**前端零改动**——挂载名与节点形状不变 |
| **S17 收敛存储层** | 删 `providers/storage_service` 与 `symbio_core` 的 `EntityStore` / `StorageService` / 存储原语，改为 `providers/vdfs_service` 的三个 `VdfsProvider` 集中实现（单文件 / 目录 / 内存）；`schemas/entities.rs` 收敛为 `DetailDefinition` 表单方言；事件总线只留 `kind = "vdfs"` | 资源存储讲的也是 VDFS 的话；磁盘布局不变，前端只退一个 `entity` 频道订阅 |

每阶段的验收：`cargo check` + `cargo test` + `vitest run` 全绿；被迁移资源的
**新建 / 列出 / 详情 / 编辑 / 删除 / 实时** 六项行为与迁移前**等价**。

## 落地进度

- **S1 页面规范地基**：本文件 + 后端 `new_types` 机制 + 前端 `<根>` 地址模型与添加/类型选择
  （**已完成**，提交 `51c7f34` 后端机制 / `9d614f4` 前端页面规范）。
- **S2 设置迁移**（**已完成**）：
  - 设置入口 `navTargetOf('setting')` 由 `/settings` 切到 `/vdfs/setting`；旧地址
    `/settings` 改为 redirect 保兼容（书签 / 深链）。
  - 后端 setting provider 的 `section_node` 按**渲染器身份**声明 `ext`：
    定义驱动分区（`session` / `local` / `web` / `gateway`，携带 `schema`）→ `ext = form`；
    前端自持分区（`appearance` / `about`，无 `schema`）→ `ext = 分区 id`。
  - 前端新增渲染器标识 `appearance` / `about` 与 `ext → 渲染器` 映射，
    并在 `registry/vdfsRenderers.ts` 装配 `Appearance.vue` / `About.vue`
    （`setting:appearance` / `setting:about` 的实体 editor 注册随之退场）。
  - VDFS 列表项图标复用实体机制的**项级图标**（`kind + 节点名`，与 `config_type`
    项级分发同构），设置分区图标与迁移前等价。
  - VDFS 页「返回」在虚拟根处回主界面（本页整页替换主布局，避免困住用户）。
- **S3 会话迁移**（**已完成**）：
  - **后端 `session` provider**：根 = 会话清单（`new_types = [会话]`、`root_access = l`），
    节点 = 会话（`ext = session`、`kind = session`、`status` 反映 working、`name = 会话 id`、
    `title = display_title()`——`metadata.title` 优先，否则取**最后一条用户消息**；
    `description = derive_session_summary`——**最新一条助手回复**）。`read` 返回会话
    元数据 JSON；`write` 分两支——`create` 位 → 新建（**id 由 provider 生成**，路径名
    去扩展名作标题，`created_via = "vdfs"`），否则 → 合并 `metadata` / `title`（其余字段
    明确拒绝，不静默丢弃）；`delete` 转发 `delete_session_internal`。
  - **消息（转写）不经 VDFS**：聊天流仍由既有 chat 协议承载，VDFS 只承担「资源读写」
    这一层，避免出现两套写路径。
  - **机制补缺**：`vdfs/write` 的 `create` 位此前**未透传到 `VdfsContent`**，provider
    无法区分「新建 / 覆盖」；已补 `VdfsContent.create`（`#[serde(default, skip_serializing_if)]`
    + `with_create()`）+ `protocol.rs` 三处透传 + 单测。
  - **实时**：provider 持有 `broadcast::Sender<VdfsChange>`（变更源的持有者，**非轮询**），
    `watch` spawn 转发任务、`unwatch` abort；变更点在 `vdfs/write` 的覆盖分支
    （原 `session/update` 的 `invoke_update`，该路由已于 2026-09-23 退役）
    与 `delete_session_internal`。
  - **前端**：`VdfsView` 新增 `embedded` 装配形态（左栏由应用外壳承担，本页只出中栏 + 右栏）；
    `/vdfs/:mount?` 移入 `MainLayout` 子路由；首页 `/` 重定向到 `/vdfs/session`；
    `navTargetOf('session')` 由 `/` 切到 `/vdfs/session`；旧 `session` 实体页实例（`path: ''`）
    退场。会话详情的**删除动作**经 `mechanismActions` 注入（判据 = 节点访问位 `w`）。
  - **已知取舍**：VDFS 新建会话是「立即创建 + 命名」（VDFS 的 `write { create }` 语义），
    与实体机制的「懒创建（发送首条消息才建）」不同；后者属聊天流优化，待 S5 后统一。
    （**S19 已收敛**：新建 = 选中一张草稿节点，首条消息才真正 `vdfs/write`；
    id 由 provider 生成，见 §5。）
- **S4 其余迁移**（**已完成**）：`model` / `agent` / `skill` / `mcp` 四类不再各写一份
  provider，而是由**一个通用适配器**统一接入。
  - **通用适配器** `symbio_core/vdfs/entity_adapter.rs` 的 `EntityVdfsAdapter`：
    把**任意** `EntityProvider` 接成 VDFS 子目录。核心洞见——实体机制与 VDFS 是
    **同一批资源的两套寻址方式**，因此复用既有能力而非重写：
    子目录 label / order / 可写性 / `new_types` ← `entities::provider_registry()`；
    `list` ← `list_items`；`stat` / 节点呈现 ← `summarize` + `detail_definition`；
    `read` ← 摘要 `extra.config`（与实体详情页预填**同源**）；
    `write` / `delete` ← `entity_write` / `entity_delete`；`watch` ← provider 侧
    `broadcast::Sender<VdfsChange>`。**新增一种实体类型时，VDFS 侧零改动。**
  - **消除两条链路的逻辑分叉**：把 `entities/upload` 的 manifest 分支与 `entities/delete`
    抽成公开的 `entities::entity_write` / `entity_delete`，实体机制与 VDFS 共用同一份
    校验 / 写盘 / 事件发布。
  - **节点 `ext` 选取**：`detail_definition` 有定义 → `ext = form`（前端通用表单渲染器
    解析 `schema`）；无定义 → `ext = <kind>`（机制级只读视图）。`ext` 仍是详情渲染器的
    唯一分发键，适配器不参与渲染决策。
  - **新建语义由插件自持**：新增 `EntityProvider::new_entity_manifest(id, title)` 默认方法。
    实体机制走完整表单一次上传；VDFS `write { create }` 只给路径名，故插件各给一份
    **最小可用配置**：`model` 取预设首项 + `skip_validation`；`skill` 满足
    `name == id` 且 description ≥ 10 字；`mcp` 给 stdio 骨架（`type` / `command` / `args`）。
  - **可写性双重判定**：`writable()` = 注册表 `supports_upload` **且** provider 有
    `category()` + `manifest_file()`（EntityStore 型）。目录自管型 `agent`（
    走 `AgentDirStore`）因此**自动降级为只读**——避免「声明了可新建但落盘必失败」。
    `agent` 以**只读挂载**接入：列表 + 详情可用，新建仍走实体页的 zip 上传。
  - **导航顺序单一真相源**：新增 `entities::nav_meta_of(kind)`；`session` / `setting`
    自持 provider 的 `label` / `order` 改为从注册表读（原来硬编码 10 / 60，与注册表的
    1…6 不一致，会让左栏顺序错乱）。
  - **外壳左栏切到 `<根>` 根**：新建 `composables/useNavRail.ts`
    （`navTargetOf` 恒为 `/vdfs/{mount}`、挂载清单模块级单例 + `loadMounts` / `reloadMounts`、
    订阅 `vdfs` 总线重拉）；`useEntityProviders` 只保留实体注册表职责。
  - **`form` 渲染器补删除动作**：`VdfsFormDetail` 原先假设 `form` = 设置分区（增删无语义）；
    S4 起 `model` / `skill` / `mcp` 详情也走 `form`，故按访问位注入 `mechanismActions`
    （`w` ⇒ 可删），经 `@delete` 回页面层统一走 `vdfs/delete`（与 `VdfsSessionDetail` 同构）。
  - **已知取舍**：`local`（本地文件）当时同为 VDFS 子目录，故左栏曾出现第 7 项；
    该问题的最终解法不是可见性标记，而是把物理层从 `plugins/local` 迁入
    `plugins/vdfs`（见 `plugins/vdfs/physical.rs` 模块文档与 §7.1 的 S9 记录）。
    `agent` 的新建暂不支持 VDFS 路径（zip 语义）。
- **S5 下线旧协议**（**已完成**）：
  - **第一步（已完成）**：旧专项路由重定向改指 VDFS 页——
    `model-providers` → `/vdfs/model`、`mcp` → `/vdfs/mcp`、`skill` → `/vdfs/skill`、
    `agent` → `/vdfs/agent`（原先一律指向将被下线的 `/entities/{kind}`）。
    书签 / 深链从此直达 VDFS，不再经过统一实体页。
  - **阻塞已解除（S6）**：会话的「管理内部实体」能力（工作目录树 / 子会话）
    已在 VDFS 上重建，容器页不再是不可替代的入口。详见下面的 S6 记录。
  - **页面下线（已完成）**：移除三条旧路由与整簇页面代码——
    - `/entities/:types?` 改为**保兼容重定向**（单 kind 直达 `/vdfs/{kind}`，
      `all` / 多 kind 回落 `/vdfs/session`）；
    - 容器页 `/container/:kind/:id/entities`、`/agent/:agentId/entities`
      不再保留（能力已由 S7 的 `<id>/<子类别标签>/<条目>` 寻址承担）；
    - 随之删除 `views/WorkbenchView.vue`、`composables/useWorkbenchView.ts`、
      `composables/useWorkbench.ts`、`composables/useEntityProviders.ts`、
      `components/entities/EntityTree.vue` / `EntityTreeNode.vue` /
      `EntityDetailPanel.vue` 及其单测。**前端自此只剩一台三栏工作台**
      （外壳 `MainLayout` + 页面 `VdfsView`）。
  - **连带修复**：实体操作前缀（`worker/session` 等）原先由实体页挂载时
    拉取 `entities/providers` 填充，页面下线后无人加载 → 会话清单会打到
    错误地址。改为由 `services/entities.ts` **自持幂等加载**（首个 entities
    操作前拉一次，失败可重试）。
  - **剩余（已由 S8 完成）**：`services/session.ts` 的 `listSessions()` 曾仍走
    `entities/list`（需要 `message_count` / `metadata`，当时 VDFS 节点未
    下发这些字段）；S8 已迁到 `vdfs/list` 并整体删除 `services/entities.ts`。

- **S6 会话内部在 VDFS 上重建**（**已完成**）：会话保持**叶子**（点击 = 聊天
  详情，语义不变），其内部结构作为**会话同名目录**挂在其下：

  | 地址 | 语义 |
  | --- | --- |
  | `<id>` | 会话叶子（`ext = session`，聊天详情） |
  | `<id>/subsession[/<sub>]` | 子会话清单 / 单个子会话（查看 · 删除） |
  | `<id>/workdir[/<rel>]` | 工作目录树（文件可查看 / 编辑） |

  - 入口：会话详情新增机制动作「浏览内部」（`VdfsSessionDetail` 注入，
    经 `@browse` 由页面层 `enter(node.path)` 完成）。
  - 后端：会话 `VdfsProvider` 重写 `list/stat/read/write/delete` 处理嵌套路径，
    场景实现复用既有 `workdir` 模块（两条链路同一份校验与 IO）：
    - 新增 `workdir::read_content`（另 `list_children` 去掉未用的 ctx 参数）；
    - `WorkdirWatchManager` 注入 VDFS 广播源，文件变化与机制变更**合流**，
      `<根>` 页面不另开监听；
    - `stat(<id>)` 按**目录视图**回答（只给 `l`）——`stat` 结果被分发层用作
      「当前目录节点」，其访问位决定是否给出新建入口。
  - 前端：`useVdfs.creatableTypes` 的子目录回退**仅限会话根**——否则每个
    子目录都会长出与其语义无关的新建入口（如工作目录里出现「新建会话」）。

- **S7 容器子实体在 VDFS 上重建**（**已完成**）：把 S6 的会话内部寻址**推广到
  通用适配器**——`EntityVdfsAdapter` 现在按 `container_kinds_for(kind)` 支持
  `<id>/<子类别>/<条目>` 三级寻址，agent 目录内部的提示词 / 技能 / MCP
  由此在 VDFS 上可见可编辑（原容器页的最后一处不可替代能力）。

  - 路径段用子类别**标签**（与 S6 同口径）；子实体的 `name` 是 agent 目录内
    相对路径（唯一，可含 `/`），`title` 是 basename（可读）。
  - **新建落位由 `path_hint` 决定**（`prompts/<name>.md` + 文件名 → 实际路径，
    缺省内容取 `default_content`）——路径模板仍是后端唯一真相源，前端零知识。
  - 入口复用既有 `open-container` 通道：由**详情定义**声明该动作（agent 已有
    定义），`VdfsFormDetail` 转发为 `browse`，页面层 `enter(node.path)`。
  - 前端修正：`VdfsFormDetail` 原先按 `w` 位**整表**剥掉定义动作，会把只读
    资源的「浏览内部」一并剥掉；现仅保留纯导航动作 `open-container`。
  - 通用规则：**条目有容器子实体且无详情定义 ⇒ 视为目录**（点进去浏览内部），
    有详情定义则仍是文档（详情优先 + 动作入口）。

- **S8 会话清单上 VDFS（`entities/*` 前端归零）**（**已完成**）：最后一处仍在
  调用 `entities/list` 的前端代码——`services/session.ts` 的 `listSessions()`
  ——改走 `vdfs/list`（`<根>/session`），`services/entities.ts` 整体删除。

  - **会话节点自带清单字段**：`session_node` 在 flatten 的 `attributes` 上挂
    `message_count` / `metadata` / `meta_tags`。它们是**场景数据**，VDFS 只透传；
    会话清单因此不必再为「拿 metadata」保留第二条链路。
  - `meta_tags` 由新增的 `session_meta_tags()` 产出，**实体机制与 VDFS 共用**
    （与 `display_title` / `derive_session_summary` 同口径），两条链路呈现一致。
  - 前端映射：`id` ← `name`、`name` ← `title`、`is_working` ← `status == working`。
  - `VdfsView.tagsOf` 新增渲染 `meta_tags`（后端声明、前端原样渲染，零类型知识），
    会话列表恢复「工作目录名 + 消息数」标签。
  - **前端自此不再有任何 `entities/*` 调用点**；`schemas/entities.ts` 保留
    （`DetailDefinition` 仍是 VDFS `ext = form` 的宿主方言），后端 `entities/*`
    协议与其注册表继续为 VDFS 适配器服务。

- **S9 导航可见性**（**已落地，但机制已整体撤销**）：左栏规范是「六类资源」，
  而 `local`（本地文件）当时是第 7 个子目录。S9 的做法是在机制层引入
  `VdfsProvider::nav_visible()` + 节点属性 + `VdfsMountInfo.nav_visible`
  + 前端过滤，把 `local` 藏起来。

  **该整套机制随后被删除**：物理文件层从 `plugins/local` 迁入 `plugins/vdfs`
  （见 `plugins/vdfs/physical.rs` 的模块文档），它不再是子目录、不参与 `<根>`
  的目录合成，左栏自然只剩六类资源。于是 `nav_visible`、`mount_node()`、
  `VdfsMountInfo`、`vdfs/providers` 端点、`plugins/local/vdfs.rs`
  与前端 `mountNavVisible()` 一并退场——**问题消失了，而不是被标记掩盖**。

  这是一次「先加机制、再发现机制不必存在」的记录：正确结论是
  **一个东西不该出现在左栏，就不该是子目录**，而不是给它打一个隐藏标记。

  **2026-09-15 追加（改造三之后）**：上面的结论要分两种情况读。S9 之所以能撤销机制，
  是因为 `local` 当时**本来就不必是子目录**（物理层已并入 `plugins/vdfs`）——
  「不该出现就不该是子目录」对那种情形成立。但改造三之后，`web` / `local` /
  `gateway` 三个目录**必须留在树里**：它们的全部内容就是一份配置文档，而这份文档的
  地址是**真实地址** `<根>/<插件>/PLUGIN.yml`（vdfs.md §3.4），删掉目录等于删掉
  配置的地址。「它存在、但不必出现在清单里」于是成了另一个问题，答案仍是**机制级**的：
  `VdfsNode::hidden`（与文件系统的隐藏属性同义，文件 / 目录通用：列表里不出现、
  可达性不受影响）+ `VdfsProvider::root_hidden()`（provider 的根也不过是一个目录
  节点，所以「它显示还是隐藏」由这条声明回答，容器合成该目录节点时回填）。

  它**不是** S9 那个 `nav_visible` 的复活：那套东西引入了 `VdfsMountInfo` /
  `mount_node()` / `vdfs/providers` 一整套「挂载清单」中间物，而这次没有任何新概念
  ——隐藏属性本来就该是节点的属性，过滤本来就该在产出列表的一方（容器过滤自己合成的
  子目录清单，以及任何子 provider 交回来的 `list` 结果）。另见 vdfs.md §3.2 / §13.2
  与 `docs/archive/vdfs-review.md` §7 的 S10（该评估已归档）。

  同时，设置页（`<根>/setting`）现在会列出这些配置文档——但那是**另一个目录的清单**，
  条目携带的是各自的真实地址，点开读写仍落在拥有者那份文件上：一处隐藏、一处列出，
  配置本身只有一个地址，没有重复。

- **S10 节点动作（`vdfs/action`）**（**已完成**）：S5 下线实体页时丢了一项能力——
  `model` / `mcp` 的「测试连接」仍在后端（`EntityProvider::test_status`），
  但**前端已无入口**（`VdfsFormDetail` 只接了 `@delete`，且 `capabilities.
  test_connection` 恒为 `false`）。本阶段把它补回 VDFS：

  | 层 | 改动 |
  | --- | --- |
  | 协议 | `vdfs/action`（`{ path, action, payload? }`），与 12 个既有操作同表分发 |
  | trait | `VdfsProvider::action()` 默认 `NotImplemented`；`VdfsMountTable` 只做路径转发 |
  | 适配器 | `EntityVdfsAdapter::action()`：`test` → `EntityProvider::test_status`（`connected` ⇒ `ok`），其余标识 `NotImplemented` |
  | 呈现 | 按钮仍由**详情定义**声明（`actions` + `cap.test_connection` 条件）；`VdfsFormDetail` 把「定义是否声明 test」翻译成能力位，不再硬编码 `false` |
  | 执行 | `useVdfs.runAction(id)` → `vdfs/action` → toast 结果；`testing` 忙态与 `saving` 分开 |

  - **动作 = provider 自持的动词**：VDFS 只透传 `(路径, 标识, 载荷)`，不解释语义；
    「是否支持」由 provider 回答（`NotImplemented`），「长什么样」由宿主方言决定
    ——与 `ext` 决定渲染器是同一个分层原则。
  - 只读节点仍保留 `test` 与 `open-container` 动作：二者都不依赖写权限
    （原先只读时只保留 `open-container`，会把只读资源的自检一并剥掉）。
  - 覆盖：`action_test_uses_entity_test_status`（成功 / 失败 / 未实现 / 不存在的条目 /
    非条目路径）、`action_forwards_verb_and_relative_path`（分层只转发）+ 前端
    `runVdfsAction` 三例（口径翻译、无载荷不发字段、载荷透传）。

- **S11 下线 `entities/*` 调用协议**（**已完成**）：前端（S8）与 LLM 早已零调用，
  剩下的只是**对外遗留面**——6 个插件仍把 `entities/*` 路由到
  `entities::dispatch`。本阶段拆除：

  | 位置 | 处理 |
  | --- | --- |
  | mcp / model / session / setting / skill | 删掉 `route()` 里的 `entities::dispatch` 分支 |
  | agent | 删掉 `entities/list|detail|get|upload|delete` 五个分支与 `entities_*` 三个处理函数（保留 `agent` 的导出路由） |
  | home | 删掉 `entities/providers`（资源类别改由 `<根>` 目录合成下发）及其 `provider_order_override` |
  | `symbio_core/entities.rs` | 删除 `dispatch` 与 7 个 `dispatch_*`、zip 工具、`providers_response*`；保留 `EntityProvider` trait + `entity_write` / `entity_delete`（适配器的唯一依赖） |
  | `schemas/entities.rs` | 删除 `ENTITIES_*` 路径常量与协议请求/响应（保留 `DetailDefinition`、`EntitySummary`、`EntityUploadResponse` / `EntityStatusResponse`） |
  | gateway | 只读白名单由 `entities/*` 换成 VDFS 读操作（`vdfs/list|tree|stat|read|search`） |

  - **实体机制退为内部抽象**：`EntityProvider` 仍在（它是「资源怎么存、怎么校验」的
    实现），但不再有对外地址；唯一消费者是 `EntityVdfsAdapter`。
  - **已知取舍**：随协议一并消失的两个入口，迁移前**已从 UI 不可达**（S5 下线实体页
    后就没有调用方）——① skill / agent 的 **zip 导入**（**已由 S12 以「新建类型
    `zip`」的形式回归**，见下）；② 服务器下发的**导航顺序覆盖**
    `symbio.provider_order`（顺序现由注册表 `order` 决定，不再提供配置覆盖）。
  - 净变化：核心 `entities.rs` 1467 → 634 行，协议契约 700 → 563 行。
  - **前端收尾**：`schemas/entities.ts` 删除协议时代类型（`ProviderInfo` /
    `ProvidersResponse` / `ContainerKindInfo` / `EntitiesListResponse` /
    `EntityUploadResponse` / `EntityStatusResponse` / `DetailDefinitionResponse` /
    `ENTITY_LABELS`），只留 VDFS `ext = form` 的宿主方言（`DetailDefinition` 及其
    附属形状）；`DetailForm` / `Session` 等组件里指向 `entities/*` 的注释改为
    指向 `vdfs/write` / `vdfs/delete` / `vdfs/action`。

- **S12 整包导入（zip 作为一种「新建类型」）**（**已完成**）：S5 下线实体页后
  zip 导入就没有入口，S11 删协议时又连后端实现一起删掉。回归的做法**不新增
  协议操作**——导入本就是「新建」，只是内容来自本地文件：

  | 层 | 内容 |
  | --- | --- |
  | 机制 | `VdfsNewType.source`（`VDFS_NEW_SOURCE_FILE = "file"`）：类型声明「内容取自本地文件」；`VDFS_EXT_ZIP = "zip"` 作为导入类型的扩展名 |
  | 后端 | `EntityProvider::import_zip(ctx, name, zip)` 钩子，默认实现 `entity_import_zip`（EntityStore 型通用解包，**整目录覆盖**）；agent 重写走 `AgentDirStore::import`（id 取自包内 manifest，同名替换） |
  | 适配器 | `root_new_types()` 按注册表 `supports_import` 追加 zip 类型；`write` 的**二进制分支**（`b64`）承接导入，只对挂载根下的条目有效，回 provider 给的 id 并广播变更 |
  | 前端 | `source = file` 的类型渲染**文件选择器**（而非命名输入），目标名由 `newFileNameOf(file.name, ext)` 推导；`arrayBufferToBase64` 分块编码后走既有 `writeVdfsBinary` |

  - 能力来源改为注册表显式声明：新增 `supports_import`（agent / skill / mcp 为
    true）。目录自管型 `agent` 这类类型也能导入——不必先有实体目录。
  - 顺带补上一个静默缺口：agent **没有** `delete_item` 钩子（目录自管型
    EntityStore 型），VDFS 删除 agent 目录会落到默认实现报 `NotImplemented`；
    现已重写为 `AgentDirStore::delete`。
  - 修掉一个历史缺陷：zip 解包先规范化再判隐藏文件，否则 `./a/b.txt` 会因首段
    `.` 被整条丢弃（原实现顺序相反）。

- **S13 整包导出（导入的逆动作）**（**已完成**）：导入回归后，导出是唯一还没
  有 VDFS 等价物的动作（`agent` 的导出动作因它暂留）。它与导入**共用同一
  个「整包往返」语义**，因此也不新增协议操作——做一个节点动作即可：

  | 层 | 内容 |
  | --- | --- |
  | 机制 | `VDFS_ACTION_EXPORT = "export"`：与 `test` 同为 `vdfs/action` 的动词取值 |
  | 后端 | `EntityProvider::export_zip(ctx, id)` 钩子，默认实现 `entity_export_zip`（`zip_dir` 打包整个实体目录，包内顶层目录 = id，与导入端 `strip_common_root` 配对）；agent 重写走 `AgentDirStore::export` |
  | 结果形状 | `EntityExport { id, filename, b64 }`——字段名与 `VdfsContent.b64` 同构，作为**文件载荷**随 `VdfsActionResult.data` 回传 |
  | 适配器 | `action()` 按标识分派 `action_test` / `action_export`；导出只对条目有效，先做存在性校验，不支持时透传 `NotImplemented` |
  | 声明 | agent / skill / mcp 的 `detail_definition` 各自声明 `export` 动作（`supports_import` 为真的三类） |
  | 前端 | `actionFileOf(data)` 按**形状**判定（`filename` + `b64`）而非按动作名——命中即 `downloadBlob`；`DetailForm` 未识别的动作原样 emit `action`，故新增动作前端零改动 |

  - 至此 `agent` 插件**不再有任何自有协议路由**：`route()` 直接返回 `NotFound`
    并指引到 `<根>/agent/…`，`host/handlers.rs` 整个删除。
  - 前端「动作忙态」统一：`DetailForm` 中除 `save` / `delete` 外的动作共用页面
    层的单一忙态标记（同一时刻只可能有一个动作在执行）。

- **S12 清理（历史冗余与被替换代码）**（**已完成**）：新机制稳定后，把随旧协议
  一起失去消费者的东西清掉：

  | 位置 | 处理 |
  | --- | --- |
  | `EntityProviderInfo` | 删 `prefix` / `provider_name` / `compact_list` / `status_indicator` / `capabilities`，导入能力改由 `supports_import` 表达（agent 的 `supports_upload` 随之修正为 `false`——它本就无法「最小 manifest 新建」） |
  | `EntityProvider` trait | 删无调用方的 `provider_name()` 与 `get_item()`（含 session 的重写） |
  | `schemas/entities.rs` | 删 `EntityCapabilities`（能力模型已换成访问位 + 注册表 + 声明的动作）与协议时代的请求/响应（`EntitiesList*` / `EntityUploadRequest` / `EntityGetRequest` / `EntityDeleteRequest` / `EntityStatusRequest` / `DetailDefinition*`）、`ContainerKindInfo`、`EntitySummary.provider` |
  | `agent` 旧协议路由 | 删已被 VDFS 取代的列表/读取/上传/删除/预览，只留尚无等价物的导出 |
  | 前端 | `EntityCapabilities` 收敛为表单渲染器真正消费的两项（`mutable` / `test_connection`），并注明它由渲染器按访问位自算、后端不再下发 |

- **S14 三栏唯一化（首页 = `<根>`，浏览内部 = push 页面）**（**已完成**）：
  一个 VdfsView 对应一个 vdfs 地址；三栏结构组件全 App **只有一份**。

  | 位置 | 处理 |
  | --- | --- |
  | 路由 | 首页 `/` 重定向改为 `/vdfs`（地址 `<根>` 本身，不再落到 `/vdfs/session`）；`vdfs/:mount?` 升级为 `vdfs/:dir(.*)*`（`<根>` 之下可多级深链） |
  | VdfsView | 目录变化由 `router.replace` 改为 `router.push`（每个地址 = 一个可回退的页面）；「浏览内部」/ open-container 由全窗口浮层改为 **push 进容器目录地址**；删除 `containerNode` / Teleport 装配 |
  | VdfsContainerBrowser | **整体删除**——它复制了 VdfsView 的列表 / 详情 / 新建状态机 / 图标徽标函数与几乎全部样式；push 后由同一个 VdfsView 承接，不存在第二份三栏实现 |
  | MainLayout | NavRail 头部新增**左上角返回键**（仅非首页地址页可见）：点击回上一级目录，逐级上溯到首页；导航高亮改前缀匹配（深地址仍高亮所属子目录）；去掉 `RouterView :key`（路由变化由 VdfsView 的 dirParam watch 在实例内消化，浏览器历史因此可用） |
  | useNavRail | `loadCategories` / `reloadCategories` 更名为 `loadNavDirs` / `reloadNavDirs`（`<根>` 子目录清单，无 category 概念） |
  | 测试 | 删除 `vdfsCategoryOf`（category 概念退场）的用例 |

  - **为什么删除浮层**：浮层与首页「同构」却各持一份实现（约 600 行重复），
    每次改三栏交互都要双写。push 页面后浮层的独立 `useVdfs` 实例不再必要——
    返回键即回到出发地址，外层状态天然保留（就是上一级目录的页面状态）。
  - **返回键放应用外壳**：三栏的左栏属于外壳；首页（`<根>`）无返回键，
    非首页地址页（push 出来）才有——与「一个 VdfsView 对应一个 vdfs 地址」
    的模型一致。

- **S15 三栏控件自包含（数据地址绑定；S14 的修正）**（**已完成**）：
  S14 把左栏留在外壳，导致 push 出来的地址页左栏仍是 `<根>` 子目录
  （而正确形态是**绑定地址**的子目录，如会话内部页显示 `<id>` 之下的
  子会话 / 工作目录）。三栏结构遂封装为**自包含控件**，宿主只注入宿主件：

  | 位置 | 处理 |
  | --- | --- |
  | `VdfsWorkbench.vue`（新增） | **三栏控件，唯一实现**：绑定一个**数据地址**（`addr` prop）自包含渲染——左栏 = 绑定地址的子目录导航、中栏 = 选中子目录内容、右栏 = 详情。控件不知道浏览器路由；钻入（点中栏目录 / 「浏览内部」）只 `emit('open', addr)`，呈现方式由宿主决定。`rail-header` / `rail-footer` 插槽 = 宿主件注入点 |
  | `useVdfs.ts` | 数据层绑定 `addr`（Ref）：左栏 / 选中 / 当前目录 / 详情全部由绑定地址驱动；每个地址的左栏选中项有记忆（往返 push / 返回后恢复） |
  | `VdfsView.vue` | 瘦身为**路由宿主**：浏览器地址 ↔ 数据地址换算（`/vdfs` ↔ `<根>`；`/vdfs/<dir…>` ↔ `<根>/<dir…>`，两个概念、一处换算）+ 宿主件注入——首页左下角系统目录入口、非首页左上角返回键（回 **push 来源页** = 浏览器历史 back，不是父目录；深链直开回首页）+ logo |
  | `useNavRail.ts` | **整体删除**——数据层面没有特殊 Nav 概念，外壳左栏与页面左栏是同一份获取逻辑，`useVdfs` 是唯一数据层 |
  | `MainLayout.vue` | 外壳三栏拆除：只剩 RouterView + 全局 Toast + 全局初始化（会话事件监听 / 工作区恢复）——三栏由路由页面自持 |
  | `HomedirEntry.vue`（新增） | 系统目录入口自足化（按钮 + 切换对话框 + 连接显示），谁嵌入谁拥有 |

  - **核心原则**：数据地址 ≠ 浏览器地址。控件/数据层只认数据地址
    （`<根>`、`<根>/session/<id>`），浏览器地址只是承载（甚至可以直接
    urlencode 数据地址）；两者在 `VdfsView` 一处换算。
  - **首页与内部管理页零差别**：同一个控件、同一份数据逻辑，唯一区别是
    绑定的数据地址（`<根>` vs `<根>/session/<id>` / `<根>/agent/<id>`）。

- **S16 收敛终局（废除实体机制，各插件直连 `VdfsProvider`）**（**已完成**）：
  S4 的适配器是**过渡装置**——它让「先有实体机制、再搬上 VDFS」两件事并行推进，
  代价是把每类资源的差异挤进一张 20 钩子的通用接口，再由 1500 行的适配器翻译。
  收敛期结束后整层删除：

  | 删除 | 取代者 |
  | --- | --- |
  | `EntityProvider` trait（20 钩子） | 各插件的 `impl VdfsProvider` |
  | `provider_registry()` / `EntityProviderInfo` | 各 provider 的 `label()` / `order()` / `icon()` / `root_new_types()` |
  | `EntityVdfsAdapter`（1504 行） | 无——插件直连，不需要适配器 |
  | `nav_meta_of()` | 各 provider 自持字面量 |

  - `model` / `skill` / `agent` 三个插件本次补上直连实现（`session` / `setting` /
    `mcp` 此前已直连）。**挂载名与节点形状完全不变**，故前端零改动。
  - `symbio_core/entities.rs` 降为**存储原语自由函数**（写盘 / 删除 / 导入 / 导出），
    没有 trait、没有注册表。
  - 变更广播从适配器实例提到 `vdfs/host.rs`（`notify_change` / `watch_changes` /
    `unwatch_changes`），**按 kind 全局持有**——同一 provider 每次 `traverse` 都会新构造，
    按实例持有会让订阅与投递配不上对。
  - **不变量**：`vdfs_provider.rs` 未因本次收敛做任何修改（它是核心机制）。

- **S17 收敛存储层（`storage_service` → `vdfs_service`，entity 词汇清零）**（**已完成**）：
  S16 删掉了「差异集中在一张 trait」的那一层，但**落盘那一层仍然讲着非 VDFS 的话**：
  磁盘资源由一套私有抽象（`StorageService` / `EntityStore`）表达，各插件再手翻成
  `VdfsNode` / `VdfsContent`——「一类资源 = 一份存储抽象 + 一份翻译」。本阶段把这套
  抽象换成**直接就是 `VdfsProvider` 的三个集中实现**：

  | 删除 | 取代者 |
  | --- | --- |
  | `providers/storage_service`（`StorageService` / `FileEntityStore` / `path_resolver::safe_id`，含一份从未参与编译的 `entity_store.rs`） | `providers/vdfs_service`：`SingleFileVdfs` / `DirVdfs` / `MemoryVdfs` |
  | `symbio_core::entities` 的存储原语自由函数 | `vdfs_service::entry`（寻址 + 原子落盘 + 广播）与 `vdfs_service::pack`（zip / base64）；「manifest 补 id」下沉为 model / mcp 各一份 `with_id` |
  | `symbio_core/providers/storage.rs`（trait + `categories` / `manifests` 常量） | 类别段名 = 插件名 = vdfs 子目录名（`PLUGIN_*` 常量），主文件名写在各插件内部的 `const MANIFEST` |
  | `schemas/entities.rs` 的 `EntitySummary` / `EntityUploadResponse` / `EntityExport` / `ENTITY_*` 常量 | 列表项与详情输入 = `VdfsNode`；写响应 = `VdfsWriteResponse`；导出载荷 = `VdfsPack`（线上字段与原 `EntityExport` 逐字一致） |
  | 事件总线第二条频道 `kind = "entity"`（`KIND_ENTITY` + `publish_entity_changed` / `publish_entity_status`） | 唯一一条 `kind = "vdfs"`：生命周期与运行时状态变化都经 `notify_change` 广播 |

  - **磁盘布局一个字没改**：仍是 `<homedir>/<category>/<id>/<manifest>`。
    三型只是三种**访问拓扑**（单文件不外露条目内部 / 目录可下钻 / 内存不落盘），
    换拓扑不动数据，因此**前端零改动**——地址、`ext`、`schema`、访问位全部原样。
  - **不是第二个抽象**：三个实现本身就是完整的 `impl VdfsProvider`（不是 trait、
    不是适配器、没有钩子表），差异由调用点以普通参数传入；`vdfs_provider.rs` 与
    `plugins/vdfs/*`（协议 / 访问层 / 物理层）**本次零改动**。
  - **前端连带改动**（唯一一处）：`services/eventBus.ts` 的 `KIND_ENTITY` 与实体
    生命周期 / 状态分支退场，清单与状态角标一律订阅 `kind = 'vdfs'`。
  - **明确代价**：`notify_change(kind, path)` 发**无载荷**变更（S27 起信封连操作
    枚举也没有，形状是 `{path, data?}`）。资源信号在前端**只能防抖重拉**；
    `VdfsChange` 的类型上不存在 `to` / `delta` / `node` / `content` 字段（**批次 G
    已删除**，S27 起也不以信封字段的形式回来——载荷统一收进 `data`，消息的增量与
    删除语义由 `ChatMessage` 自身承载）。
    状态角标同理：收到该节点的一次无载荷变更后重读 `vdfs/stat`。

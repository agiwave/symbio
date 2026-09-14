# 统一实体管理机制与规范

状态：规范（**后端内部机制**；`entities/*` 调用协议已于 S11 下线，S12/S13 补齐整包导入与导出）
范围：Symbio 全部「实体管理」类功能（顶层实体 + 容器子实体）

> **对外已只有一个协议（S11）**：`entities/*` 调用协议已下线——不再有任何插件
> 路由它，`entities::dispatch` 与其请求/响应一并删除。资源访问统一经 VDFS
> （`.vdfs/<子目录>/…`，子目录名 = 插件名，由注册方选定）。
>
> **本文件剩下的仍是现行机制**：`EntityProvider` trait（差异化钩子）、注册表
> （`provider_registry` / `nav_meta_of`）、`entity_write` / `entity_delete`
> （写盘与删除的唯一实现）、`entity_import_zip` / `entity_export_zip`（S12/S13）、容器子实体钩子——它们由
> VDFS 的 `EntityVdfsAdapter` 调用，是「资源怎么存、怎么校验」的实现，与对外
> 地址无关。
>
> **前端已收口（S5）**：`views/WorkbenchView.vue` 与其页面逻辑
> （`useWorkbenchView` / `useWorkbench` / `useEntityProviders`）以及
> `EntityTree` / `EntityDetailPanel` 均已删除，前端资源页只有 VDFS 一台
> （见 `docs/design/vdfs-frontend.md` §7）。文中出现的前端页面/组件名与
> `entities/*` 路由均作历史说明。
关联：`docs/design/open-agent-bundle-spec.md`（OAB 标准）、
`symbio/src/symbio_core/schemas/entities.rs`（协议权威定义）、
`symbio/src/symbio_core/entities.rs`（注册表与统一分发）

> 本文件只写**规范与机制**。具体实体（会话、模型、设置分区……）如何应用
> 机制一律属于**范例**（§6），不是机制的组成部分；实例的新增/下线/调整
> 不影响本规范的效力，机制演进也不以任何单个实例为准。

---

## 1. 目标与不变量

一套协议、一套页面机制、一处注册——**新增一类可管理实体时，只需**：

1. 后端实现统一实体协议（`entities/*`）并在注册表登记一条 `EntityProviderInfo`；
2. 前端**默认零改动**：导航/列表/新建/删除/状态全部由机制自动生成；
   详情按能力递进自动生成（§3.2 解析链），仅当详情属**复杂形态**时
   才需要补一个专属 editor 注册。

不变量：

- **后端是单一真相源**：类型的存在性、顺序、标签、能力开关、容器子类别
  清单、详情页定义，均由后端下发；前端不做任何硬编码枚举。
- **前端是纯消费者**：前端只允许补充「UI 映射」（图标、复杂详情 editor
  组件），存放于 `registry/entityTypes.ts`，与数据契约严格分离。
- **协议形状统一**：无论顶层还是容器语义，请求/响应一律是同一组 schema。

## 2. 机制契约（**已下线对外协议** `entities/*`；内部形状仍然有效）

> 本节描述的**调用协议已于 S11 下线**：`<prefix>/entities/*` 不再有任何路由，
> 其请求/响应结构（`EntitiesList*` / `EntityGetRequest` / `EntityUploadRequest`
> / `EntityDeleteRequest` / `EntityStatusRequest` / `DetailDefinition*`）已从
> `schemas/entities.rs` 删除。保留本节是为了说明**机制内部形状**的来历：
> 列表项仍是 `EntitySummary`，写盘结果仍是 `EntityUploadResponse`，状态仍是
> `EntityStatusResponse`——它们由 `EntityVdfsAdapter` 直接使用，只是不再经协议。

### 2.1 注册表 `provider_registry()`

每条 `EntityProviderInfo`（宿主级单一真相源，子目录的标签 / 顺序 / 能力由此派生）：

| 字段 | 语义 |
|---|---|
| `kind` / `label` / `order` | 类型标识（同时是 `.vdfs` 子目录名）/ 展示标签 / 展示顺序（导航排序权威；**无配置覆盖**） |
| `supports_upload` | 能否以「最小 manifest」新建（一次 `vdfs/write { create }`） |
| `supports_import`（S12） | 能否**整包导入**（zip）；目录自管的类型（agent bundle）也可为 true |
| `container_kinds` | **容器声明**（见 §2.3；空 = 条目不是容器） |

> 已删除的字段：`prefix`（协议路径前缀）、`provider_name`、`capabilities`
> （`EntityCapabilities`）、`compact_list`、`status_indicator`——前三者随协议
> 下线，后两者是已删除的实体页的列表行为开关（S12 清理）。

### 2.2 统一操作（现由 VDFS 承担）

| 原端点 | 现在的路径 |
|---|---|
| `entities/list` | `vdfs/list`（`.vdfs/<kind>`）→ `EntityProvider::list_items` |
| `entities/get` | `vdfs/read` → 摘要 `extra.config`（钩子 `get_item` 已随 S12 删除） |
| `entities/upload`（manifest） | `vdfs/write` → `entity_write` |
| `entities/upload`（zip） | **S12 起是「新建类型 `zip`」**：`vdfs/write`（二进制）→ `EntityProvider::import_zip` |
| `entities/delete` | `vdfs/delete` → `entity_delete` |
| `entities/status` | `vdfs/action { action: "test" }` → `EntityProvider::test_status` |
| `agent/bundle/export` | `vdfs/action { action: "export" }`（S13）→ `EntityProvider::export_zip` |
| `entities/detail` | 列表节点自带 `schema`（详情定义随列表下发） |
| `entities/watch` / `unwatch` | `vdfs/watch` / `vdfs/unwatch`（容器子实体经 `watch_container`） |

- 列表项一律是 `EntitySummary`：`kind / id / name / status / summary` +
  `extra`（类型特有字段 flatten）。
- 写入二选一：`manifest`（表单型，JSON）或**整包**（zip 字节，二进制通道）。

### 2.3 容器语义（container）

**当 provider 声明了 `container_kinds` 时，其每个条目（item）本身是一个容器**，
内部托管声明的子实体类型。容器语义 = 在统一操作请求上**多带一个可选字段
`container`（容器条目 id）**：

| 操作 | 顶层语义 | 容器语义（payload 带 `container`） |
|---|---|---|
| `list` | 顶层实体列表 | 容器内子实体列表；`sub_kind` 可选过滤；条目 `kind` 区分子类型 |
| `get` | 实体详情 | 读子实体；`id` 为容器内相对路径；内容在 `extra.content` |
| `upload` | zip / manifest 创建 | 写子实体文件；`name` 为容器内相对路径，内容取 `manifest.content` |
| `delete` | 删除实体（走 `delete_item` 钩子） | 删子实体文件；`id` 为容器内相对路径 |

`ContainerKindInfo { kind, label, description?, path_hint?, default_content?, capabilities, view? }`
逐子类别下发：标签、语义说明、**新建路径模板**（`<name>` 占位符）、
**新建内容模板**、能力开关、**中栏结构形态**。前端新建表单完全由此驱动。

约束：

- **`path_hint` 为空 = 系统管理型子类别**：不可由用户创建（前端隐藏新建
  入口），仅支持查看/删除——子实体由系统派生（如会话的子会话）。
- **中栏结构形态（list / tree，机制级）**：`view` 缺省 = 列表；
  `tree` = 树视图。二者只是中栏的两种**结构机制**，不含任何场景语义：
  - 树节点 = 统一 `EntitySummary`：`id` = 容器内相对路径、`parent` =
    父路径（根层缺省）、`expandable` = 可展开提示；
  - **懒加载**：树子类别经 `entities/list` 的 `parent` 请求参数逐层下发
    （`parent` 缺省 = 根层），数据源仍是 provider 的 container 钩子
    （`list_container_items` 的 `parent` 参数）；
  - 通用渲染件 `EntityTree` / `EntityTreeNode`（机制内置，唯一实现）：
    节点图标经 entity 注册表按 kind 映射（provider 场景注册），选择上抛
    后沿用容器页既有详情流（content 分流 / 能力门控）；
  - 文件系统目录树只是 tree 机制下的**一个场景实现**（如会话的工作目录
    浏览，`kind = "dir"`），机制本身不感知文件语义。
- **子实体详情分流**：`entities/get` 响应带 `extra.content`（内容型）→
  标准文本编辑器（写回按子类别能力门控：`mutable && !read_only`）；
  不带 → 只读详情面板。
- 容器子实体的路径合法性由**后端**校验（路径白名单，拒绝穿越），
  前端模板仅用于展示。
- 容器列表响应的 `capabilities` 取该子类别的声明值；响应回显 `container`。

**统一动作区**（页面机制，机制动作的唯一定义点与渲染约定）：

- **机制动作** = `open-container`（条目所在 provider 声明 `container_kinds`
  时注入，路由推入 `/container/:kind/:id/entities`）| `test`（能力
  `test_connection`）| `delete`（能力 `mutable`）。页面按能力/状态计算
  动作集，**排除当前详情定义已声明的动作**（定义动作优先，机制不重复注入）。
- **渲染一律在详情页内部**（详情页只有自定义 / 机制化两种形态，页面不得
  另加动作外框），经 `mechanism-actions` prop 注入：
  - DetailForm（定义驱动）：与定义动作同排渲染于表头动作区；
  - 自定义 editor：在自身动作区并排渲染（如会话聊天头部按钮行）；
  - 通用只读兜底（EntityDetailPanel）：在面板头部渲染。
- **图标优先的动作风格**：动作按 `icon` 字段（缺省按语义 id）映射为图标
  按钮（tooltip = 动作名，进行中 = `busy_label`）；无图标映射的动作回落
  为文字按钮。图标 SVG 映射是前端唯一持有的部分（`registry/entityTypes.ts`，
  纯 UI 资产）。

### 2.4 状态与事件约定（机制级，与具体实体无关）

- **状态取值**：`status ∈ active | working | disabled | error | unknown`
  （列表摘要与状态事件同用此约定）。`working` 表示实体正在工作，
  列表以独立视觉态渲染（脉冲点 + 「工作中」角标），不得折叠为其他取值。
- **事件分级**：`entities/status` 触发**状态事件**（列表项即时补丁）；
  provider 发出的 bus 事件分**粗粒度**（`type == "status" | "title" | "data"`，
  语义级变化；`data` = 容器子实体数据变更，载荷含场景定位信息如
  `workdir`/`path`，机制只消费「变更已发生」语义）与**细粒度**（如流式
  数据帧）。机制列表与树视图只对状态事件与粗粒度事件做出反应，
  细粒度事件**不得**触发刷新。
- **容器数据订阅**：`entities/watch` / `entities/unwatch` 操作对（载荷
  复用 `container` + `sub_kind`）——实时视图（树）在**挂载期**订阅、
  卸载时取消，监听生命周期与视图严格绑定；provider 默认 no-op，
  实时场景实现引用计数（同一容器数据多方共享监听，最后一个订阅方
  释放后停止）。**释放为世代守卫的宽限延迟**（迟到的 unwatch 因世代
  推进而作废，不误杀重新建立的订阅）。前端订阅为 kind 级、handler 内
  按容器过滤——**不得**携带 sessionId 订阅（会触发会话回放缓冲的
  drain，破坏聊天流式恢复）。

## 3. 前端机制分层

| 层 | 位置 | 职责 |
|---|---|---|
| 注册表 store | `composables/useEntityProviders.ts` | 拉取并缓存 `entities/providers`；标签/能力查询；应用外壳导航（`useNavRailItems` + `navTargetOf`） |
| 服务层 | `services/entities.ts` | 协议函数（`listEntities / getEntity / putContainerEntity / deleteEntity / uploadEntityZip / uploadEntityForm / getEntityStatus / getDetailDefinition`），带可选 `container` |
| 三栏容器 | `components/common/Workbench.vue` + `useWorkbench.ts` | 「侧边栏 + 列表 + 详情」容器与状态机唯一实现（应用外壳模式 / 实体页模式） |
| **统一实体页** | `views/WorkbenchView.vue` + `composables/useWorkbenchView.ts` | **全 App 唯一实体页面**（entity / container 双模式，见 §3.1）；页面与 ts 均为标准复用件 |
| 应用外壳 | `views/MainLayout.vue` | 纯壳：Workbench（rail 来自 `useNavRailItems`）+ RouterView + 应用级服务，不持有实体逻辑 |
| 详情 editor 注册表 | `registry/entityTypes.ts` | 仅为**复杂详情**注册 kind 级 / 项级 editor 组件（§3.2 解析链第一优先） |
| 通用详情渲染器 | `components/entities/DetailForm.vue` | 定义驱动的详情/新建表单唯一实现（§3.2） |
| 树视图渲染件 | `components/entities/EntityTree.vue` | 中栏 tree 形态唯一实现：层级 + 懒加载 + 选择（§2.3），不含场景语义 |
| 动作渲染件 | `components/entities/EntityActions.vue` | 机制/定义动作统一按钮渲染（图标优先，§2.3 统一动作区） |
| 图标注册表 | `registry/entityTypes.ts` | kind → SVG 图标（含容器子类别与树节点）+ 动作图标映射 |

### 3.1 WorkbenchView 的两种模式（同一组件、同一组合式）

页面形态由**路由参数自动判定**，实例内恒定；两模式唯一差异点：
① 类别来源；② list/delete 是否携带 `container` 字段。

| | leaf 模式 | container 模式 |
|---|---|---|
| 路由 | `/entities/:types?`、`/settings` | `/container/:kind/:id/entities`、`/agent/:agentId/entities` |
| 侧边栏 | 由 MainLayout 应用外壳承担 | 自带 = `ProviderInfo.container_kinds`（子类别 + 计数 + 返回键） |
| 类别来源 | providers 注册表解析 `:types`（`resolveActiveTypes`） | `container_kinds`（标签/路径模板/内容模板/能力/结构形态后端下发） |
| 列表 | 各类别 `entities/list` 并行，混合平排 | `entities/list`（payload.container 单请求全量，核心按 kind 分箱） |
| 中栏形态 | 列表（tree 形态按 provider 声明同形扩展） | **列表或树**（子类别 `view` 声明；tree = `EntityTree` 懒加载） |
| 详情 | 编辑器解析链（§3.2） | content 分流：文本编辑器（能力门控写回）/ 只读面板 |
| 新建 | 类型选择 → 解析链（§3.2）/ zip / JSON（能力驱动分流） | 名称 + 内容（路径模板 `path_hint` + 内容模板 `default_content`） |

新建分流细则：当某 kind 既有详情定义又具备 `zip_upload` 能力时，
新建态 = 定义表单为主 + 「或上传 ZIP」次级入口（完整目录包不丢能力）。

### 3.2 详情页生成：解析链与定义协议

**编辑器解析链**（entity 详情/新建统一适用，按序第一个命中者生效）：

1. **注册专属 editor**（`registry/entityTypes.ts`）——仅限**复杂详情**；
2. **定义驱动**（后端 `entities/detail` 返回非空 `DetailDefinition`
   → `DetailForm.vue` 通用渲染器动态生成）；
3. **通用兜底**（能力分流：zip 上传面板 / JSON manifest 表单 / 只读详情）。

**复杂详情判定标准**（满足其一才允许注册 editor，否则必须走定义或兜底）：
详情需要持久前端状态（如聊天工作区的会话选中）、或即时生效型前端 store
交互（如 appearance）。**只读概览型**（如 bundle 概览）由 `info` 绑定表达，
**不算**复杂详情；除此之外的表单类详情一律由定义表达。

**定义协议**（`entities/detail`）：请求 `{kind, id}`，`id` 为空 = 请求
「新建态」定义；trait 默认 `detail_definition() -> None`（无定义 → 解析链
下探）。定义 `DetailDefinition`（权威 schema 见 `schemas/entities.rs`）
必须能完整表达一个交互不复杂的表单详情：

- **绑定** `binding`：`upload`（实体实体：预填 `item.config`，保存 emit
  save → `entities/upload` manifest）| `config`（配置分区：`load_path`/
  `save_path` 走目标插件的标准 config 协议路由（`CONFIG_GET`/`CONFIG_SET`
  = `config/get` / `config/set`，渲染器自持保存）| `info`（只读
  概览：无保存，`static` 字段取值来自 item 顶层（extra flatten），
  动作仅限 `open-container`/`delete` 等机制通道动作）；
- **分区/折叠** `sections[]`：`collapsed` = 默认折叠；
- **控件全集** `widget ∈ text | password | number | select | textarea |
  toggle | datalist | list | map | static`（password 显隐、number
  min/max/step、textarea rows/full_width、datalist 动态建议；`list` =
  字符串数组（编辑态每行一项）、`map` = 键值对（编辑态每行 `KEY=VALUE`），
  序列化约定与后端 `validate_manifest` 两侧一致；`static` 只读展示，
  `options` 可作值→标签映射）；
- **字段条件显隐** `visible_when: DetailCondition`（不满足时整行不渲染
  且**不参与保存**，用于互斥字段组，如 transport 专属字段）；
- **预设联动** `presets{field, fill, presets[]}`：选中预设后按 `fill`
  策略（`if_empty`/`always`）填充 `set`、**总是**应用 `set_always`、并把
  `options[key]` 注入对应字段的动态候选；
- **条件徽标/动作** `DetailCondition{key, equals, not_equals, truthy, all}`
  （键 = 表单字段 / `is_existing` / `is_default` / `cap.<name>`）；
  动作 `id` ∈ `save | test | delete | set-default | open-container`
  （`open-container` 经 `payload.kind` 路由推入容器实体页）；`icon` 可选
  （缺省按 id 的默认图标映射渲染图标按钮，无映射回落文字按钮）；
- **派生回落链**：`title_from`（标题）、`subtitle_from`（副标题，select
  值自动映射选项标签）、`name_from`（保存补名）、`id_from`（新建 slug +
  `-2` 去重；后端 `validate_manifest` 为最终兜底）。

### 3.3 实时列表（机制能力）

- ① **状态事件** → 列表项状态角标即时补丁（不重拉清单）；
- ② **粗粒度 bus 事件**（§2.4）→ 防抖（800ms）刷新该类清单；
- 禁止轮询、禁止私有刷新通道；细粒度事件不得触发刷新。
  后端任何语义级变化（新建/删除/更名/运行状态）自动反映到列表。
- `capabilities.refreshable == false` 的类型（如 session）清单由
  ①② 事件通道自持同步，**不提供手动刷新**；刷新按钮仅服务于
  「清单可能被外部修改」的类型（model/mcp/skill/agent 等）。

### 3.4 列表头动作（能力驱动）

列表头上方的动作按钮（新建 / 刷新）**全部由后端能力决定**，
前端不得按 kind 硬编码其显隐：

- **新建**：实体模式要求 `creatableInActive` 非空（manager 通道
  `supports_upload && mutable`，或 editor 引导通道 `independent_form` +
  kind 级 editor 注册）；容器模式要求当前子类别声明了 `path_hint`
  （为空 = 系统管理型，如子会话由机制派生，无新建入口）。
  setting（`mutable=false` 且无 kind 级 editor）两通道皆不满足 → 不显示。
- **刷新**：实体模式要求所有活动类型均声明 `refreshable: true`
  （任一类型清单由事件通道自持同步或固定即不显示）；容器模式看当前
  子类别能力。树/条目清单（可浏览外部修改）保留手动刷新。

路由（机制实例，非机制组成部分）：

- `/entities/:types?` —— 顶层统一实体页（entity）；
- `/settings` —— 同一 WorkbenchView 的 setting 实例；
- `/container/:kind/:id/entities` —— 通用容器实体页（container）；
- `/agent/:agentId/entities` —— agent 兼容别名。

## 4. 扩展指引（新增一类可管理资源）

**后端**：

1. 实现 `EntityProvider` trait（`kind`、按需重写 `list_items` /
   `delete_item` / `import_zip` / `test_status`；若是 EntityStore 目录型资源，
   实现 `category()` + `manifest_file()` + `summarize()` 即可走默认流程）；
2. 在 `provider_registry()` 登记一条 `EntityProviderInfo`
   （`kind` / `order` / `label` / `supports_upload` / `supports_import` /
   `container_kinds`）——**插件 route 不再需要接任何分发**（`entities/*` 已下线），
   VDFS 侧由 `EntityVdfsAdapter` 自动为它生成 `.vdfs/<kind>` 子目录；
3. （可选）**整包导入 / 导出**：`supports_import = true` + 按需重写
   `import_zip` / `export_zip`（目录自管的类型必须重写；EntityStore 型走默认的
   通用解包 / 打包，二者互为逆向），并在 `detail_definition` 里声明 `export` 动作；
4. （可选）条目是容器：登记 `container_kinds` 并实现四个 `*_container_item` 钩子；
5. （可选，**详情默认路径**）重写 `detail_definition` 钩子下发
   `DetailDefinition`（§3.2），前端零页面/零 ts 开发。

**前端**：**零改动**——子目录、导航、列表、新建 / 导入 / 删除、实时性全部由
VDFS 机制生成；仅当详情属「复杂形态」才补一个专属 editor 与图标注册。

## 5. 一致性要求

- 任何新的实体管理功能 **不得** 绕开本机制新造私有协议。
- 实体管理页面 **不得** 新增第二个 Vue 视图或页面级组合式：全 App 实体
  页面唯一为 `WorkbenchView.vue`（逻辑唯一在 `useWorkbenchView.ts`），
  MainLayout 只是应用外壳。
- 实体列表 **不得** 自建轮询或私有刷新通道：实时性统一走 §3.3 的事件
  订阅；实体删除统一走 `entities/delete`（非实体存储型 provider 重写
  `delete_item` 钩子），不得绕开机制自持删除。
- 详情 editor 注册 **不得** 违反 §3.2 解析链与复杂详情判定标准：
  可由定义表达的表单详情注册专属 editor 属于违规实现。
- 前端 **不得** 硬编码实体类型清单、类别标签、路径模板、能力开关、
  列表头动作显隐（新建/刷新由 `creatableInActive`/`path_hint`/
  `refreshable` 能力判定，见 §3.4）、预设数据；只允许注册图标与
  复杂详情 editor 这类纯 UI 映射。
- 请求/响应结构变更必须先改 `symbio_core/schemas/entities.rs` 与
  `tauri/src/schemas/entities.ts` 两侧契约，再改实现。

## 6. 范例（实例，非机制组成部分）

以下实例仅演示机制的应用方式；它们的增删改不改变本规范。

### 6.1 会话（session）——「非实体存储 + editor 引导型创建」

- **列表** = 机制标准列表（摘要含实时 working 状态）；清单由生命周期
  事件通道自持同步，列表头**无刷新按钮**（`refreshable=false`，§3.4）；
- **详情** = kind 级注册的复杂详情 editor（聊天工作区）：机制选中是唯一
  真相（`:key` 重挂载 + `watch item.id → store.selectSession`）；
- **新建** = editor 引导通道：`independent_form` + kind 级 editor 注册 →
  「新建」渲染引导态；单类型页 + 清单为空 + 可创建 → 自动进入新建态；
  editor `emit('created', id)` 后页面刷新清单并选中之；
- **删除** = 后端重写 `delete_item`（先 abort 活跃任务再删）；前端删除
  能力 = `capabilities.mutable && !read_only`（与 `supports_upload` 解耦）；
- **显示名** = 后端 `Session::display_title` 单一来源（title 优先 → 首条
  用户文本消息自动生成 → 「新对话」），落盘后持久化并发 `title` 粗粒度事件。

### 6.2 模型（model）——定义驱动的复杂度上限

详情/新建由后端 `entities/detail` 的 `DetailDefinition` 完整表达
（upload 绑定）：供应商预设联动（`set`/`set_always`/动态候选）、模型
datalist、API Key 显隐、高级设置折叠分区、条件徽标（默认/已停用）与
五类动作。预设数据由后端单一真相源下发。

### 6.3 设置分区（setting）——config 绑定

session/local/web 三分区由后端下发 config 绑定定义（`load_path`/
`save_path` = 各插件标准 `config/get|set` 路由），DetailForm 渲染并自持保存。
appearance（前端 store 即时生效）按 §3.2 判定标准保留注册 editor；
about（纯信息展示）亦保留。
分区清单**固定**：列表头**无新建/刷新按钮**（`mutable=false` 且
`refreshable=false`，§3.4）。

### 6.4 MCP / Skill——upload 绑定 + 结构化控件

- **mcp**：详情/新建由后端下发定义（`server.json` 同构 manifest）。
  传输类型 select 联动 `visible_when` 字段显隐（stdio → command/args/env；
  http/sse → url/headers/timeout），`list`/`map` 控件承载 args/env/headers/
  工具过滤；`validate_manifest` 做 transport 必填校验并规范化为
  server.json；`summarize` 下发 `extra.config` 供预填。zip 上传保留
  （新建表单次级入口）。
- **skill**：详情/新建由后端下发定义，表单 ↔ SKILL.md（YAML frontmatter
  + Markdown body）双向映射（`validate_manifest` 生成 / `summarize`
  解析 `extra.config`）。BUG-SR6（名称 == 目录 id）与 BUG-SR7
  （description ≥ 10 字符）在保存时即校验。zip 上传保留（含脚本/参考
  文件的完整技能包）。

### 6.5 agent（OAB bundle）——容器语义 + info 概览

条目即容器：`container_kinds` 声明 prompt/skill/mcp 子类别（标签/路径
模板/内容模板/能力），容器实体页与子实体管理完全由后端声明驱动。agent
详情由后端下发 **info 绑定**定义（只读概览：版本/来源层级/安装目录/
内部实体计数（list_items 下发 `count_*`））。「管理内部实体」为机制动作
（§2.3 统一动作区），与删除同排渲染；新建态无定义，保留 zip 上传流程。

### 6.6 会话（session）——子会话容器语义 + 目录树场景

- **存储**：子会话不是顶层平级实体，而是存放在父会话目录内
  `<父>/sessions/<子>/`；归属由 `metadata.parent_session_id` 声明，
  文件后端据此路由（save 路由 / load·delete 回退查找，调用方无感知）；
  `list_sessions` 只列顶层，删除父会话级联删除子会话。
- **容器声明**：`SESSION_CONTAINER_KINDS` 声明两个子类别——
  「子会话」（列表视图，系统管理型，仅查看/删除）与「目录树」
  （tree 机制的场景实现：会话工作目录的层级浏览，`view = "tree"` +
  懒加载，文件可查看/编辑，能力 `BUNDLE_FILE` 门控写回与删除）。
  provider 按 `sub_kind` 分流 list/get/put/delete 容器钩子，经统一协议
  `entities/*` + `container` 字段访问。
- **实时性**：目录树场景实现 `watch_container`/`unwatch_container`——
  树视图挂载期订阅会话工作目录监听（引用计数；不同会话各自 workdir
  各自监听，共享 workdir 共享监听），文件变化经粗粒度 `data` 事件
  （kind = session、sessionId = 会话 id）驱动树视图与详情编辑器防抖重载。
- **前端**：会话详情（聊天工作区）经机制动作「管理内部实体」push 进
  `/container/session/:id/entities`，侧边栏 = 两个子类别（后端声明）；
  目录树子类别中栏渲染 `EntityTree`（懒加载 + 实时刷新），树节点图标按
  `config_type`（directory/file）项级分发；树节点（文件）点击后内容经
  既有 content 分流查看/编辑；删除同为机制动作。

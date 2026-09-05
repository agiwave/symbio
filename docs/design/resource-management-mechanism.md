# 统一资源管理机制与规范

状态：现行规范
范围：Symbio 全部「资源管理」类功能（顶层资源 + 容器子资源）
关联：`docs/design/open-agent-bundle-spec.md`（OAB 标准）、
`symbio/src/symbio_core/schemas/resources.rs`（协议权威定义）、
`symbio/src/symbio_core/resources.rs`（注册表与统一分发）

---

## 1. 目标与不变量

一套协议、一套页面机制、一处注册——**新增一类可管理资源时，只需**：

1. 后端实现统一资源协议（`resources/*`）并在注册表登记一条 `ResourceProviderInfo`；
2. 前端补缺失的**详情页**（经 editor 注册表注册）。

除此之外（导航、列表、容器页、新建/删除/状态等）全部机制化，前端零改动。

不变量：

- **后端是单一真相源**：类型的存在性、顺序、标签、能力开关、图标语义层级、
  容器子类别清单，均由后端注册表下发；前端不做任何硬编码枚举。
- **前端是纯消费者**：前端只补充「UI 映射」（图标、详情 editor 组件），
  存放于 `registry/resourceTypes.ts`，与数据契约严格分离。
- **协议形状统一**：无论顶层还是容器语义，请求/响应一律是同一组 schema。

## 2. 协议契约（`resources/*`）

### 2.1 注册表端点 `resources/providers`

返回 `ProvidersResponse { providers: ProviderInfo[] }`。每个 `ProviderInfo`：

| 字段 | 语义 |
|---|---|
| `kind` / `label` / `order` | 类型标识 / 展示标签 / 展示顺序（导航排序权威） |
| `prefix` | 资源操作路径前缀（前端拼 `${prefix}/resources/<op>`） |
| `capabilities` | 能力开关（zip 上传 / 独立表单 / 实时状态 / 可写 / 连接测试 / 只读） |
| `supports_upload` / `compact_list` / `status_indicator` | 列表页行为开关 |
| `container_kinds` | **容器声明**（见 §2.3；空 = 条目不是容器） |

### 2.2 五个统一操作

```
<prefix>/resources/list     → ResourcesListResponse { kind, capabilities, items, container? }
<prefix>/resources/get      → ResourceSummary（容器语义时 extra.content 为文件内容）
<prefix>/resources/upload   → ResourceUploadResponse { kind, id, created }
<prefix>/resources/delete   → ResourceUploadResponse（同形状）
<prefix>/resources/status   → ResourceStatusResponse（可选能力）
```

- 列表项一律是 `ResourceSummary`：`kind / id / name / status / summary` +
  `extra`（类型特有字段 flatten）。
- 上传二选一：`zip_b64`（目录型资源）或 `manifest`（表单型资源）。

### 2.3 容器语义（container）

**当 provider 声明了 `container_kinds` 时，其每个条目（item）本身是一个容器**，
内部托管声明的子资源类型（如 agent bundle 内部的 prompt / skill / mcp）。
容器语义 = 在统一操作请求上**多带一个可选字段 `container`（容器条目 id）**：

| 操作 | 顶层语义 | 容器语义（payload 带 `container`） |
|---|---|---|
| `list` | 顶层资源列表 | 容器内子资源列表；`sub_kind` 可选过滤；条目 `kind` 区分子类型 |
| `get` | 资源详情 | 读子资源；`id` 为容器内相对路径；内容在 `extra.content` |
| `upload` | zip / manifest 创建 | 写子资源文件；`name` 为容器内相对路径，内容取 `manifest.content` |
| `delete` | 删除资源（走 `delete_item` 钩子；EntityStore 型默认实现，非实体存储型 provider 重写，如 session 先 abort 再删） | 删子资源文件；`id` 为容器内相对路径 |

`ContainerKindInfo { kind, label, description?, path_hint?, default_content?, capabilities }`
逐子类别下发：标签、语义说明、**新建路径模板**（`<name>` 占位符，如
`prompts/<name>.md`）、**新建内容模板**、能力开关。前端新建表单完全由此驱动。

约束：

- 容器子资源的路径合法性由**后端**校验（如 agent 插件的
  `classify_resource_path` 白名单，拒绝穿越），前端模板仅用于展示。
- 容器列表响应的 `capabilities` 取该子类别的声明值；响应回显 `container`。

## 3. 前端机制分层

| 层 | 位置 | 职责 |
|---|---|---|
| 注册表 store | `composables/useResourceProviders.ts` | 拉取并缓存 `resources/providers`；标签/能力查询；应用外壳导航（`useNavRailItems` + `navTargetOf`） |
| 服务层 | `services/resources.ts` | 协议函数（`listResources / getResource / putContainerResource / deleteResource / uploadResourceZip / uploadResourceForm / getResourceStatus`），带可选 `container` |
| 三栏容器 | `components/common/Workbench.vue` + `useWorkbench.ts` | 「侧边栏 + 列表 + 详情」容器与状态机唯一实现（应用外壳模式 / 资源页模式） |
| **统一资源页** | `views/WorkbenchView.vue` + `composables/useWorkbenchView.ts` | **全 App 唯一资源页面**（entity / container 双模式，见 §3.1）；页面与 ts 均为标准复用件 |
| 应用外壳 | `views/MainLayout.vue` | 纯壳：Workbench（rail 来自 `useNavRailItems`）+ RouterView + 应用级服务，不持有资源逻辑 |
| 详情 editor 注册表 | `registry/resourceTypes.ts` | kind 级（`'model'`）与项级（`'agent:bundle'`，按 `item.config_type` 分发）注册详情组件；未注册走通用兜底 |
| 图标注册表 | `registry/resourceTypes.ts` | kind → SVG 图标（含容器子类别，如 prompt） |

### 3.1 WorkbenchView 的两种模式（同一组件、同一组合式）

页面形态由**路由参数自动判定**，实例内恒定；两模式唯一差异点：
① 类别来源；② list/delete 是否携带 `container` 字段。

| | entity 模式 | container 模式 |
|---|---|---|
| 路由 | `/resources/:types?`、`/settings` | `/container/:kind/:id/resources`、`/agent/:agentId/resources` |
| 侧边栏 | 由 MainLayout 应用外壳承担 | 自带 = `ProviderInfo.container_kinds`（子类别 + 计数 + 返回键） |
| 类别来源 | providers 注册表解析 `:types`（`resolveActiveTypes`） | `container_kinds`（标签/路径模板/内容模板/能力后端下发） |
| 列表 | 各类别 `resources/list` 并行，混合平排 | `resources/list`（payload.container 单请求全量，核心按 kind 分箱） |
| 详情 | editor 注册表分发 → 通用兜底（zip 面板 / JSON 表单 / 只读详情） | 标准文本编辑器（`resources/get` extra.content + `resources/put` 写回） |
| 新建 | 类型选择 → 专属 editor / zip / JSON（能力驱动分流） | 名称 + 内容（路径模板 `path_hint` + 内容模板 `default_content`） |

容器条目详情组件（如 Agent.vue）只做**只读概览**（`useContainerOverview`：
类别 + 计数）；管理逻辑一律在 WorkbenchView 页面侧，不得在详情组件内重复。

### 3.2 会话（session）如何纳入机制

session 是「非实体存储 + editor 引导型创建」的资源范例，展示机制的完整弹性：

- **列表** = 机制标准列表（`resources/list`，后端摘要含实时 `is_working` 状态）；
- **详情** = kind 级注册的专属 editor（`Session.vue`：聊天工作区
  ChatMainPanel + SessionExplorerPanel）；机制选中（`:key` 重挂载）是唯一真相，
  editor 内 `watch item.id → store.selectSession`；
- **新建** = editor 引导通道：`capabilities.independent_form` + kind 级 editor
  注册 → 「新建」按钮渲染 editor 引导态（`item=null`）；**单类型页 + 清单为空 +
  可创建 → 自动进入新建态**（session 首启即显示「新建会话」引导）；
  editor 创建成功后 `emit('created', id)`，页面刷新清单并选中之；
- **删除** = 后端重写 `ResourceProvider::delete_item`（先 abort 活跃任务再删），
  前端 `capabilities.mutable && !read_only` 即出现删除按钮，走统一
  `resources/delete`；删除能力与 `supports_upload`（上传通道）解耦判定；
- **实时列表（机制能力）**：① resource 状态事件 → 列表项状态角标即时补丁；
  ② 该 kind 的任意 bus 事件 → 防抖（800ms）刷新该类清单。后端任何变化
  （会话新建/删除/标题变更、运行状态）自动反映到列表，前端零轮询。

路由：

- `/resources/:types?` —— 顶层统一资源页（entity）；
- `/settings` —— 同一 WorkbenchView 的 setting 实例（分区清单来自后端 setting/resources/list）；
- `/container/:kind/:id/resources` —— 通用容器资源页（container）；
- `/agent/:agentId/resources` —— agent 兼容别名（同组件、props 固定 `containerKind: 'agent'`）。

## 4. 扩展指引（新增一类可管理资源）

**后端**：

1. 实现 `ResourceProvider` trait（`kind`、按需重写 `list_items` /
   `upload` / `delete` / `test_status`；若是 EntityStore 目录型资源，
   实现 `category()` + `manifest_file()` + `summarize()` 即可走默认流程）；
2. 在插件 route 顶部接入 `crate::symbio_core::resources::dispatch`；
3. 在 `provider_registry()` 登记一条 `ResourceProviderInfo`
   （kind / prefix / capabilities / order / label / supports_upload）；
4. （可选）条目是容器：登记 `container_kinds` 并实现四个 `*_container_item` 钩子。

**前端**：

1. 若详情不是通用兜底形态：写详情组件并在 `registry/resourceTypes.ts` 注册
   （kind 级或项级）；若子类别需要专属图标：登记图标。
2. 其余（导航、列表、容器页、新建/删除）零改动。

## 5. 一致性要求

- 任何新的资源管理功能 **不得** 绕开本机制新造私有协议（历史教训：
  `agent/bundle/resource/*` 专用协议已被本规范收编删除）。
- 资源管理页面 **不得** 新增第二个 Vue 视图或页面级组合式：全 App 资源页面
  唯一为 `WorkbenchView.vue`（逻辑唯一在 `useWorkbenchView.ts`），
  MainLayout 只是应用外壳（会话页 = 该页的 session 实例）。
- 资源列表 **不得** 自建轮询或私有刷新通道：实时性统一走 §3.2 的事件订阅
  （状态补丁 + 防抖刷新）；资源删除统一走 `resources/delete`（非实体存储型
  provider 重写 `delete_item` 钩子），不得绕开机制自持删除。
- 前端 **不得** 硬编码资源类型清单、类别标签、路径模板、能力开关；
  只允许注册图标与详情 editor 这类纯 UI 映射。
- 请求/响应结构变更必须先改 `symbio_core/schemas/resources.rs` 与
  `tauri/src/schemas/resources.ts` 两侧契约，再改实现。

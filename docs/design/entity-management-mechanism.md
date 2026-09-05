# 统一实体管理机制与规范

状态：现行规范
范围：Symbio 全部「实体管理」类功能（顶层实体 + 容器子实体）
关联：`docs/design/open-agent-bundle-spec.md`（OAB 标准）、
`symbio/src/symbio_core/schemas/resources.rs`（协议权威定义）、
`symbio/src/symbio_core/resources.rs`（注册表与统一分发）

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

## 2. 协议契约（`entities/*`）

### 2.1 注册表端点 `entities/providers`

返回 `ProvidersResponse { providers: ProviderInfo[] }`。每个 `ProviderInfo`：

| 字段 | 语义 |
|---|---|
| `kind` / `label` / `order` | 类型标识 / 展示标签 / 展示顺序（导航排序权威） |
| `prefix` | 实体操作路径前缀（前端拼 `${prefix}/entities/<op>`） |
| `capabilities` | 能力开关（zip 上传 / 独立表单 / 实时状态 / 可写 / 连接测试 / 只读） |
| `supports_upload` / `compact_list` / `status_indicator` | 列表页行为开关 |
| `container_kinds` | **容器声明**（见 §2.3；空 = 条目不是容器） |

### 2.2 统一操作

```
<prefix>/entities/list     → EntitiesListResponse { kind, capabilities, items, container? }
<prefix>/entities/get      → EntitySummary（容器语义时 extra.content 为文件内容）
<prefix>/entities/upload   → EntityUploadResponse { kind, id, created }
<prefix>/entities/delete   → EntityUploadResponse（同形状）
<prefix>/entities/status   → EntityStatusResponse（可选能力）
<prefix>/entities/detail   → DetailDefinitionResponse（详情定义，见 §3.2；可选能力）
```

- 列表项一律是 `EntitySummary`：`kind / id / name / status / summary` +
  `extra`（类型特有字段 flatten）。
- 上传二选一：`zip_b64`（目录型实体）或 `manifest`（表单型实体）。

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

`ContainerKindInfo { kind, label, description?, path_hint?, default_content?, capabilities }`
逐子类别下发：标签、语义说明、**新建路径模板**（`<name>` 占位符）、
**新建内容模板**、能力开关。前端新建表单完全由此驱动。

约束：

- 容器子实体的路径合法性由**后端**校验（路径白名单，拒绝穿越），
  前端模板仅用于展示。
- 容器列表响应的 `capabilities` 取该子类别的声明值；响应回显 `container`。

### 2.4 状态与事件约定（机制级，与具体实体无关）

- **状态取值**：`status ∈ active | working | disabled | error | unknown`
  （列表摘要与状态事件同用此约定）。`working` 表示实体正在工作，
  列表以独立视觉态渲染（脉冲点 + 「工作中」角标），不得折叠为其他取值。
- **事件分级**：`entities/status` 触发**状态事件**（列表项即时补丁）；
  provider 发出的 bus 事件分**粗粒度**（`type == "status" | "title"`，
  语义级变化）与**细粒度**（如流式数据帧）。机制列表只对状态事件与
  粗粒度事件做出反应，细粒度事件**不得**触发清单刷新。

## 3. 前端机制分层

| 层 | 位置 | 职责 |
|---|---|---|
| 注册表 store | `composables/useEntityProviders.ts` | 拉取并缓存 `entities/providers`；标签/能力查询；应用外壳导航（`useNavRailItems` + `navTargetOf`） |
| 服务层 | `services/resources.ts` | 协议函数（`listResources / getResource / putContainerResource / deleteResource / uploadResourceZip / uploadResourceForm / getResourceStatus / getDetailDefinition`），带可选 `container` |
| 三栏容器 | `components/common/Workbench.vue` + `useWorkbench.ts` | 「侧边栏 + 列表 + 详情」容器与状态机唯一实现（应用外壳模式 / 实体页模式） |
| **统一实体页** | `views/WorkbenchView.vue` + `composables/useWorkbenchView.ts` | **全 App 唯一实体页面**（entity / container 双模式，见 §3.1）；页面与 ts 均为标准复用件 |
| 应用外壳 | `views/MainLayout.vue` | 纯壳：Workbench（rail 来自 `useNavRailItems`）+ RouterView + 应用级服务，不持有实体逻辑 |
| 详情 editor 注册表 | `registry/entityTypes.ts` | 仅为**复杂详情**注册 kind 级 / 项级 editor 组件（§3.2 解析链第一优先） |
| 通用详情渲染器 | `components/entities/DetailForm.vue` | 定义驱动的详情/新建表单唯一实现（§3.2） |
| 图标注册表 | `registry/entityTypes.ts` | kind → SVG 图标（含容器子类别） |

### 3.1 WorkbenchView 的两种模式（同一组件、同一组合式）

页面形态由**路由参数自动判定**，实例内恒定；两模式唯一差异点：
① 类别来源；② list/delete 是否携带 `container` 字段。

| | leaf 模式 | container 模式 |
|---|---|---|
| 路由 | `/entities/:types?`、`/settings` | `/container/:kind/:id/resources`、`/agent/:agentId/resources` |
| 侧边栏 | 由 MainLayout 应用外壳承担 | 自带 = `ProviderInfo.container_kinds`（子类别 + 计数 + 返回键） |
| 类别来源 | providers 注册表解析 `:types`（`resolveActiveTypes`） | `container_kinds`（标签/路径模板/内容模板/能力后端下发） |
| 列表 | 各类别 `entities/list` 并行，混合平排 | `entities/list`（payload.container 单请求全量，核心按 kind 分箱） |
| 详情 | 编辑器解析链（§3.2） | 标准文本编辑器（`entities/get` extra.content + `entities/put` 写回） |
| 新建 | 类型选择 → 解析链（§3.2）/ zip / JSON（能力驱动分流） | 名称 + 内容（路径模板 `path_hint` + 内容模板 `default_content`） |

容器条目详情组件只做**只读概览**（`useContainerOverview`：类别 + 计数）；
管理逻辑一律在 WorkbenchView 页面侧，不得在详情组件内重复。

### 3.2 详情页生成：解析链与定义协议

**编辑器解析链**（entity 详情/新建统一适用，按序第一个命中者生效）：

1. **注册专属 editor**（`registry/entityTypes.ts`）——仅限**复杂详情**；
2. **定义驱动**（后端 `entities/detail` 返回非空 `DetailDefinition`
   → `DetailForm.vue` 通用渲染器动态生成）；
3. **通用兜底**（能力分流：zip 上传面板 / JSON manifest 表单 / 只读详情）。

**复杂详情判定标准**（满足其一才允许注册 editor，否则必须走定义或兜底）：
详情需要持久前端状态（如聊天工作区的会话选中）、富交互非表单形态
（如 bundle 只读概览 + 内部实体入口）、或纯信息展示（非编辑表单）。
除此之外的表单类详情一律由定义表达。

**定义协议**（`entities/detail`）：请求 `{kind, id}`，`id` 为空 = 请求
「新建态」定义；trait 默认 `detail_definition() -> None`（无定义 → 解析链
下探）。定义 `DetailDefinition`（权威 schema 见 `schemas/resources.rs`）
必须能完整表达一个交互不复杂的表单详情：

- **绑定** `binding`：`upload`（实体实体：预填 `item.config`，保存 emit
  save → `entities/upload` manifest）| `config`（配置分区：`load_path`/
  `save_path` 走标准 `config get|set`，渲染器自持保存）；
- **分区/折叠** `sections[]`：`collapsed` = 默认折叠；
- **控件全集** `widget ∈ text | password | number | select | textarea |
  toggle | datalist`（password 显隐、number min/max/step、textarea
  rows/full_width、datalist 动态建议）；
- **预设联动** `presets{field, fill, presets[]}`：选中预设后按 `fill`
  策略（`if_empty`/`always`）填充 `set`、**总是**应用 `set_always`、并把
  `options[key]` 注入对应字段的动态候选；
- **条件徽标/动作** `DetailCondition{key, equals, not_equals, truthy, all}`
  （键 = 表单字段 / `is_existing` / `is_default` / `cap.<name>`）；
- **派生回落链**：`title_from`（标题）、`subtitle_from`（副标题，select
  值自动映射选项标签）、`name_from`（保存补名）、`id_from`（新建 slug +
  `-2` 去重；后端 `validate_manifest` 为最终兜底）。

### 3.3 实时列表（机制能力）

- ① **状态事件** → 列表项状态角标即时补丁（不重拉清单）；
- ② **粗粒度 bus 事件**（§2.4）→ 防抖（800ms）刷新该类清单；
- 禁止轮询、禁止私有刷新通道；细粒度事件不得触发刷新。
  后端任何语义级变化（新建/删除/更名/运行状态）自动反映到列表。

路由（机制实例，非机制组成部分）：

- `/entities/:types?` —— 顶层统一实体页（entity）；
- `/settings` —— 同一 WorkbenchView 的 setting 实例；
- `/container/:kind/:id/resources` —— 通用容器实体页（container）；
- `/agent/:agentId/resources` —— agent 兼容别名。

## 4. 扩展指引（新增一类可管理实体）

**后端**：

1. 实现 `EntityProvider` trait（`kind`、按需重写 `list_items` /
   `upload` / `delete` / `test_status`；若是 EntityStore 目录型实体，
   实现 `category()` + `manifest_file()` + `summarize()` 即可走默认流程）；
2. 在插件 route 顶部接入 `crate::symbio_core::entities::dispatch`；
3. 在 `provider_registry()` 登记一条 `EntityProviderInfo`
   （kind / prefix / capabilities / order / label / supports_upload）；
4. （可选）条目是容器：登记 `container_kinds` 并实现四个 `*_container_item` 钩子；
5. （可选，**详情默认路径**）重写 `detail_definition` 钩子下发
   `DetailDefinition`（§3.2），前端零页面/零 ts 开发。

**前端**：

1. 详情默认零改动（定义驱动或通用兜底）；仅当详情满足 §3.2「复杂详情
   判定标准」才写专属 editor 并注册；需要专属图标：登记图标。
2. 其余（导航、列表、容器页、新建/删除、实时性）零改动。

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
  预设数据；只允许注册图标与复杂详情 editor 这类纯 UI 映射。
- 请求/响应结构变更必须先改 `symbio_core/schemas/resources.rs` 与
  `tauri/src/schemas/resources.ts` 两侧契约，再改实现。

## 6. 范例（实例，非机制组成部分）

以下实例仅演示机制的应用方式；它们的增删改不改变本规范。

### 6.1 会话（session）——「非实体存储 + editor 引导型创建」

- **列表** = 机制标准列表（摘要含实时 working 状态）；
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
五类动作。原前端 Model.vue 与预设常量已删除——预设数据由后端单一真相源下发。

### 6.3 设置分区（setting）——config 绑定

session/local/web 三分区由后端下发 config 绑定定义（`load_path`/
`save_path` = 各插件标准 `config get|set`），DetailForm 渲染并自持保存。
appearance（前端 store 即时生效）、about（信息展示）按 §3.2 判定标准
保留注册 editor。

### 6.4 agent（OAB bundle）——容器语义

条目即容器：`container_kinds` 声明 prompt/skill/mcp 子类别（标签/路径
模板/内容模板/能力），容器实体页与子实体管理完全由后端声明驱动；agent
详情为项级注册的复杂详情（只读概览 + 内部实体管理入口）。

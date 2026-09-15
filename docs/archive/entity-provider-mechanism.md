# 实体提供者机制（EntityProvider，已归档）

> **⚠️ 历史归档（2026-09-15）** —— 本文件描述的是**已废除的「实体提供者机制」**：
> `EntityProvider` trait（20 个钩子）+ `provider_registry()` 注册表 +
> `EntityVdfsAdapter`（1504 行，把 trait 接成 VDFS 子目录）。三者均已删除。
>
> **现行机制**见 [design/vdfs.md](../design/vdfs.md) §13.4——**每个资源插件
> 直接实现 `VdfsProvider`**。本文件当时所说的「`symbio_core/entities.rs` 只留
> 存储原语自由函数」也已是历史：`EntityStore` / `StorageService` 与那组存储原语
> （`symbio_core::entities`）随后一并废除，资源存储现在是
> **`VdfsProvider` 的三个集中实现**（`providers/vdfs_service`：单文件 / 目录 / 内存），
> 磁盘布局一字未改。
> 归档经过亦记于 [design/vdfs-frontend.md](../design/vdfs-frontend.md) §7 的 S16 / S17。
>
> 本文件仅作**历史参考，不再维护**。

原状态：规范（后端内部抽象）
原范围：各插件「实体」的存储 / 校验 / 容器语义实现层——即 VDFS 挂载点背后「资源怎么存、怎么校验」

关联（**均为当时形态**）：
- [vdfs.md](../design/vdfs.md)（VDFS 机制规范；本文件描述的是它当时 §11 / §13.4 所依赖的实体底座）
- [vdfs-frontend.md](../design/vdfs-frontend.md)（前端页面规范）
- `symbio/src/symbio_core/entities.rs` —— trait、注册表、统一操作实现（**已删除**）
- `symbio/src/symbio_core/schemas/entities.rs` —— 数据结构权威定义（**已收敛为
  `DetailDefinition` 表单方言模块**，`EntitySummary` / `EntityExport` 等协议类型删除）
- `symbio/src/symbio_core/vdfs/entity_adapter.rs` —— `EntityVdfsAdapter`，本机制的**唯一消费者**（**已删除**）

> **对外只有一个协议（VDFS）**：`entities/*` 调用协议已于 S11 下线，前端统一实体页
> 机制已于 S5 收口（前端资源页只有 VDFS 一台，见 [vdfs-frontend.md](../design/vdfs-frontend.md) §7）。
> 历史形态（含迁移期的端点映射表）见
> [archive/entity-management-mechanism.md](./entity-management-mechanism.md)。
>
> 本文件只写**规范与机制**。具体实体（会话、模型、设置分区……）如何应用机制一律
> 属于**范例**（§7），不是机制的组成部分；实例的新增/下线/调整不影响本规范的效力，
> 机制演进也不以任何单个实例为准。

---

## 1. 定位与不变量

实体机制是 VDFS 的**后端底座**：VDFS 只认「路径 + 访问位」，不认识任何具体资源的
语义；「一类资源有哪些条目、怎么校验、怎么落盘、有没有容器子实体」全部由本机制回答。

一次接入的完整动作：实现 `EntityProvider` + 在 `provider_registry()` 登记一条
`EntityProviderInfo`。此后：

- `EntityVdfsAdapter` 自动为它生成一个 `.vdfs/<kind>` 子目录；
- 导航标签 / 顺序 / 可新建类型 / 可整包导入 / 容器子类别全部由注册表派生；
- **前端零改动**（仅「复杂详情」才需登记一个 editor，见 §6）。

不变量：

- **后端是单一真相源**：类型存在性、顺序、标签、能力开关、容器子类别清单、详情
  定义，均由后端下发；前端不做任何硬编码枚举。
- **一处注册**：类型清单只出现在 `provider_registry()`；`nav_meta_of()` 让不经过
  适配器的挂载点（session / setting 等自持 `VdfsProvider` 的插件）复用同一份标签
  与顺序，二者**恒等**。
- **写盘 / 删除只有一份实现**：`entity_write` / `entity_delete` / `entity_import_zip`
  / `entity_export_zip` 是唯一实现，适配器与各 provider 的默认钩子都指向它们。
- **trait 不描述访问方式**：`EntityProvider` 只描述「怎么存、怎么校验」；地址、
  访问位、协议信封一律属于 VDFS。

## 2. `EntityProvider` trait —— 差异化钩子

必填仅 `kind()`；其余全部有默认实现（默认不适用时返回 `NotImplemented`），
插件按需重写。

### 2.1 存储与列表

| 钩子 | 默认 | 重写场景 |
|---|---|---|
| `category() -> Option<&'static str>` | `None` | 返回 `EntityStore` 分类；`None` = 非实体目录存储（session 走 `SessionStore`），写盘 / 删除的默认实现即 `NotImplemented` |
| `manifest_file() -> Option<&'static str>` | `None` | 主文件名（`Some(category)` 时用于默认 `list_items` 与 manifest 写盘） |
| `list_items(ctx) -> Vec<EntitySummary>` | `EntityStore` 枚举 + `summarize`（需 `category` + `manifest_file`） | 有独立数据源的（model / session / agent）接管；纯目录实体（mcp / skill）不必重写 |
| `summarize(ctx, id, manifest) -> EntitySummary` | `EntitySummary::new(kind, id, id)` | 下发 `name` / `status` / `summary` / `extra`（类型特有字段 flatten；前端列表展示与详情预填都读它） |

> 默认 `list_items` 对 manifest 读失败**不阻塞列表**——损坏条目降级为占位摘要。

### 2.2 写入与删除

| 钩子 | 默认 | 说明 |
|---|---|---|
| `validate_manifest(ctx, id, manifest) -> Value` | 原样放行 | 写盘前校验 / 规范化；返回值即实际落盘内容（model 用它补 id / name 缺省） |
| `new_entity_manifest(id, title) -> Value` | `{ id, name }` | VDFS `write { create }` 的**最小落盘 manifest**——语义是「先落一份可用默认配置，用户随后在详情里完善」。**有必填字段的插件必须重写，否则该类型在 VDFS 侧无法新建** |
| `on_uploaded(ctx, id)` | no-op | 写盘成功后的内存同步（mcp 回灌 config、model 同步注册表、agent 失效缓存） |
| `delete_item(ctx, id)` | `EntityStore` 目录删除（幂等：磁盘已无目录仅告警） | 非实体存储型必须重写（session 先 abort 活跃任务再删） |
| `on_deleted(ctx, id)` | no-op | 删除成功后的内存 / 缓存清理 |

> **实体 id 由路径承载**：编辑链路下发的 manifest 只含纯字段值、不含 `id`；
> `entity_write` 统一以路径 id 兜底补全（`ensure_manifest_id`），故 provider 的
> `validate_manifest` 可以安全地把 manifest 反序列化到 `id` 必填的结构体。

### 2.3 状态

| 钩子 | 默认 | 说明 |
|---|---|---|
| `test_status(ctx, id) -> EntityStatusResponse`（类型已删除） | `NotImplemented` | VDFS 侧表现为节点动作 `test`（`VFDS_ACTION_TEST`）。**连接失败应映射为 `Ok(status: "failed")` 而非 `Err`**——失败是**结果**，不是协议错误 |

列表摘要的 `status ∈ active | working | disabled | error | unknown`；`test_status`
的结果另用 `connected` / `failed`（常量 `ENTITY_STATUS_CONNECTED` /
`ENTITY_STATUS_FAILED`）。

### 2.4 整包导入 / 导出

| 钩子 | 默认 | 说明 |
|---|---|---|
| `import_zip(ctx, name, zip) -> EntityUploadResponse` | `entity_import_zip`（EntityStore 型通用解包，整目录覆盖） | VDFS 侧是**一种「新建类型」**（`ext = zip`、`source = file`），**不是第二条协议**。目录自管类型（agent bundle）重写——id 取自包内 manifest，`name` 只是建议名 |
| `export_zip(ctx, id) -> EntityExport` | `entity_export_zip`（打包整个实体目录） | VDFS 侧是节点动作 `export`（`VFDS_ACTION_EXPORT`），与导入互为逆向 |

### 2.5 详情定义

| 钩子 | 默认 | 说明 |
|---|---|---|
| `detail_definition(ctx, id) -> Option<DetailDefinition>` | `None` | `None` = 无定义（前端回退机制级只读视图）；`id` 为空 = 请求**新建态**定义。见 §6 |

## 3. 注册表

`provider_registry() -> &'static [EntityProviderInfo]`：编译期常量表，**顺序即展示
顺序**（无配置覆盖，原 `symbio.provider_order` 已随 `entities/providers` 下线）。

`EntityProviderInfo`：

| 字段 | 语义 |
|---|---|
| `kind` | 类型标识，**同时是 `.vdfs` 子目录名** |
| `order` | 展示顺序（导航排序权威） |
| `label` | 展示标签 |
| `supports_upload` | 能否以「最小 manifest」新建（一次 `vdfs/write { create }`） |
| `supports_import` | 能否**整包导入**（zip）；目录自管的类型（agent bundle）也可为 true |
| `container_kinds` | 容器声明（§5）；空 = 条目不是容器 |

`nav_meta_of(kind) -> Option<(&'static str, i32)>`：返回 `(label, order)`，供不经过
`EntityVdfsAdapter` 的挂载点复用，使 `.vdfs` 左栏的顺序与标签和注册表**恒等**；
未登记的 kind 返回 `None`。

## 4. 统一操作实现

四个自由函数（`symbio_core/entities.rs`）是「写盘 / 删除 / 导入 / 导出」的**唯一
实现**——适配器与各 provider 的默认钩子都走它们，因此两条链路行为完全一致
（同一份校验、同一份写盘、同一个事件）。

- `entity_write(provider, ctx, id, manifest)` —— 职责链：`ensure_manifest_id` →
  `validate_manifest` 规范化 → 写盘 → `on_uploaded` 内存同步 → 发布实体生命周期
  事件。无 `category` 返回 `NotImplemented`；无 `manifest_file` 返回校验错误。
- `entity_delete(provider, ctx, id)` —— 删除 + 回调 `on_deleted` + 发布事件。
- `entity_import_zip(provider, ctx, name, zip)` —— 通用解包
  （`parse_zip` → `strip_common_root` → 整目录覆盖）。
- `entity_export_zip(provider, ctx, id)` —— 打包实体目录（`zip_dir`）。

zip 工具（`zip_dir` / `parse_zip` / `strip_common_root` / `extract_zip_to_entity`）与
base64 编解码（`encode_b64` / `decode_b64`）同在此模块；`EntityError` 为模块内错误
类型，`From<EntityError> for PluginError` 提供转换。

## 5. 容器语义

**当 provider 声明了 `container_kinds` 时，其每个条目本身是一个容器**，内部托管
声明的子实体类型。

`ContainerKindSpec`（编译期静态版，下发给前端时转为 `ContainerKindInfo`）：

| 字段 | 语义 |
|---|---|
| `kind` / `label` / `description` | 子实体类型 / 展示标签 / 语义说明（前端新建与编辑表单的提示文本） |
| `path_hint` | 新建路径模板（`<name>` 占位符），如 `prompts/<name>.md`；**空 = 系统管理型**：不可由用户创建，仅支持查看 / 删除 |
| `default_content` | 新建内容模板 |
| `view` | 中栏结构形态：`VIEW_LIST`（缺省）= 列表；`VIEW_TREE` = 树视图（懒加载） |

`container_kinds_for(kind)` 是「kind → 子实体声明」的唯一映射（当前：
`ENTITY_AGENT` → `AGENT_CONTAINER_KINDS`、`ENTITY_SESSION` → `SESSION_CONTAINER_KINDS`）。

四个容器钩子 + 订阅钩子（默认全部 `NotImplemented` / no-op）：

| 钩子 | VDFS 侧 | 说明 |
|---|---|---|
| `list_container_items(ctx, sub_kind, container, parent)` | `list` | `sub_kind` 可选过滤（`None` = 全部子类型，条目 `kind` 供前端分类计数）；`parent` 仅树视图使用，`None` = 根层，懒加载逐层下发 |
| `get_container_item(ctx, id, container)` | `read` | 内容型置于 `extra.content` |
| `put_container_item(ctx, id, content, container)` | `write` | `id` 为容器内相对路径 |
| `delete_container_item(ctx, id, container)` | `delete` | — |
| `watch_container` / `unwatch_container` | `watch` / `unwatch` | 默认 no-op = 无实时能力；实时场景实现**引用计数**（同一容器数据多方共享监听），视图挂载期订阅、卸载时配对取消 |

约束：

- **路径合法性由后端校验**（路径白名单，拒绝穿越）；`path_hint` 模板仅用于前端展示。
- **树视图只是结构机制**：`view = "tree"` 只定义「层级 + 懒加载 + 选择」，不含任何
  文件系统语义；会话工作目录浏览（`kind = "dir"`）只是它的一个**场景实现**。
- **订阅不得携带 sessionId**（会触发会话回放缓冲的 drain，破坏聊天流式恢复）；
  前端订阅为 kind 级、handler 内按容器过滤。

## 6. 详情定义（`DetailDefinition` —— symbio 的 `schema` 方言）

`DetailDefinition` **不属于 VDFS 协议**，而是 symbio 的 `schema` 方言
（[vdfs.md](../design/vdfs.md) §11）：适配器把它挂在节点 `schema` 上、`ext = form`，前端
通用表单渲染器据此动态生成详情页。

**编辑器解析链**（按序第一个命中者生效）：

1. 注册专属 editor（**仅限复杂详情**）；
2. 定义驱动（`detail_definition` 返回非空 → 通用渲染器动态生成）；
3. 通用兜底（能力分流：zip 上传面板 / JSON manifest 表单 / 只读视图）。

**复杂详情判定标准**（满足其一才允许注册 editor，否则必须走定义或兜底）：详情
需要持久前端状态（如聊天工作区的会话选中），或即时生效型前端 store 交互
（如 appearance）。**只读概览型**（如 bundle 概览）由 `info` 绑定表达，**不算**
复杂详情。

定义能力（权威 schema 见 `schemas/entities.rs`）：

- **绑定** `binding ∈ upload | config | info`：`upload`（实体：预填 `item.config`，
  保存走 manifest 写入）| `config`（配置分区：经 `load_path` / `save_path` 走目标
  插件的标准 config 协议，渲染器自持保存）| `info`（只读概览：无保存，动作仅限
  `open-container` / `delete` 等机制通道动作）；
- **分区** `sections[]`（`collapsed` = 默认折叠，如「高级设置」）；
- **控件** `widget ∈ text | password | number | select | textarea | toggle |
  datalist | list | map | static`（`list` = 字符串数组、`map` = 键值对，序列化
  约定与后端 `validate_manifest` 两侧一致；`static` 只读展示，`options` 可作
  值 → 标签映射）；
- **条件显隐** `visible_when: DetailCondition`（不满足时整行不渲染且**不参与保存**）；
- **预设联动** `presets { field, fill, presets[] }`（`fill ∈ if_empty | always`
  填充 `set`、**总是**应用 `set_always`，并把 `options[key]` 注入对应字段的动态候选）；
- **徽标 / 动作**：`DetailBadge{ when, label, style }`；
  `DetailAction{ id, label, style, icon, when, disabled_when, payload, busy_label }`，
  `id ∈ save | test | delete | set-default | open-container`（`open-container` 经
  `payload.kind` 路由推入容器实体页）；`icon` 缺省按 id 的默认图标映射，无映射回落
  文字按钮；
- **派生回落链**（均为「首个非空」）：`title_from` / `title_fallback`、
  `subtitle_from`、`name_from`（保存补名）、`id_from`（新建 slug；前端 `-2` 去重，
  后端 `validate_manifest` 兜底）。

## 7. 范例（实例，非机制组成部分）

> 以下实例仅演示机制的应用方式；它们的增删改不改变本规范。

- **会话（session）** —— 非实体存储（`category() == None`）+ 专属 editor 引导型创建：
  `list_items` 自持、`delete_item` 重写（先 abort 活跃任务再删）、
  `supports_upload = false`；条目是容器（`SESSION_CONTAINER_KINDS`：子会话列表 +
  目录树 tree）。
- **模型（model）** —— 定义驱动的复杂度上限：`upload` 绑定，供应商预设联动、
  datalist、API Key 显隐、高级设置折叠分区、条件徽标与五类动作；`validate_manifest`
  做必填校验。
- **设置（setting）** —— `config` 绑定：分区清单固定，`supports_upload = false`；
  各分区经 `config/get` / `config/set` 读写（appearance / about 为前端自持）。
- **MCP / Skill** —— `upload` 绑定 + 结构化控件；`summarize` 下发 `extra.config`
  供预填，`validate_manifest` 双向映射（mcp ↔ `server.json`、skill ↔ `SKILL.md`）；
  `supports_import = true`（zip 整包）。
- **agent（OAB bundle）** —— 容器语义 + `info` 概览：`supports_upload = false` /
  `supports_import = true`（只能整包导入，`import_zip` 重写走 `BundleStore`）；
  条目托管 prompt / skill / mcp 三类子实体。

## 8. 扩展指引（新增一类可管理资源）

**后端**：

1. 实现 `EntityProvider`（`kind`；按需重写 `list_items` / `delete_item` /
   `import_zip` / `test_status` / `detail_definition`；EntityStore 目录型资源实现
   `category()` + `manifest_file()` + `summarize()` 即可走默认流程）；
2. 在 `provider_registry()` 登记一条 `EntityProviderInfo` —— 插件 route **不再需要
   接任何分发**，VDFS 侧由 `EntityVdfsAdapter` 自动为它生成 `.vdfs/<kind>`；
3. （可选）整包导入 / 导出：`supports_import = true` + 按需重写 `import_zip` /
   `export_zip`，并在 `detail_definition` 里声明 `export` 动作；
4. （可选）条目是容器：登记 `container_kinds` 并实现四个 `*_container_item` 钩子
   （+ 实时场景的 `watch_container` / `unwatch_container`）。

**前端**：**零改动** —— 子目录、导航、列表、新建 / 导入 / 删除、实时性全部由 VDFS
机制生成；仅当详情属「复杂形态」才补一个专属 editor 与图标注册。

## 9. 一致性要求

- 任何新的实体管理功能 **不得** 绕开本机制新造私有协议；对外访问一律经 `vdfs/*`。
- **写盘 / 删除 / 导入 / 导出不得有第二份实现**：只能走 `entity_write` /
  `entity_delete` / `entity_import_zip` / `entity_export_zip`，或重写对应 provider
  钩子后仍委托它们。
- **类型清单只归注册表**：不得在别处硬编码 kind、标签、顺序；不经过适配器的挂载点
  用 `nav_meta_of()` 取同一份元数据。
- **能力只由声明决定**：新建看 `supports_upload` / `path_hint`，导入看
  `supports_import`，删除看 provider 是否重写 `delete_item`；消费者不得按 kind
  判定能力。
- 详情 editor 注册 **不得** 违反 §6 解析链与复杂详情判定标准：可由定义表达的表单
  详情注册专属 editor 属于违规实现。
- 请求 / 响应结构变更必须先改 `symbio_core/schemas/entities.rs` 与
  `tauri/src/schemas/entities.ts` 两侧契约，再改实现。

# 统一资源管理器：混合列表 + 前后端动态 provider 注册（当前实现）

> 本文是**现状实现说明**（非历史方案）。历史演进过程见 `统一资源管理器多类型升级.md` / `统一资源管理器混合列表+优化.md` / `统一资源协议trait化与死代码清理.md`。

## 目标与已确认决策

- 前端按类型（扩展名 kind / kind:ext）注册对应 editor，新增类型只改注册

- 后端以**声明式静态注册表**`provider_registry()` 主动声明有哪些资源 provider（宿主级单一真相源）

- 前端自动拉取已注册 provider，动态生成左侧导航（**含"设置"入口**，不再写死）

- 聚合端点为宿主 HomePlugin.route 的 `resources/providers` 分支（复用 callPlugin 通路）

- **六类 provider**：model / mcp / agent / skill / session / setting

## 架构总览

```
后端：核心层 provider_registry() 静态声明表
  [{kind, provider_name, prefix, capabilities, order, label, supports_upload, compact_list, nav}]
       ↑ 新插件接入 = 实现 ResourceProvider + 登记一条 + 插件 route 接 dispatch（前端零改动）
       ↓
宿主 HomePlugin.route 新增 "resources/providers" 分支 → 返回 provider_registry()
       ↓（callPlugin('/resources/providers')）
前端：fetchProviders() → 运行时 provider store（模块级单例）
  ├─ MainLayout：左侧导航按 ProviderInfo.nav 分组动态生成
  │    ├─ 资源区（nav='resources'）：model/mcp/agent/skill
  │    └─ 设置区（nav='settings'）：setting（"设置"入口由注册表驱动，非硬编码）
  ├─ ResourceManagerView：以 providers 为类型集合 → 混合平排列表 / settings 分区实例
  └─ 类型注册表退化为纯展示层：editor(Vue 组件) + icon(SVG) 注册（支持 kind:ext 复合键）
```

**分层原则**：

- **存在性/能力/前缀/顺序/标签/导航归属/简洁模式**：后端 `provider_registry` 决定（权威）

- **editor(Vue 组件) / icon(SVG)**：必须前端按 kind（或 kind:ext）注册（后端无法下发 UI）

## 后端

### 1) 核心层声明式注册表 — `symbio/src/symbio_core/resources.rs`

```rust
pub struct ResourceProviderInfo {
    pub kind: &'static str,
    pub provider_name: &'static str,   // 路径 [provider_name]/[id].[kind]
    pub prefix: &'static str,          // 资源操作前缀（前端 `${prefix}/resources/<op>`）
    pub capabilities: ResourceCapabilities,
    pub order: i32,                    // 展示顺序（导航/类型选择/列表顺序基准）
    pub label: &'static str,
    pub supports_upload: bool,         // 是否可在资源管理器内创建/删除
    pub compact_list: bool,            // 列表简洁模式（仅图标+标题，如设置分区）
    pub nav: &'static str,             // 主导航归属：NAV_RESOURCES / NAV_SETTINGS；空串不进导航
}
pub fn provider_registry() -> &'static [ResourceProviderInfo]
```

登记内容：

| kind    | prefix           | supports\_upload | compact\_list | nav            |
| ------- | ---------------- | ---------------- | ------------- | -------------- |
| model   | `worker/model`   | true             | false         | resources      |
| mcp     | `mcp`            | true             | false         | resources      |
| agent   | `agent`          | true             | false         | resources      |
| skill   | `skill`          | true             | false         | resources      |
| session | `worker/session` | false            | false         | （空，走独立"会话"主入口） |
| setting | `setting`        | false            | true          | settings       |

- `supports_upload` 独立于能力表，表达"该类型在资源管理器内可创建/删除"（session/setting 为 false，修复"新建必失败"）

- 序列化 DTO 为 `ProviderInfo`（`&'static str` → `String`），随 `ProvidersResponse { providers }` 下发；字段含 `compact_list` 与 `nav`

### 2) 宿主 route 分支 — `symbio/src/plugins/home/plugin.rs`

在 `match path` 早期（parse\_path 兜底之前）：

```rust
"resources/providers" => return Ok(PluginPayload::new(&resources::ProvidersResponse::from_registry()));
```

顶层无插件前缀，不与各插件 `{plugin}/resources/*` 冲突。

### 3) setting 插件 — `symbio/src/plugins/setting/plugin.rs`

实现 `ResourceProvider`，route 顶部接 `dispatch`。`list_items` 返回固定分区清单
（appearance/session/local/web/about，**按声明顺序即展示顺序**），每项 `extra.config_type`
作为 editor 的"扩展名"。supports\_upload=false → 不可新建/删除；各分区保存由前端 editor 自持通道完成。

## 前端

### 4) schema — `tauri/src/schemas/resources.ts`

- kind 用开放 `string`（`ResourceType` 联合类型已删除，所有资源操作参数即 `string`）

- `ProviderInfo` 接口含 `compact_list?` / `nav?`；无 `DEFAULT_CAPABILITIES`（服务层用 `UNKNOWN_CAPABILITIES` 兜底）

- `NAV_RESOURCES` / `NAV_SETTINGS` 常量与后端对齐

### 5) services — `tauri/src/services/resources.ts`

- 所有函数 `type: string`；未知类型回退只读空态

- `fetchProviders()` → `callPlugin('resources/providers')`，同时填充 kind→prefix 缓存

### 6) 运行时 provider store — `tauri/src/composables/useResourceProviders.ts`

- `providers`（模块级单例）+ 幂等 `loadProviders()`

- 派生：`resourceNav`（nav==='resources'）、`settingsNav`（nav==='settings'）、`labelOf`、`capabilitiesOf`、`creatableProviders`

### 7) 类型注册表（纯展示层）— `tauri/src/registry/resourceTypes.ts`

- 无硬编码 prefix/capabilities/label；仅 editor + icon 注册

- `registerResourceEditor(kind|'kind:ext', Component)`：`setting:appearance` / `setting:session` / `setting:local` / `setting:web` / `setting:about` → 各自 editor；`model` → `ModelProviderForm`；未注册走通用兜底（zip/JSON/只读详情）

- `registerResourceIcon` 含 `setting` 整体齿轮图标（导航用）

### 8) 导航动态化 — `tauri/src/views/MainLayout.vue`

- 启动时 `await loadProviders()`；左侧导航**全部**由 providers 渲染：

  - 资源区 `v-for="p in resourceNav"`；设置区（footer）`v-for="p in settingsNav"`（"设置"按钮不再手写）

  - 图标查 icon 注册表（未注册用默认），label 用后端

### 9) 统一资源视图 — `tauri/src/views/ResourceManagerView.vue`

- 类型集合来自 provider store；`/resources/:types?`（缺省 all）、`/settings`（types='setting'）

- **混合平排列表**：所有类型所有项展平为一张列表，**按服务器返回顺序**（类型按 activeTypes，类型内按 `resources/list` 原序；不做前端 name 排序）

- compact\_list 类型项仅显示类型图标+标题

- 复合选择键 `${kind}:${id}`；每项 meta 路径 tag（可复制）

- 新建：多类型先选类型（仅列 creaTable provider），单类型直接进入；session/setting 无新建/删除

- 实时状态订阅 per-kind（禁止轮询）

### 10) 逻辑组合式 + 单测

- `useResourcePage.ts` 承载活动类型解析、typeStates 加载、`buildMixedItems`（保序展平）、复合选中、新建类型选择、操作定向刷新

- 单测：`useResourcePage.spec.ts`（保序/过滤/canCreate/选中）、`useResourceProviders.spec.ts`（幂等/过滤）、`registry/__tests__/resourceTypes.spec.ts`、setting 插件 `list_items` 保序断言

## 已知边界 / 刻意保留差异

- **session**：资源协议仅 list/status（upload/delete 返回 NotImplemented，为导入/导出留槽位）；不进主导航（走会话主入口）

- **setting**：分区清单不可在资源管理器内新建/删除；"设置"入口由 nav='settings' 驱动

- **compact**：当前实现为**页面级**判断（`activeTypes.some(compact_list)` 则整页简洁）；未细化到逐 item

- 详情 editor 的读取按 kind → kind:ext → 通用兜底三级解析

## 验证

1. 后端：`cargo clippy --all-targets` 零告警 → `cargo test`（含 setting `list_items` 保序断言）
2. 前端：`npx vue-tsc --noEmit` → `npx vitest run` → `npx vite build`
3. 手测：`resources/providers` 返回六类；/settings 分区保序、compact 显示、图标正常；多类型新建先选类型


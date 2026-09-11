# 级联选项机制与规范

状态：现行规范
范围：会话页输入区下方的「选项行」（根选项 + 级联子项 + 自动化表单）
关联：`docs/design/entity-management-mechanism.md`（表单机制复用）、
`symbio/src/symbio_core/schemas/options.rs`（协议权威定义）、
`symbio/src/symbio_core/option.rs`（收集机制 `OptionVisitor` / `collect_options`）、
`symbio/src/plugins/session/options.rs`（选项宿主）、
`tauri/src/composables/useSessionOptions.ts` + `tauri/src/components/chat/ChatOptionBar.vue`
（前端唯一实现）

> 本文件只写**规范与机制**。具体选项（工作目录、智能体、模型、运行模式、
> 风险等级、心跳任务……）一律属于**范例**（§7），不是机制的组成部分；
> 实例的新增/下线/调整不影响本规范的效力，机制演进也不以任何单个实例为准。

---

## 1. 目标与不变量

会话页输入区下方的选项行（历史上由前端写死：工作目录选择器 + 智能体/模型/
模式/风险下拉）**改由后端决定**：后端在统一端点上一次下发根选项列表，前端只
渲染机制、转发调用。

新增一个选项时，只需：

1. 后端在自身插件的 `traverse(available_options)` 分支注册一个 `OptionNode`
   （或由既有宿主声明）；
2. 前端**默认零改动**。

不变量：

- **后端是单一真相源**：选项的显示信息、状态、类型、层级、当前选中值、动作
  目标与参数全部由后端下发；前端不做任何业务枚举。
- **前端是纯消费者**：前端只允许补充「UI 映射」（图标名 → emoji）与「机制级
  原语」（原生目录/文件对话框、点路径写入），存放于 `registry/optionIcons.ts`
  与 `useSessionOptions.ts`，与业务契约严格分离。
- **单通道**：根层与子层共用 `options/list`，仅以 `parent` 参数区分——与实体
  机制 `entities/list` 的树懒加载同构。不为同一机制造第二个协议通道。
- **选项不感知彼此**：插件之间不可见，跨插件的展示顺序用**号段约定**（§3.2）
  协调，不引入插件间直接依赖。

## 2. 协议契约（`options/list`）

### 2.1 端点

```
worker/session/options/list  → OptionsResponse { nodes: OptionNode[] }
```

请求 `OptionsRequest`：

| 字段 | 语义 |
|---|---|
| `session_id` | 会话作用域。宿主据此把会话状态（`AGENT_ID` / `PROVIDER_ID` / `WORKDIR`）注入收集上下文，供贡献方回填「当前选中值」。缺省 = 草稿态（新建会话前），宿主按默认值下发 |
| `parent` | 父节点 id。缺省 = 根层；非空 = 该节点的子项（`sub` 懒加载） |

**选项宿主 = session 插件**：只有它持有会话上下文与「状态落库」的目标
（`session.metadata`），因此由它提供该端点，并把「全项目收集」（§3）与
「宿主自有选项」合流为一次响应。

### 2.2 节点与动作

`OptionNode`（`#[serde(default)]`，缺省字段省略）：

| 字段 | 语义 |
|---|---|
| `id` | 节点 id（会话作用域内唯一；子项建议带父前缀） |
| `label` / `icon` / `description` | **显示信息**：显示名 / 图标名（前端 UI 资产映射）/ 语义说明（悬浮提示） |
| `option_type` | **类型**：`invoke` \| `sub` \| `form`（缺省 `invoke`） |
| `order` | 展示顺序（升序；跨插件贡献时必需） |
| `status` / `status_detail` / `enabled` | **状态信息**：`active`\|`working`\|`disabled`\|`error`\|`unknown`（与实体机制 §2.4 同一语义）；补充说明；是否可选（`false` = 只读展示） |
| `value` / `value_label` | 当前选中值 / 其展示文本（状态型选项；缺省展示文本 = `value`） |
| `action` | `invoke` / `form` 的执行规格（见下） |
| `form` / `data` | `form`：表单定义（`DetailDefinition`，与实体详情表单同一套 schema）/ 表单初始数据 |
| `children` | `sub`：子选项（内联；空 = 经 `parent` 懒加载） |

`OptionAction`：

| 字段 | 语义 |
|---|---|
| `endpoint` | 后端服务路径（前端 `callPlugin` 调用）；空 = 纯原生动作（仅取值不外呼） |
| `payload` | 固定参数（动态参数合并其上） |
| `pick` | **机制原生取值原语**（闭集 `directory` \| `file`）。后端无法唤起原生对话框，故由前端取值后写入 `bind` |
| `bind` | 动态参数的写入路径（点路径，如 `metadata.workdir`）。`pick` 与 `form` 保存共用 |

### 2.3 三类选项

| 类型 | 语义 | 交互 |
|---|---|---|
| `invoke` | 调用特定后端服务 | 点击 → 合并参数（`pick` 或固定 `payload`）→ `callPlugin(endpoint, payload)` |
| `sub` | 子选项列表（级联） | 展开菜单 → 选中子项（子项自身仍是选项节点，通常为 `invoke`） |
| `form` | 自动化表单 | 打开表单（复用 `DetailDefinition` / `DetailForm`）→ 保存 → 调用 `endpoint` |

- `sub` 的层级**任意深度**：子项可再是 `sub`；子项内联（`children`）或经
  `parent` 懒加载。前端以**菜单栈**渲染，深度不构成特例。
- `form` 与实体详情表单**同源**：字段 / 分区 / 条件显隐（`visible_when`）/
  预设联动 / 结构化控件（list / map / toggle / number / textarea…）完全复用，
  仅绑定模式为 `option`（§4.3）。新增表单型选项 = 后端下发定义，前端零改动。

### 2.4 状态与展示约定（机制级）

- 状态取值复用实体机制 §2.4 的闭集；`disabled` 与 `enabled = false` 语义分工：
  `enabled = false` 表示**不可交互**（前端不响应点击），`status = disabled`
  表示「未启用但可交互」（如心跳表单未开启）。二者常同时置位表示只读。
- `working` 由前端渲染为旋转指示，`error` 渲染为告警标记，**不得**折叠成
  其他取值。
- 「当前选中值」的表达是**父节点带 `value` + 子项各带自身 `value`**；前端据
  `child.value === parent.value` 判定选中态。故子项 `value` 必须与父节点
  `value` 处于同一取值域（如「不使用 Agent」子项 `value = ""` ↔ 父节点
  `value = ""`）。

### 2.5 会话作用域注入与状态落库

- **宿主统一注入会话作用域**：`options/list` 返回前，宿主遍历节点树，为每个
  `action.payload`（对象/缺省载荷）补上 `session_id`。贡献方**无需感知会话
  标识**；载荷已显式声明者以贡献方为准。（前端另经 `callPlugin` 的
  `{ session_id }` 传入路由上下文，二者同源。）
- **单一落库目标**：所有状态型选项的选择统一指向
  `SESSION_STATE_ENDPOINT = worker/session/update`，把值**浅合并**写入
  `session.metadata`。后端各解析链（`orchestrator::resolve_session_params`、
  `tool_executor`）已按 metadata 回退取值，因此：

  > **前端不需要知道任何业务字段名**——选项的展示、选择、落库全部由后端声明。

  这是「前端零业务代码」的关键，也是「只允许一条写入路径」的体现：任何
  绕过该端点的前端旁路写入（如组件内直接调 `session/update` 写业务字段）
  都属违规。

## 3. 收集机制（`OptionVisitor` + `Plugin::traverse`）

### 3.1 与 `CapabilityVisitor` 的关系

选项收集是能力收集（`CapabilityVisitor` + `traverse(available_tools)`）的
**平行第二通道**：

```
session ──collect_options(parent, ctx)──▶ parent.traverse(available_options)
                                            ├─ session : 工作目录 / 风险等级 / 运行模式 / 心跳
                                            ├─ agent   : 智能体选择
                                            └─ model   : Model 选择
```

- 共享的是**收集机制**：traverse 广播、上下文键（`PATH` + `OPTION_VISITOR`）、
  失败降级（父插件缺失返回空收集器；单插件失败仅记日志）。
- 不复用的是**产物语义**：能力是「可调用对象」，选项是「可展示的数据节点」。
  混装会让 `list_capability` 的消费方（LLM 工具协议）被迫理解选项结构。
- 收集器接口 `OptionVisitor`（`register_option` / `register_batch` /
  `list_options`）：按 id 去重（后者覆盖、保留先注册槽位），按 `order` 稳定排序。
- 收集**无状态**：每次调用返回全新 visitor，节点携带本次会话的实时选中值，
  **不可跨请求复用**。

### 3.2 order 号段约定

插件之间不可见，故 order 采用**号段约定**（各插件在自身定义本地常量）：

| 号段 | 归属 |
|---|---|
| `10` | 工作目录 |
| `20` | 智能体 |
| `30` | Model |
| `40` | 风险等级 |
| `50` | 运行模式 |
| `60` / `61` | 心跳任务（配置表单 / 立即执行） |

新增贡献方取空闲号段；号段冲突即机制级冲突，须在规范内重新分配。

### 3.3 贡献约定

贡献方在 `traverse` 中按 `ctx[PATH] == TRAVERSE_AVAILABLE_OPTIONS` 判定是否
贡献，随后：

1. 从 `ctx` 读取本插件需要的会话状态（由宿主注入，如 `WORKDIR` / `AGENT_ID` /
   `PROVIDER_ID`）——**不自行加载会话**，保持插件间零耦合；
2. 构造 `OptionNode`（推荐用协议层提供的构造器：`OptionNode::invoke` /
   `sub` / `session_state`、`OptionAction::session_state_set` / `session_state_bind`、
   链式 `with_icon` / `with_order` / `with_value_label` / `with_status` /
   `with_description` / `with_enabled`）；
3. `ctx.get::<Arc<dyn OptionVisitor>>(OPTION_VISITOR)` 后 `register_option`。

无法回填当前值时（如选中的实体已不存在）应仍下发节点并允许重选，不做特例。

## 4. 前端机制分层

前端实现只有三件：一个组合式函数（机制）、一个渲染件（UI）、一个表单承载件。

### 4.1 `useSessionOptions`（机制核心）

- `refresh()`：拉取根选项（`options/list`，`parent` 缺省）；
- `loadChildren(parentId)`：子项懒加载；
- `dispatch(action, dynamic?)`：**通用分派**——
  1. 深拷贝 `action.payload`；
  2. `pick`：唤起原生对话框（`directory` / `file`），用户取消则中止；
     `dynamic`：取表单/程序化动态值；
  3. 动态值按 `action.bind` 的**点路径**写入载荷（父级缺失自动补对象）；
  4. 有会话 → `callPlugin(endpoint, payload, undefined, { session_id })`，
     随后把载荷中的 `metadata` 补丁镜射到会话 store，并 `refresh()` 回读权威值；
     无会话（草稿态）→ 把补丁缓冲到 `draftMetadata`，不落后端（§4.4）。
- `dispatch` 是**唯一**知道 `pick` / `bind` / `metadata` 语义的地方，且这些语义
  都是机制级的（原生对话框、点路径、`session/update` 的浅合并），不含业务字段。

### 4.2 `ChatOptionBar.vue`（唯一渲染件）

- 渲染根节点：图标 + 标签 + 当前值 + 状态标记；`enabled = false` 时不响应点击；
- `sub`：**菜单栈**（**任意深度**）——点击入栈、菜单左上「返回」出栈；
  内联为空则懒加载；子项选中态按 §2.4 判定；
- `invoke`：直接 `dispatch`；草稿态下原生取值就地回显；
- `form`：打开 `OptionFormDialog`；
- 点击空白处关栈；成功后仅在失败时提示，成功以「回读到的状态变化」为反馈。

### 4.3 `OptionFormDialog` 与 `DetailForm` 的 `option` 绑定

- `DetailForm` 新增绑定模式 `option`（与 `upload` / `config` / `info` 并列）：
  **预填**自 `optionData`（= 节点 `data`），**保存** `emit('option-save', 字段值)`
  ——纯字段值、不含 id、不参与实体校验。
- 序列化差异：`upload` 绑定跳过 `visible_when` 不满足的字段（互斥字段不落库）；
  `option` 绑定保存**全部字段**——表单选项对应一份**完整配置对象**（如心跳任务：
  关闭开关不得丢失间隔/提示词）。
- 关闭入口由 `OptionFormDialog` 注入一个机制动作 `cancel`（复用统一动作区，
  避免叠加外框 header 与 `DetailForm` 已有 header 冲突）。

### 4.4 草稿态（新建会话前）

新建会话前尚无 `session_id`，没有落库目标：

- 宿主按**默认值**下发全部选项（含会话自有选项），选项行照常完整渲染；
- 用户的每次选择由 `useSessionOptions` 缓冲为 `draftMetadata`（对
  `action.payload.metadata` 的浅合并累积）；
- 创建会话时把该补丁**整体透传**给 `createSession(metadata)`，与后端
  `session/update` 同一浅合并语义——**前端不解释字段名**；
- 进入真实会话后缓冲失效，选项值改由后端回读。

## 5. 扩展指引（新增一个选项）

1. **状态型（单选/多档）**：在贡献方 `traverse(available_options)` 中构造
   `sub` 节点，子项用 `OptionNode::session_state(id, label, key, value)`——
   落库路径与语义由该构造器统一到 `session/update`。若状态由别的插件持有，
   在**该插件**中贡献（如智能体选项由 agent 插件贡献）。
2. **命令型（无状态动作）**：构造 `invoke` 节点，`action.endpoint` 指向后端
   服务，`payload` 给固定参数。
3. **表单型**：构造 `form` 节点，`form` 给 `DetailDefinition`（`binding = "option"`）、
   `data` 给初始值、`action.payload.metadata` + `action.bind` 指定落库位置。
4. **原生取值**：在 `action` 上声明 `pick`（`directory` / `file`）与 `bind`。
5. 需要**只读/禁用**时用 `with_enabled(false)`（+ 可选 `with_status("disabled")`），
   并在 `description` 说明原因——前端零特判。

前端**不需要任何改动**；仅当需要新图标时才在 `registry/optionIcons.ts` 补一条映射。

## 6. 一致性要求

- **禁止旁路**：状态写入只允许经选项机制（`session/update`）；不得在组件内
  硬编码选项、不得为单一选项新增协议端点、不得引入第二条选项通道。
- **禁止业务字段外泄**：前端不得出现业务字段名（`workdir` / `agent_id` /
  `provider_id` / `risk_level` / `mode` / `heartbeat`…）——这些只应出现在后端
  与「会话 store 的既有读取器」中。
- **单一入口**：根层与子层共用一个端点；新增层级不新增协议。
- **失败降级**：收集失败不阻断会话页；选项行退化为空（不得报错弹窗阻塞）。
- **顺序稳定**：新增贡献方必须遵守 order 号段；相同 order 保持注册序。

## 7. 范例（实例，非机制组成部分）

### 7.1 工作目录（`invoke` + 原生取值）

`pick = "directory"`、`bind = "metadata.workdir"`、`endpoint = session/update`。
已绑定目录且**已有对话历史**时置 `enabled = false`（换目录会混淆上下文，
须新建会话）——原前端的锁语义**上提为后端声明**，前端不再判断消息数。

### 7.2 风险等级 / 运行模式（`sub` + 状态型子项）

各档子项经 `OptionNode::session_state` 落库 `metadata.risk_level` /
`metadata.mode`；父节点 `value` = 当前档位，`value_label` = 中文标签。

### 7.3 智能体 / Model（跨插件贡献）

- agent 插件贡献 `sub` 节点：子项 = 「不使用 Agent」（`value = ""`）+ 各可用
  bundle；当前值由宿主注入的 `ctx[AGENT_ID]` 回填。
- model 插件贡献 `sub` 节点：子项 = 各启用 Provider；无可用 Provider 时仍
  下发**禁用态**节点并给出引导文案（前端零特判）。当前值由 `ctx[PROVIDER_ID]`
  或默认 Provider 回填。

### 7.4 心跳任务（`form` 复用实体表单机制 + `invoke` 命令）

- `form` 节点：字段 = 启用开关 / 空闲间隔 / 任务提示词 / 携带历史；四项
  **恒可见**（基础设置不加 `visible_when` 门控）——`DetailField` 只能表达
  「显/隐」，没有「可见但禁用」（`disabled_when` 仅存在于 `DetailAction`）；
  若按启用态显隐，未启用时表单只剩开关、看不到任何可填参数。恒可见还允许
  「先填参数、再开开关、一次保存」（`option` 绑定保存**全部字段**，关开关不丢参数）。
  `bind = "metadata.heartbeat"`（经 `OptionAction::session_state_bind`），
  定义与实体详情表单同一套 schema，前端复用唯一渲染器。
- `invoke` 节点「立即心跳」：`endpoint = worker/session/heartbeat/trigger`，
  未启用（或缺提示词）时置为只读并说明原因。

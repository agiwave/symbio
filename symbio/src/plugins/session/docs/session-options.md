# 会话选项 = 会话配置表单

状态：现行规范
范围：会话页输入区下方的「选项栏」与会话详情页的可修改项（**同一批字段、两种渲染形态**）
关联：`docs/design/vdfs.md`（`node.schema` 详情方言）、
`symbio/src/symbio_core/schemas/detail.rs`（`DetailDefinition` / `DetailField` 方言权威定义）、
`symbio/src/symbio_core/option.rs`（收集机制 `OptionVisitor` / `collect_options`）、
`symbio/src/plugins/session/options.rs`（选项宿主：会话自有字段的声明）、
`tauri/src/schemas/vdfs-form.ts`（前端方言 + 条件求值 / 紧凑取值规则）、
`tauri/src/composables/useSessionOptionBar.ts` + `tauri/src/components/chat/ChatOptionBar.vue`
（前端唯一实现）、
`docs/design/session-options-unification.md`（本规范的**来由与迁移账**）

> 本文件只写**规范与机制**。具体选项（工作目录、智能体、模型、运行模式、
> 风险等级、心跳任务……）一律属于**范例**（§9），不是机制的组成部分；
> 实例的新增/下线/调整不影响本规范的效力，机制演进也不以任何单个实例为准。

---

## 1. 目标与不变量

会话选项（工作目录 / 智能体 / Model / 运行模式 / 风险等级 / 心跳任务……）
**就是会话配置表单的字段**。同一份 `DetailDefinition` 有两个消费方：

- **选项栏**（输入区下方）——紧凑渲染形态：一排按钮 + 一层候选菜单；
- **会话详情页**（`DetailForm`）——纵向表单渲染形态。

「新建会话」与「打开会话详情」进入的是**同一份定义**，不只是同一个渲染器。

新增一个选项时，只需：

1. 后端在自己的 `traverse(available_options)` 分支注册一个 `DetailField`
   （或由宿主在 `session_option_fields()` 里声明）；
2. 前端**零改动**。

不变量：

- **后端是单一真相源**：字段、候选、条件显隐/禁用、子表单、缺省值全部由后端
  下发；前端不做任何业务枚举。
- **前端零业务字段名**：写入的键来自 `field.key`，前端只搬运。前端代码里出现
  `workdir` / `agent_id` 一类业务键即违规（测试用虚构字段名 `alpha` / `beta`
  反向钉住这条）。
- **前端零业务枚举**：候选来自 `field.options`，前端只做 `value → label` 查表。
- **单一写入路径**：落库只有 `vdfs/write`——「地址即作用域」，不再有选项专用端点。
- **选项不感知彼此**：插件之间不可见，跨插件的展示顺序用**号段约定**（§5.2）
  协调，不引入插件间直接依赖。

## 2. 三条通路

```
定义   <根>/session/<id> → node.schema               = DetailDefinition { binding: "option", … }
       <根>/session      → new_types[session].schema = 同一份
当前值 <根>/session/<id> → node.attributes.metadata  （键 = 定义里的字段 key）
落库   vdfs/write(<根>/session/<id>, {"metadata": {<字段 key>: <值>}})
```

> ⚠️ **「当前值」那一行的层级只对 Rust 侧成立**：`VdfsNode.attributes` 是
> `#[serde(flatten)]` 的，线上（`vdfs/list` / `vdfs/stat` 的 JSON，以及前端
> `services/session.ts`）看到的是**摊平**后的节点——`metadata` / `message_count` /
> `meta_tags` 都是节点的**顶层键**，没有 `attributes` 这一层。
> 写 e2e / 前端断言时读 `node.metadata`，不要读 `node.attributes.metadata`
> （`e2e/cases/t13-session-options.mjs` 就是按线上形状断言的）。

| 通路 | 载体 | 为什么在这里 |
|---|---|---|
| 定义 | `node.schema`（已落盘）/ `new_types[].schema`（草稿） | 与模型 / 智能体 / SKILL 的新建表单同一套下发方式——列表项自带 `schema`，谁手上有那个节点谁就能渲染 |
| 当前值 | `node.attributes.metadata` | 值本来就是会话状态；metadata 是它的既有归处 |
| 落库 | `vdfs/write` | 会话的写通道早已统一到 VDFS；选项不该另开一条 |

**定义只挂两处载体，`stat` 不挂**：`list`（清单，逐项复用同一份）与
`root_new_types`（草稿）。`stat` 是热路径（每次变更通知都会重读），而定义要经一次
全项目广播（含 agent 目录扫描）——挂上去等于给热路径加一次目录 I/O。

**定义与会话无关**：它只声明「有哪些字段、候选有哪些、什么条件禁用」，既不带当前
值，也不问「是哪个会话」。因此同一份定义在「新建草稿」与「已落盘会话」两个载体上
**逐字节相同**，`default` 也始终表示「后端的缺省回落」而非当前值
（`plugins/session/options.rs::build_option_definition` 用**空的请求上下文**收集，
签名里没有 `ctx`）。

## 3. 定义方言（复用 `DetailDefinition`）

选项栏不引入新方言：`DetailField` 已有的表达能力覆盖了选项栏的全部形态。

| 选项形态 | 表达方式 |
|---|---|
| 单选（风险等级 / 运行模式） | `widget = "select"` + `options`（`value → label`，可带 `description`） |
| 原生取值（工作目录） | `widget = "path"` + `pick = DETAIL_PICK_DIRECTORY` |
| 结构化子对象（心跳任务） | `widget = "form"` + `form = DetailDefinition`（子定义） + `default`（子对象缺省值） |
| 布尔（开关） | `widget = "toggle"` |
| 只读 / 禁用 | `disabled_when = DetailCondition`（由**声明**给出，不由 Rust 算成布尔） |
| 条件显隐 | `visible_when = DetailCondition` |

`options` 在**没有候选菜单**的 widget（`path` / `form`）上退化为一张
**值→标签表**：`path` 用它表达「未设置 ⇒ 未选择目录」，`form` 用它把子定义的
代表值（`title_from`）映射成「已开启 / 未开启」。这样「未设置时按钮显示字段名」
这类视觉回归不必新增方言字段就能避免。

### 3.1 条件求值的作用域（必须写明）

`binding = "option"` 时，`DetailCondition` 的求值作用域是：

```
{ ...node.attributes, ...字段值 }
```

后者覆盖前者。所以：

- `message_count`（节点属性）与 `workdir`（字段值）可以写在同一个 `all` 里；
- **草稿节点没有任何属性**。判定「已有对话历史」必须用 `truthy: true`
  （对缺席键求值 `false`），**不能**用 `not_equals: 0`（对缺席键求值 `true`
  ——那会让新建会话一上来就把工作目录锁死）。这是这条规则最容易踩的坑，
  已在 `plugins/session/options.test.rs` 钉死。

求值与紧凑取值的唯一实现在 `tauri/src/schemas/vdfs-form.ts`
（`evalDetailCondition` / `compactFieldText`），纵向表单与选项栏**共用**，防两形态漂移。

## 4. 紧凑渲染形态的取值规则

选项栏按钮上的文本（`compactFieldText`）按以下顺序取值：

1. **生效值** = 字段值 ?? `default`；
2. `widget = "form"` ⇒ 取子定义 `title_from` 指出的**代表值**；
3. 代表值为空（`null` / `""`）⇒ 查 `options` 里 `value = ""` 的项（「未选择目录」）；
4. `widget = "path"` ⇒ 取路径末段（`basename`）；
5. 否则查 `options`（值→标签表）⇒ 命中取 `label`，未命中回落字符串本身；
6. 仍未命中 ⇒ 回落 `field.label`（**最后**手段，不是默认行为）。

候选菜单的选中态用同一份「生效值」判定（`isSelected`），保证「按钮文本」与
「菜单勾选」永远同源。

## 5. 收集机制（`OptionVisitor` + `Plugin::traverse`）

### 5.1 与 `CapabilityVisitor` 的关系

选项收集是能力收集（`CapabilityVisitor` + `traverse(available_tools)`）的
**平行第二通道**：

```
session ──collect_options(parent, ctx)──▶ parent.traverse(available_options)
                                            ├─ session : 工作目录 / 风险等级 / 运行模式 / 心跳
                                            ├─ agent   : 智能体
                                            └─ model   : Model
```

- 共享的是**收集机制**：traverse 广播、上下文键（`PATH` + `OPTION_VISITOR`）、
  失败降级（父插件缺失返回空收集器；单插件失败仅记日志）。
- 不复用的是**产物语义**：能力是「可调用对象」，选项是「字段声明」。
  混装会让 `list_capability` 的消费方（LLM 工具协议）被迫理解选项结构。
- 收集器接口 `OptionVisitor`（`register_option_field(order, field)` /
  `list_option_fields()`）：按 `key` 去重（后者覆盖、保留先注册槽位），
  按 `order` 稳定排序。
- `order` **不下发**：它只是收集层排序用的号段，前端收到的是已排好序的数组
  ——数组序 = 展示序，比「各自按 order 再排一次」是更强的保证。
- 收集**无状态**：产物是「有哪些字段 + 候选有哪些」的**声明**，不含任何会话
  状态；不缓存只是因为候选集来自运行期数据（agent 目录 / provider 表）。

### 5.2 order 号段约定

插件之间不可见，故 order 采用**号段约定**（各插件在自身定义本地常量）：

| 号段 | 归属 |
|---|---|
| `10` | 工作目录 |
| `20` | 智能体 |
| `30` | Model |
| `40` | 风险等级 |
| `50` | 运行模式 |
| `60` | 心跳任务 |

新增贡献方取空闲号段；号段冲突即机制级冲突，须在规范内重新分配。

### 5.3 贡献约定

贡献方在 `traverse` 中按 `ctx[PATH] == TRAVERSE_AVAILABLE_OPTIONS` 判定是否贡献，
随后：

1. 算出**候选集**（枚举运行期数据源，如 agent 目录 / provider 表）；
2. 构造 `DetailField`（推荐用 `DetailOption` 造候选项，`icon` 给图标名）；
3. `ctx.get::<Arc<dyn OptionVisitor>>(OPTION_VISITOR)` 后
   `register_option_field(order, field)`。

⚠️ **不要回填「当前选中值」**：值随节点 `attributes.metadata` 下发，前端按
`field.key` 自取；「值 → 标签」由 `field.options` 承担。贡献方读会话状态来算当前值
是旧形态的遗留做法（也是旧形态需要宿主注入 `AGENT_ID` / `PROVIDER_ID` 的原因），
新形态下**没有理由**。唯一例外是 `default` 需要后端解析链的结论时（如 Model 的
「生效 Provider = 请求显式 > 默认」——只有后端知道默认是哪个），此时把**结论**
写进 `default`，而不是写当前值。

## 6. 前端机制分层

前端实现只有三件：一个组合式函数（机制）、一个渲染件（UI）、一个表单承载件。

### 6.1 `useSessionOptionBar`（机制核心）

- `fields`：把定义拍平成有序字段数组（`sections[].fields`）；
- `save(key, value)`：**唯一**的落库入口——
  - 会话态：`updateSession(id, { [key]: value })`
    （内部走 `vdfs/write(<根>/session/<id>, {"metadata": …})`），随后把同一份补丁
    **镜射**进会话 store（`applySessionMetadataPatch`）做即时回显；
  - 草稿态：缓冲进 `draftMetadata`，由新建流程在创建会话时一次写入。

**没有网络层**：本文件不发任何请求去「拉选项」——定义随节点来，值随节点来。

**为什么保存后要本地镜射**：后端写入后确实会广播 VDFS 变更，但那条链路是
**防抖重拉清单**（800ms）。让按钮上的当前值等一个往返才更新是可见的迟钝。
镜射与后端浅合并语义一致，后端事件随后幂等收敛。

### 6.2 `ChatOptionBar.vue`（唯一渲染件）

按 `DetailField.widget` 分派：

| widget | 交互 |
|---|---|
| `select` | 点击展开候选菜单（**一层**；候选 = `field.options`），选中即 `save` |
| `path`（带 `pick`） | 唤起原生对话框（`directory` / `file`），取值即 `save`；取消则中止 |
| `form` | 打开 `OptionFormDialog`（子定义 + 子对象值） |
| `toggle` | 就地翻转布尔值 |

未列出的 widget 在选项栏里是**惰性**的（只显示当前值）——选项栏是「一排紧凑
按钮」，不是纵向表单；需要完整表单编辑的字段请用 `widget = "form"` 包一层。

`disabled_when` 命中 ⇒ 按钮置灰且不响应点击；`busy` 期间整条选项栏禁用
（避免并发写同一会话）。

### 6.3 `OptionFormDialog` 与 `DetailForm` 的 `option` 绑定

- `DetailForm` 的绑定模式 `option`（与 `upload` / `info` 并列）：
  **预填**自传入的字段值，**保存** `emit('option-save', 字段值)`——纯字段值、
  不含 id、不经资源侧的 `write` 校验（保存目标是会话状态，不是某个资源）。
- 序列化差异：`upload` 绑定跳过 `visible_when` 不满足的字段（互斥字段不落库）；
  `option` 绑定保存**全部字段**——表单选项对应一份**完整配置对象**（如心跳任务：
  关闭开关不得丢失间隔 / 提示词）。
- 关闭入口由 `OptionFormDialog` 注入一个机制动作 `cancel`（复用统一动作区，
  避免叠加外框 header 与 `DetailForm` 已有 header 冲突）。

### 6.4 草稿态（新建会话前）

新建会话前尚无 `session_id`，没有落库目标：

- 定义来自 `<根>/session` 的 `new_types[session].schema`，与已落盘会话**同一份**，
  选项栏照常完整渲染；
- 用户的每次选择由 `useSessionOptionBar` 缓冲为 `draftMetadata`；
- 创建会话时把该补丁**整体透传**给 `createSession(metadata)`，与后端浅合并语义
  一致——**前端不解释字段名**；
- 进入真实会话后缓冲失效，值改由节点 metadata 回读。

## 7. 图标

图标名 → emoji 的映射只有一处：`tauri/src/registry/vdfsIcons.ts` 的
`fieldIcon()`（含 `DEFAULT_FIELD_ICON`）。后端只给**名字**（`folder` / `agent` /
`model` / `risk` / `run-mode` / `heartbeat`…），前端负责视觉。这是允许前端持有的
「UI 映射」，与业务契约严格分离。

## 8. 一致性要求

- **禁止旁路**：状态写入只允许经 `vdfs/write(<根>/session/<id>)`；不得在组件内
  硬编码选项、不得为单一选项新增协议端点、不得引入第二条选项通道。
- **禁止业务字段外泄**：前端不得出现业务字段名（`workdir` / `agent_id` /
  `provider_id` / `risk_level` / `mode` / `heartbeat`…）——这些只应出现在后端
  与「会话 store 的既有读取器」中。
- **失败降级**：收集失败 / 定义缺席不阻断会话页；选项栏退化为空（不得报错弹窗阻塞）。
- **顺序稳定**：新增贡献方必须遵守 order 号段；相同 order 保持注册序。
- **定义与值分离**：定义不得携带当前值，也不得依赖会话上下文——否则草稿态与
  详情态会拿到不同的定义，「两处共用一份」的保证就没了。

## 9. 范例（实例，非机制组成部分）

### 9.1 工作目录（`path` + 原生取值）

`widget = "path"`、`pick = "directory"`、`icon = "folder"`；
`options = [{ value: "", label: "未选择目录" }]` 供未设置时显示。
已绑定目录**且已有对话历史**时经 `disabled_when` 置灰（换目录会混淆上下文，
须新建会话）——锁语义由后端声明，前端不判断消息数。

### 9.2 风险等级 / 运行模式（`select`）

各档经 `options` 声明（`low` / `medium` / `high`、`interactive` / `auto`），
`default` 给出缺省档位。落库键 = `field.key`。

### 9.3 智能体 / Model（跨插件贡献）

- agent 插件贡献 `select`：候选 = 「不使用 Agent」（`value = ""`）+ 各可用 agent
  目录；`default = ""`。
- model 插件贡献 `select`：候选 = 各启用 Provider；**无可用 Provider 时下发单个
  占位候选「未配置」**（说明指向设置页），而不是为它新增「无条件禁用」这种机制。
  `default` 取**生效 Provider**（请求显式 > 默认）——这一步的解析链只有后端知道，
  故由后端把结论写进定义。

### 9.4 心跳任务（`form` 复用详情表单方言）

- `widget = "form"`，子定义字段 = 启用开关 / 空闲间隔 / 任务提示词 / 携带历史；
- 四项**恒可见**（基础设置不加 `visible_when` 门控）——若按启用态显隐，未启用时
  表单只剩开关、看不到任何可填参数。恒可见还允许「先填参数、再开开关、一次保存」
  （`option` 绑定保存**全部字段**，关开关不丢参数）；
- `default` = 子对象缺省配置（未启用 / 300 秒 / 空提示词 / 携带历史）：
  未配置过心跳的会话因此显示「未开启」而不是把字段名印在按钮上；
- 子表单字段的 `interval_seconds` 缺省值与子对象缺省值**同源**（一个常量），
  两处各写一个数字迟早会漂（已有断言）。

> **「立即心跳」按钮已取消（2026-09-18）**。它原是一个独立的命令型选项，
> 取消理由是**它的作用与「在输入框里直接发一条消息」完全重复**——心跳的实质
> 就是往会话发一轮提示词，用户想立刻做一次，在输入框发即可。
> 论证见 [legacy-route-migration.md §5.1](./legacy-route-migration.md)。

## 10. 已下线的旧机制（2026-09-23）

在这之前，会话选项走的是与其它资源**平行的一套机制**：

| 旧物 | 现状 |
|---|---|
| `symbio_core/schemas/options.rs`（`OptionNode` / `OptionAction` / `OptionType` / `OptionDisplay` / `OptionsRequest` / `OptionsResponse` / `OPTIONS_LIST` / `SESSION_STATE_ENDPOINT` / `OPTION_PICK_*`） | **已删除**（`OPTION_PICK_*` 迁 `detail.rs` 的 `DETAIL_PICK_*`；节点类型由 `DetailField` 取代） |
| `worker/session/options/list` 路由与 `handle_list_options` | **已删除**（子层懒加载一并取消：候选只有一层） |
| `OptionVisitor::register_option` / `register_batch` / `list_options` | **已删除**（只剩 `register_option_field` / `list_option_fields`） |
| `plugins/session/options.rs` 的 `*_option()` 节点构造器、`apply_chat_bar_display_defaults`、`inject_session_scope`、`find_node` | **已删除** |
| agent / model 的 `contribute_options` 旧节点分支与 `current_label` 回填 | **已删除** |
| 前端 `schemas/options.ts` / `services/options.ts` / `composables/useSessionOptions.ts` / `registry/optionIcons.ts` | **已删除**（图标并入 `registry/vdfsIcons.ts`） |

**为什么必须下线**：同一件事有两条下发通道，而没有任何守卫会因「两边说的不一样」
变红。迁移期靠「同一次广播喂两套产物」维持一致，那只是权宜——真正的解药是删掉
其中一套。完整账目见 `docs/design/session-options-unification.md`。

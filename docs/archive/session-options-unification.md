# 会话选项的 schema 化 —— 让「选项行」回到统一的 `node.schema` 下发

状态：**已实施**（S1–S6 全部落地，2026-09-23；门禁全绿：`cargo test --lib` 925 passed / e2e 13/13）
范围：会话页输入区下方的选项行（工作目录 / 智能体 / Model / 风险等级 / 运行模式 / 心跳）
关联：[vdfs.md](./vdfs.md) §7（`ext` 决定详情页面）、[vdfs-frontend.md](./vdfs-frontend.md)、
`symbio_core/schemas/detail.rs`（表单方言权威定义）、
`symbio_core/schemas/options.rs`（旧选项协议，**已删**）、
`symbio_core/option.rs`（收集机制）、
`plugins/session/options.rs`（选项宿主）、
`plugins/session/docs/session-options.md`（现行规范）、
`plugins/session/docs/legacy-route-migration.md`（旧路由审计）

> 本文只写**取舍与不变量**。落地顺序与验收见 §9（含各期的实施注记——实际落点与
> 本文原计划的差异都记在那里）；被否决的方案见 §11。

---

## 1. 问题：同一件事有两套下发机制

| | A：schema 属性（**统一体系**） | B：option 体系（**会话独有**） |
|---|---|---|
| 消费者 | model / agent / skill / mcp / setting 分区 | 只有会话页的选项行 |
| 定义载体 | `node.schema`（详情）、`new_types[].schema`（新建） | `OptionNode`（`option_type` + `action` / `children` / `form`） |
| 下发通道 | `vdfs/list` · `vdfs/stat`（**随节点一起**） | 专用端点 `worker/session/options/list`（`parent` 懒加载） |
| 当前值 | 随节点（`attributes`）或 `vdfs/read` | 贡献方在节点上算好 `value` / `value_label` |
| 状态写入 | `vdfs/write` | 专用端点 `worker/session/update`（`action.payload` + `bind` 点路径） |
| 跨插件汇聚 | 无（每个 provider 自持自己的定义） | `OptionVisitor` + `traverse(available_options)` |
| 前端实现 | `DetailForm`（唯一渲染器） | `useSessionOptions` + `ChatOptionBar` + `OptionFormDialog` + `services/options.ts` + `schemas/options.ts` |

B 的**每一格**在 A 里都有对应物，且 A 是「新增一类资源前端零改动」的那一套。B 的存在
使「新增一个会话选项」与「新增一类资源」是两条不同的扩展路径，且 B 的协议面（1 个路由 +
1 个状态端点 + 1 套节点类型 + 5 对镜像结构体）需要独立维护与守卫。

**目标**：会话详情页的**外观与交互不变**，但可修改的选项改由 `node.schema` 下发、
经 `vdfs/write` 落库；B 的端点、节点类型、前端 fetch 层全部下线。

---

## 2. 关键观察：B 的两半，一半已经是 A 的

这是本设计成立的前提，逐条已在代码中核实：

| B 的职责 | 现状 | 结论 |
|---|---|---|
| **状态落库** | `vdfs/write(<根>/session/<id>, {"metadata":{…}})` **已存在**，且与 `session/update` **共用** `Session::merge_metadata_object`（`plugin/vdfs_provider.rs` 覆盖分支，注释明写「同一份实现」） | 不需要 `SESSION_STATE_ENDPOINT` |
| **草稿态落库** | `vdfs/write(<根>/session, {create:true, metadata:{…}})` **已存在**，注释即「使用方给的 metadata（**草稿态**选择的 workdir / agent / model / mode…）」 | 草稿缓冲机制原样保留，只换出口 |
| **当前选中值** | `session_node()` 已把 `metadata` 挂在节点 `attributes` 上（`plugins/session/plugin/nodes.rs`） | 不需要贡献方各自回填 `value` |
| **新建态的定义** | `new_types[].schema` 已是「新建表单」的统一下发位，`useVdfs.draftNodeOf` 已按 `type.schema` 构造草稿节点 | 草稿选项栏**零新增机制** |
| **选项列表的下发** | 只有 `options/list` 一个端点 | **B 唯一真正独有的东西** |

也就是说：把选项行搬进 schema 体系，**要新建的机制只有「定义怎么长」这一件**；
下发、取值、落库、新建草稿四条通路都是现成的。

---

## 3. 设计：选项 = 会话配置表单的字段；选项栏 = 同一份 `DetailDefinition` 的紧凑渲染器

### 3.1 一句话

> 会话的「可修改选项」就是这张会话的**配置表单字段**；
> 选项栏是 `DetailDefinition` 的**第二种渲染形态**（与 `DetailForm` 并列，正如 `DetailShell` 是第三种）。
> 定义经 `node.schema`（详情）/ `new_types[].schema`（新建）下发，
> 当前值经 `node.attributes.metadata` 下发，落库经 `vdfs/write`。

### 3.2 三条通路

```
定义   <根>/session/<id>      → node.schema            = DetailDefinition { binding: "option", … }
       <根>/session           → new_types[session].schema = 同一份（新建草稿页）
当前值 <根>/session/<id>      → node.attributes.metadata （键 = 定义里的字段 key）
落库   vdfs/write(<根>/session/<id>, {"metadata": {<字段 key>: <值>}})
       vdfs/write(<根>/session,       {"create": true, "metadata": {…}})   ← 草稿态
```

### 3.3 现有六个选项的映射

| 现状（`OptionNode`） | 新形态（`DetailField`） |
|---|---|
| 工作目录 `invoke` + `pick=directory` + `bind=metadata.workdir` | `{ key:"workdir", widget:"path", icon:"folder", pick:"directory", disabled_when:{key:"message_count", not_equals:0} }` |
| 智能体 `sub` + 子项 `session_state("agent_id")` | `{ key:"agent_id", widget:"select", icon:"agent", options:[{value:"",label:"不使用 Agent"}, …] }` |
| Model `sub` + 子项 `session_state("provider_id")` | `{ key:"provider_id", widget:"select", icon:"model", options:[…] }` |
| 风险等级 `sub` + 子项 `session_state("risk_level")` | `{ key:"risk_level", widget:"select", icon:"risk", options:[low/medium/high], default:"medium" }` |
| 运行模式 `sub` + 子项 `session_state("mode")` | `{ key:"mode", widget:"select", icon:"run-mode", options:[interactive/auto], default:"interactive" }` |
| 心跳任务 `form` + `bind=metadata.heartbeat` | `{ key:"heartbeat", widget:"form", icon:"heartbeat", form:<原 DetailDefinition 原样> }` |

`select` 在选项栏里渲染为「按钮 + 菜单」，与今天的 `sub` 菜单**同形**——
`value → label` 的展示映射由 `field.options` 承担（今天由贡献方算 `value_label`）。

### 3.4 「前端零业务字段名」这条不变量怎么保住

今天它靠「后端下发 `action.payload` / `action.bind`」成立。新形态下：

- **键来自定义**（`field.key`），前端仍然只搬运，不硬编码 `workdir` / `agent_id`；
- 保存时前端发 `{"metadata": {<field.key>: <值>}}`，键从定义里取；
- 字段级校验由定义自带（`DetailDefinition::validate`，已有），前端不写业务规则。

**不变量完好**。反而少了两样东西：`bind` 点路径语法、`endpoint` 字符串。

### 3.5 条件求值的作用域（新规则，必须写明）

`DetailField.visible_when` 今天对表单模型求值；`info` 绑定的 `static` 字段取值来自
**节点 attributes**。选项栏两者都要：值来自 `metadata`（字段），而「已锁定」这类条件
来自节点（`attributes.message_count`）。

> **`binding = "option"` 的条件求值模型 = `{ ...node.attributes, ...字段值 }`**（后者覆盖前者）。
> 与 `info` 绑定「取值来自 attributes」同源，只是多叠一层当前编辑值。

于是工作目录的锁定条件写成 `disabled_when: { key: "message_count", not_equals: 0 }`，
**后端声明、前端零判断**（今天这条规则在 `workdir_option` 里由 Rust 算成 `enabled=false`，
迁移后语义等价）。

---

## 4. 需要给共享方言补的四件事

四件都是**通用能力**，不是为选项打的补丁——每一项在资源表单里同样成立。

| # | 新增 | 为什么是通用的 | 现状缺口 |
|---|---|---|---|
| 1 | `DetailField.icon` | 任何「带图标的字段」都需要；表单渲染器可忽略 | 选项栏要图标，字段没有这一格 |
| 2 | `DetailField.disabled_when` | **文档已自认的不对称**：`DetailAction` 有 `disabled_when`，`DetailField` 只有 `visible_when`。而「可见但不可改」是普遍需求（只读字段、锁定字段） | 选项栏需要「工作目录已锁定」 |
| 3 | `DetailField.pick`（闭集 `directory` \| `file`） | 后端唤不起原生对话框，这是**机制级原语**，与 `DetailAction` 无关；model 配置里的路径字段同样需要 | 今天只在 `OptionAction.pick` 上 |
| 4 | `DetailField.form` + `widget = "form"` | 「结构化对象字段自带子定义」——心跳是第一个实例，但 provider 高级设置是同一形状 | 今天靠 `OptionNode.form` 特例承载 |
| 5 | `DetailOption.description` | 候选项也需要一行说明（「中风险及以下自动执行；高风险需审批」）。`DetailOption` 本就定义为「**值-标签对**」，说明是同一粒度的补充；纵向表单渲染器可忽略 | 今天靠 `OptionNode.children[].description`，迁到 `select` 的候选上就没有这一格 |

> 第 5 项是实施 S2 时才发现的：原设计只列了四条，漏了候选说明。**不能省**——
> 会话选项栏的候选菜单当前会显示每个候选的一行解释（`ChatOptionBar` 的 `.mi-desc`），
> 少了它，「前端交互不变」这条硬要求就破了。

**紧凑渲染形态（选项栏）的取值规则**（`DetailField.form` 的文档里有完整表述）：
按钮文本 = `widget === 'path'` ⇒ 取路径末段；否则按 `options` 做「值→标签」查表；
`widget === 'form'` 时先按子定义的 `title_from` 链取一个代表值再查表。
这条规则让心跳的「已开启 / 未开启」得以保留（子定义 `title_from: ["enabled"]`
+ 字段 `options: [true→已开启, false→未开启]`）。

配套：`OPTION_PICK_*` 常量从 `schemas/options.rs` 迁到 `schemas/detail.rs`，
`protocol-mirror-audit` C 组的比对对象从 `options.ts` 换成 `vdfs-form.ts`（**词表不变**）。

---

## 5. 能力取舍（必须显式承认，不藏在迁移里）

| 现状能力 | 新形态 | 取舍理由 |
|---|---|---|
| `sub` 级联**任意深度**（菜单栈） | **一层**（`select` 的扁平候选） | 现存实例**全部只有一层**（workdir / agent / model / risk / mode）。为不存在的一层加 `DetailOption.children` 是给机制加复杂度换零收益。真出现两层时再加——扩展位明确（`DetailOption.children`），届时前端菜单栈代码可原样复活 |
| `invoke` 型命令选项（`action.endpoint` + `payload`） | **归 `DetailAction`** | 现存实例为 **0**（`heartbeat/trigger` 已于 2026-09-18 取消）。而「一个按钮触发一件事」正是 `DetailAction` 的语义，且它已有 `when` / `disabled_when` / `payload`。选项栏把 `definition.actions` 渲染成命令按钮即可，**零新词汇** |
| 每选项 `status`（`working` 转圈 / `error` 感叹号） | 由 `disabled_when` + 值派生 | 现存实例里只有心跳用 `status` 表达「开/关」——那本就是**值**（`enabled` 字段），不是状态。转圈/告警在选项行上从未出现过 |
| `order: i32` 号段约定（**下发到前端**） | 号段**留在后端收集层**，前端只收一个有序数组 | 跨插件排序仍需一个数（插件间不可见），但它不该是线上契约的一部分。数组序 = 展示序，是更强的保证 |
| `option_type: sub` 的**懒加载**（`parent` 参数） | 一次性内联 | 候选来自内存（agent 目录 / provider 表），懒加载省不到什么，却要多一条请求路径与一套 `parent` 寻址 |

---

## 6. 收集机制怎么办：**保留，只换产物**

`OptionVisitor` + `traverse(available_options)` 是**收集机制**，不是协议。它解决的是
「插件之间不可见，会话怎么把别家的东西汇到一处」——这个问题在新形态下**一字未变**。

改动只有一处：**产物从 `OptionNode` 变成 `(order, DetailField)`**。

```
session ──collect_options(parent, ctx)──▶ parent.traverse(available_options)
                                            ├─ session : workdir / risk_level / mode / heartbeat
                                            ├─ agent   : agent_id
                                            └─ model   : provider_id
                          → 按 order 排序 → 拍平进一个 DetailSection → 包成 DetailDefinition
```

**顺带删掉三样东西**（都是「贡献方要回填当前值」带来的）：

1. 宿主向收集上下文注入 `AGENT_ID` / `PROVIDER_ID` / `WORKDIR`（`handle_list_options` 里的
   `collect_ctx.set(...)` 三处）——**值现在由 `attributes.metadata` 提供**，贡献方无需知道会话状态；
2. 贡献方各自的 `current_label` 回填逻辑（agent / model 各一段）——`field.options` 天然是
   `value → label` 映射，前端查表即可；
3. 「宿主统一注入 `session_id` 到每个 `action.payload`」（`inject_session_scope`）——
   `vdfs/write` 的路径**就是**会话地址，作用域由地址承载，无需塞进载荷。

---

## 7. 定义挂在哪：`node.schema` + `new_types[].schema`（**不是**二选一）

两个载体服务两个消费者，都由**同一个构造函数**产出（一处真相、两处投递）：

| 场景 | 载体 | 消费者 |
|---|---|---|
| 已落盘会话的详情页 | `session_node().schema` | `Session.vue` → `ChatComposer` |
| 「新建会话」草稿页 | `<根>/session` 的 `new_types[session].schema` | `useVdfs.draftNodeOf` → `Session.vue` |

**已知代价**：定义会随会话清单的每一项重复下发（含心跳子定义，量级 ~2 KB/项）。
`session_node` 今天已经为每项带上 `metadata` / `meta_tags` / `description`——「清单一次取全」
是既有取向（见 `vdfs-session-messages.md` S8）。据此**接受**这份重复，换来的是：

- 前端**零额外请求**（今天要一次 `options/list`，现在直接从会话清单节点读）；
- `ModelChatPanel` 这类与会话树解耦的面板（只有 `sessionId`）能从 store 的会话清单拿到定义，
  不需要 `stat` 回读。

> 若将来会话数 × 定义量成为实际问题，正确的做法是在 **VDFS 机制层**加「schema 只随 `stat`
> 下发」的通用开关，而不是给会话开特例。本文不预设该开关。

---

## 8. 下线面（旧机制的完整清单）

### 8.1 后端

| 文件 / 符号 | 处置 |
|---|---|
| `symbio_core/schemas/options.rs`（整文件：`OPTION_LIST`、`SESSION_STATE_ENDPOINT`、`OptionNode`、`OptionAction`、`OptionType`、`OptionDisplay`、`OptionsRequest/Response`、`OPTION_PICK_*`） | **删除**（`OPTION_PICK_*` 迁 `detail.rs`；节点类型由 `DetailField` 取代） |
| `symbio_core/option.rs` | **保留改写**：`OptionVisitor` 的产物类型换成 `(i32, DetailField)`；`TRAVERSE_AVAILABLE_OPTIONS` 保留 |
| `plugins/session/options.rs` | **改写**为 `option_schema.rs`：`build_option_definition(&self, meta) -> DetailDefinition` |
| `plugins/session/plugin.rs` 的 `OPTIONS_LIST =>` 分发臂 | **删除**（路由下线） |
| `plugins/session/plugin/nodes.rs` 的 `session_node()` | **加一行**：挂 `schema` |
| `plugins/session/plugin/vdfs_provider.rs` 的 `new_types`（session 类型） | **加一行**：`with_schema(同一份定义)` |
| `plugins/agent/host/plugin.rs::contribute_options` | 产物改 `DetailField`（删 `current_label` 回填） |
| `plugins/model/plugin.rs::contribute_options` | 同上 |

### 8.2 前端

| 文件 | 处置 |
|---|---|
| `schemas/options.ts` | **删除**（类型从 `schemas/vdfs-form.ts` 取） |
| `services/options.ts` | **删除** |
| `constants/pluginPaths.ts` 的 `OPTIONS_PATH` / `OPTIONS_LIST` | **删除** |
| `composables/useSessionOptions.ts` | **改写**为 `useSessionOptionBar.ts`：无网络层，入参 = 定义 + 值，出参 = 保存回调（内部走 `vdfs/write`） |
| `components/chat/ChatOptionBar.vue` | **改写**：从「按 `OptionNode` 分派」变成「按 `DetailField.widget` 分派」 |
| `components/chat/OptionFormDialog.vue` | 保留（`widget = "form"` 的承载），改为收 `DetailDefinition` 而非 `OptionNode` |
| `registry/optionIcons.ts` | **并入** `registry/vdfsIcons.ts`（图标名 → 视觉只留一处；emoji 取值不变，避免视觉回归） |
| `stores/sessions.ts::applySessionMetadataPatch` | **删除**（写后重读节点，后端是唯一真相；`sessionNodeSync` 的 `updated` 收敛已覆盖跨端一致） |

### 8.3 文档与门禁

- `plugins/session/docs/cascading-options-mechanism.md` → **已改名并改写**为
  [`session-options.md`](../../symbio/src/plugins/session/docs/session-options.md)
  （《会话选项 = 会话配置表单》；旧文件名断言了一个已不存在的机制）；
- `plugins/session/docs/legacy-route-migration.md`：C 档「`options/list` 不可迁」的结论**改判**
  （它可迁，且本轮迁掉）；`§3.5 session/update` 的「前端唯一调用方」消失，需重新评估；
- `docs/reference/ROUTES.md`：删 `options/list`（**注意：本表当前并未登记它**，需一并补齐历史
  缺项说明），并核对 `session/update` 的「仅 CLI」注记；
- `scripts/protocol-mirror-audit.mjs`：删 `OptionNode` 等 5 对结构体登记与 `OptionType` /
  `OPTION_PICK_*` 两条词表登记（后者改挂 `detail.rs` ↔ `vdfs-form.ts`），**同步改
  `protocol-mirror-audit.test.mjs` 的夹具**（少铺一对基线即红，是刻意的耦合）；
- `docs/CURRENT.md`：路由变化后由门禁自动重生成（`scripts/gate.d/60-facts.mjs`）。

---

## 9. 分期实施

原则：**先加、后换、再删**。每一步结束时门禁必须全绿，且旧机制仍可用——
避免出现「一半新一半旧、两边都不完整」的中间态。

> **进度**：S1 ✅ · S2 ✅ · S3 ✅ · S4 ✅（S5/S6 不在本轮）。各期的实际落点与
> 本文的差异都记在对应小节的「实施注记」里。

### S1 · 方言补齐（纯增量，零行为变化）✅

`detail.rs` / `vdfs-form.ts` 加 §4 的四件事 + `OPTION_PICK_*` 迁位；
`formWidgets.ts` 加 `form` widget 与 `path` widget；`DetailForm` 渲染两者；
`protocol-mirror-audit` 的比对对象改挂。

**验收**：`cargo test` + `vitest` + `node scripts/gate.mjs` 全绿；行为逐字节不变。

**实施注记**：

- §4 的四件事实际是**五件**（补了 `DetailOption.description`，见 §4 的第 5 项）；
- `protocol-mirror-audit` 的 C 组**新增**一条登记（`DETAIL_PICK_*` ↔ `DETAIL_PICKS`）
  而不是把旧的改挂过去：迁移期旧家（`OPTION_PICK_*` ↔ `OPTION_PICKS`）仍在服役
  （`build_option_nodes` 与 `useSessionOptions.ts` 都还读它），撤掉守卫等于给一段
  活着的契约开天窗。C 组因此 7 → 8 张，S4 删旧家时回到 7；
- `DetailOption` 没有 `#[serde(default)]`，加字段会波及 7 处字面量构造点
  （agent / gateway / mcp / model / detail.rs 测试），已逐个补齐。

### S2 · 后端双轨（新旧并存）✅

新增 `build_option_definition`；`session_node` / `new_types` 挂上 `schema`；
agent / model 改为**同时**贡献 `DetailField`（新收集器）与 `OptionNode`（旧收集器）。
此时 `options/list` 仍然工作，前端未改。

**验收**：新增单测断言「定义里的字段 key 集合 == 会话 metadata 的选项键集合」；
`vdfs/stat` 回读能看到 `schema`；前端行为不变。

**实施注记**（三处与本文原计划不同，都是实施时才发现更好的做法）：

1. **产物并存靠同一次广播喂两个槽位**，不是两套 visitor：
   `OptionVisitor` 增 `register_option_field(order, field)` / `list_option_fields()`，
   与旧的 `register_option` / `list_options` 并列。分两次广播（或各写一份声明）
   会让两条通道有机会说不一样的话，而没有任何守卫会红。
2. **定义与会话无关，因此不需要请求上下文**：`build_option_definition()` 自己造一个
   空 `SimpleRequest` 去收集，签名里没有 `ctx`。这正是新形态相对旧形态的关键简化
   （值走 `attributes.metadata`，贡献方无需知道会话状态）——而且保证了「新建草稿」
   与「已落盘会话」两个载体上的定义**逐字节相同**，`default` 始终表示「后端的缺省
   回落」而非当前值。
3. **定义只挂两处载体，`stat` 不挂**：`list`（清单，逐项复用同一份）与
   `root_new_types`（草稿）。`stat` 是热路径（每次变更通知都会重读），而定义要经一次
   全项目广播（含 agent 目录扫描）——挂上去等于给热路径加一次目录 I/O。这也正是 §7
   原本列出的两处载体，前端从会话清单取定义即可（`ModelChatPanel` 同理）。
4. `root_new_types` 仍是 `async`：挂 `schema` 要跑一次收集。为此 `VdfsProvider::root_new_types`
   整体改成了 `async fn`（7 个实现点 + 4 个测试函数随之调整）。
5. 「无可用 Provider」这个边角情形由**单个占位候选**「未配置」表达（说明指向设置页），
   而不是为它新增「无条件禁用」这种机制——旧节点在这里是「禁用态 + 未配置」，占位
   候选保住了同一句话。

### S3 · 前端换渲染器 ✅

`ChatOptionBar` 改为吃 `DetailDefinition`；`useSessionOptionBar` 取代 `useSessionOptions`；
`ChatComposer` 传定义 + 值；`ModelChatPanel` 从 store 的会话清单取定义。
旧 fetch 层（`services/options.ts` / `schemas/options.ts` / `OPTIONS_*`）删除。

**验收**：选项栏六项全部可用（选择 → 落库 → 回读 → 回显）；草稿态六项可预选且随
懒创建一次写入；vitest 覆盖 `select` / `path` / `form` 三种 widget 的分派与保存。

**实施注记**（三处本文没写到的坑，都是**新形态下才暴露**的行为偏差）：

1. **草稿态工作目录会被锁死** —— `workdir_field` 原写
   `disabled_when: { key: "message_count", not_equals: 0 }`。草稿节点没有任何属性，
   `message_count` 缺席，而 `not_equals: 0` 对缺席键求值为 **`true`**（= 有历史）
   ⇒ 新建会话一上来就不能选目录。改用 `truthy: true`（对缺席键求值 `false`）。
   这是 §3.5 那条「条件求值作用域 `{...node.attributes, ...字段值}`」第一次真正咬人，
   已写进 Rust 文档 + 后端断言 + 前端用例（含一条「草稿态不锁」的防回归）。
2. **未设置时按钮会显示字段名** —— `path` 的「未选择目录」与心跳的「未开启」都会丢：
   紧凑渲染形态在值缺席时直接回落 `field.label`。修法**不新增方言字段**：
   让 `options` 在没有候选菜单的 widget 上退化为「值→标签表」（`path` 用
   `value = ""` 表达未设置；`form` 用子定义 `title_from` 的代表值查表），并让
   `form` 字段用 `default` 提供子对象缺省值（心跳 ⇒ `enabled = false` ⇒ 查到「未开启」）。
3. **条件求值与紧凑取值下沉为纯函数** —— 原先是 `DetailForm` 与 `ChatOptionBar`
   各写一份，两形态必然漂移。现收敛到 `schemas/vdfs-form.ts` 的
   `evalDetailCondition` / `compactFieldText`（机制唯一实现），`DetailForm` 改为委派。

### S4 · 下线旧机制 ✅

删 `options/list` 路由与 `handle_list_options`、`schemas/options.rs`、旧 `OptionNode` 贡献分支、
`applySessionMetadataPatch`；门禁与文档按 §8.3 同步。

**验收**：`node scripts/gate.mjs` 全绿（含 `dead-code-audit` 无新增死码、
`gen-current-facts --check` 通过）；`doc-find.mjs "选项"` 不再命中已删符号。

**实施注记**（三处与本文原计划不同）：

1. **`applySessionMetadataPatch` 保留**（§8.2 原计划删）。它是选项栏的**即时本地回显**：
   后端写入后确实会广播 VDFS 变更，但那条链路是**防抖重拉清单**（800ms）——
   删了会让按钮上的当前值等一个往返才更新，是可见的迟钝。它不是「第二份真相」，
   只是同一份补丁的本地镜射（浅合并语义与后端 `merge_metadata_object` 一致），
   后端事件随后幂等收敛。保留的**代价**是这条镜射必须与后端语义保持同构——
   已写进 `useSessionOptionBar.ts` 的文件头注释。
2. **`plugins/session/options.rs` 不改名为 `option_schema.rs`**（§8.1 原计划改）。
   模块内容确实从「节点构造」变成了「字段声明」，但 `options.rs` 这个名字**仍然准确**
   （它就是会话选项模块），改名只带来 `mod` 声明 / 测试文件 / 文档链接的连锁改动。
   文件名不承载「它曾经有过旧机制」这个信息——那是模块文档与 git 历史的事。
3. **规范文档改名并改写**：`plugins/session/docs/cascading-options-mechanism.md`
   → [`session-options.md`](../../symbio/src/plugins/session/docs/session-options.md)
   （旧文件名断言了一个已不存在的机制「级联选项」）。同目录 `README.md` 的登记行同步。

### S5 · 回归与守卫

- e2e：新增「会话选项：选中 → 落库 → 重启进程后仍在」用例（`e2e/cases/`，当前**零覆盖**）；
- 新增一条门禁候选：**定义字段 key 与 `Session` 解析链读取的 metadata 键必须一致**
  （防「后端加了选项但 orchestrator 不认识」这类静默失效）。

**落地情况（2026-09-23）**：e2e 用例为
[`e2e/cases/t13-session-options.mjs`](../../e2e/cases/t13-session-options.mjs)
（7 组断言，全在 gateway HTTP 边界上跑，不经过任何前端代码；已登记进
`e2e/README.md` 的用例表）。字段 key 一致性**仍在 Rust 单测里**
（`options.test.rs::field_keys_are_the_metadata_keys_the_parsers_read`），
尚未上升为门禁项——它需要一份「解析链读取的键」清单，而那份清单目前只存在于
`Session::resolve_session_params` 的实现里，硬编码进守卫会立刻与实现漂移。
**结论：暂缓上升，理由记录在此**（不是忘了）。

> 写这条 e2e 时踩到一个层级坑，已写进 `session-options.md` §2 的告示：
> `VdfsNode.attributes` 是 `#[serde(flatten)]` 的，线上节点里 `metadata` 是**顶层键**，
> 断言 `node.attributes.metadata` 会得到 `undefined`。

### S6 · `session/update` 退役（原「待决项」，2026-09-23 已落地）

原判断：S4 后前端已无调用方，只剩 CLI；CLI 需要客户端指定会话 id，而 VDFS 新建
由 provider 生成 id，因此**保留**，若要一并退役是独立决定。

**用户拍板退役，本轮落地。** 关键在于重新区分了两条被混为一谈的路：

- ❌ **给 VDFS create 请求加 `id` 字段**——真·扩协议，与「id 是存储细节」冲突；
- ✅ **让会话 provider 遵守 VDFS 已有的具名新建语义**——合规化，什么都不加。

`VdfsProvider::write` 的「两种目标形态」表与 `vdfs_service::entry::id_of` 的注释
**早已**写明：具名节点 + 不存在 ⇒ **就地创建**；只有目录自身才「名字由 provider 生成」。
会话 provider 是唯一例外（无论有无名字都自己生成 id、把名字只当标题），
对齐之后 `--session <ID>` 语义与 CLI 会话 id 格式**都不必动**——
前两轮列出的代价（用户可见的行为变化）因此全部消失。

于是 `vdfs/write(<根>/session/<id>, {create:true, metadata})` 一次调用即 upsert，
`session/update` 的职责被完全覆盖。删除面：路由臂 / `invoke_update` /
`SESSION_UPDATE` 常量 / `schemas/session/session_update.rs` / 两处调用点改写。
完整记录见
[`legacy-route-migration.md` §3.5](../../symbio/src/plugins/session/docs/legacy-route-migration.md)。

---

## 10. 不变量核对（迁移不得破坏）

| 不变量 | 是否保住 | 依据 |
|---|---|---|
| 后端是选项的唯一真相源 | ✅ | 定义 + 值 + 校验全在后端；前端只渲染与转发 |
| 前端零业务字段名 | ✅ | 键来自 `field.key`（§3.4） |
| 前端零业务枚举 | ✅ | 候选来自 `field.options`；前端只做 `value → label` 查表 |
| 单一写入路径 | ✅ **更强** | 从「选项专用端点」收敛到 VDFS 唯一写通道 |
| 只经 `services/` 出站（M-003） | ✅ | 新代码走 `services/vdfs.ts`（已有） |
| 地址常量只在 `schemas/vdfs.ts`（M-007） | ✅ | 新代码用 `vdfsSessionAddr(id)` 一族，不再有第二套地址 |
| 收集失败不阻断会话页 | ✅ | `collect_options` 的降级语义不变；定义缺失时选项栏退化为空 |
| 「点新建与选中一项进入同一详情页」 | ✅ **更彻底** | 新建态与详情态现在共用**同一份定义**，不只是同一个渲染器 |

---

## 11. 被否决的方案

**V-1 · 扩展 `OptionNode` 让它也能当 `node.schema`，但保留 `options/list` 端点。**
只统一了数据形状，没统一下发通道——用户要下线的那套机制原样还在，等于白改。

**V-2 · 把选项硬塞进 `DetailForm`（选项栏直接渲染一张普通表单）。**
选项栏是「一排紧凑按钮 + 菜单」，不是「字段标签 + 控件」的纵向表单。同一份定义、**两种
渲染形态**才是对的（正如 `DetailShell` 与 `DetailForm` 共用一份定义）。

**V-3 · 为选项栏发明第三个 schema 方言（`OptionBarDefinition`），与 `DetailDefinition` 并列。**
`DetailField` 与 `OptionNode` 的重合度极高（label / description / 候选 / 子表单 / 条件），
并列两套意味着「候选」「条件」「子表单」各要维护两份实现与两个校验器。§4 的四件事本来
就该补进共享方言——**补完之后，`DetailField` 已经能表达选项栏的全部形态**，第三套方言没有
存在理由。

**V-4 · 把定义只挂在 `<根>/session` 的 `new_types` 上，会话节点只带值。**
避免了清单里的重复下发，但让「详情页要定义」必须回读父目录节点。而 `ModelChatPanel`
与会话树是解耦的（只有 `sessionId`），拿不到父目录节点——等于给机制加一条隐式依赖。
代价换不来对等收益（§7 已量化）。

**V-5 · 保留 `action.endpoint` / `bind`，只把载体换成 `node.schema`。**
那 `vdfs/write` 的现成能力（metadata 浅合并、路径即作用域）就被绕过了，
「同一件事两条写路径」的问题原样保留。

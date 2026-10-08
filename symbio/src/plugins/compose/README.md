# Compose 插件（对话面：措辞）

「首响 / 答话 / 汇报」的措辞——**只输出文本，不做判决**。

判决的产出方见 `schemas::dialog::Verdict` 的变体表：`Answered` / `Escalate` 归
[`../classify/README.md`](../classify/README.md)，`Report` 由 `session` 自己判出。
本插件**不知道任何一个产出方存在**：它执行上游给出的判决结果，而两条调用边都从
`session` 出发 —— 插件之间没有边。

判决与措辞分开的理由不是「拆得越细越好」，而是编排层要能**执行**判决（收尾 / 进工具
循环 / 只说一句）——执行一段自由文本只能靠字符串匹配。

## 路由

清单见 [`docs/reference/ROUTES.md`](../../../../docs/reference/ROUTES.md) §Compose 插件（**权威**）：
`compose/compose`。

## 契约

入参 `ComposeRequest`，出参是 `String`（一段面向用户的文本），定义在
[`symbio_core::schemas::dialog`](../../symbio_core/schemas/dialog.rs) ——
契约跨插件（`session` 生产，本插件消费），故落在 core（ADR-023）。

出参是文本而非枚举，与 `classify` 正好相反：**判决是给编排层执行的，措辞是给人看的。**
措辞只被展示，因此不需要结构化。

**空串 = 没有对话面文本**（平凡值）：调用方（`session`）据此**不写节点**。
三条产线各自都有兜底 ⇒ 本实现**恒有文本**；空串是契约留给**其它产出方**的形态
（网关把外部客户端的 `path` 原样转发给容器，本路由可能被仓外程序直接调用）。

`ComposeRequest.context` 是 `session` 投影好的**对话线**（`conversation_view`）——
本插件不读存储，读的是投影。它与 `classify` 拿到的是**同一份规则、同一个函数**投影出来的
同一条线，否则会出现「界面看得到、插件看不到」的错位。

## 措辞怎么做的：三条产线

```text
compose(verdict)
  ├─ Report                 → 填表（事实随 snapshot 带来）             0 次 LLM 往返
  ├─ Answered{from_context} → 生成（一次静默 LLM 往返）→ 空则落变体兜底
  └─ 其余 Answered / Escalate → 模板表查码 → 未知码落变体兜底          0 次 LLM 往返
```

只有**生成**那条会碰模型，用的模型由本插件的 `model` 决定（缺席则用会话选定值）——
它与 `classify` 的分类模型**各选各的**：本插件可以用措辞更好的那一个，见配置节。

| 产线 | 文件 | 判据 |
|---|---|---|
| 模板表 | [`templates.rs`](templates.rs) | `greeting` / `thanks` / `ack` / `empty` / `clarify` / `refuse` / `needs_work` / `unclassified` —— 这些句子的**内容**是固定的，模型只会把它们写长 |
| 生成 | [`wording.rs`](wording.rs) | 只有 `from_context`（「答案已在对话里」）——那段文本只能从对话线**组织**出来，模板给不了 |
| 填表 | [`templates.rs`](templates.rs) 的 `progress_text` | 只有 `Report` —— 事实（跑了几轮 / 静默多久）已在 `ComposeRequest.snapshot` 里，缺的只是把它说成人话 |

**为什么汇报是"填表"而不是"生成"**：它的正文全部来自 `RunSnapshot` 的两个事实，
措辞是固定的。让模型来写只会多出两样东西——一次**加在用户等待期间**的往返（汇报的
全部意义是减少等待，不是延长它），以及"把 3 轮说成 4 轮"这种无从校验的漂移。
`t23-progress-report.mjs` 的 A 线用"3 轮工具恰好发 3 次请求"把这条钉住。

**填表与查表是两种数据**：表里每一行都是「与上下文无关的固定措辞」，而汇报的正文随
现状变——因此 `progress_text` 与模板表并列，而不是表里的一行（`template_for` 对
`Report` 返回 `None`，有一条用例钉着）。

**分派顺序是「先生成、生成不了落模板」**：`from_context` 在模板表里**没有行**
（有一条用例钉着），因此生成失败时它落到**变体兜底**，而不是落到某句与问题无关的模板。

**兜底随变体而不同**：`Answered` 的兜底是「好的。」（口吻是"能直接答"），`Escalate`
的兜底是「好，我来处理。」（本轮**会**干活）。用同一句会让用户以为要干活而本轮已经收尾。

**理由码不定义在本插件里**：词表是 `schemas::dialog` 的 `REASON_*`（`Verdict::reason`
的取值），生产方 `classify` 与本插件都从那里取同一批常量，**没有可漂的第二处**。
本插件因此只有一件事要保证：表里没有的码走通用模板兜底（**降级而不失效**），
以及 `from_context` 在表里**没有**行。

## 配置（本插件自己的 `PLUGIN.yml`）

| 键 | 默认 | 平凡值 | 平凡值下 |
|---|---|---|---|
| `model` | 缺席 | **缺席** | 用**会话选定的**模型 |
| `instruction` | 缺席 | **缺席** | 用内置 `INSTRUCTION` |

「要不要用它」仍是调用方的事（`SessionConfig::reply_enabled`）；这两个键管的是**怎么用它**，
与调用方无关。

**`model` 填的是 provider 条目的 id**（`<根>/model/<id>/`），不是模型名——**温度是条目的参数**
（`provider.json`），换条目就是换温度，本插件**不**再单列一个 `temperature` 键（同一件事两个
owner）。取不到（写错 / 该条目已禁用）⇒ **落回会话选定值**，答话照常生成——**降级而不失效**；
代价是 typo **没有错误信号**，因此 `compose` 把"选中了哪个模型"记进 `plugin_debug!`。

**`instruction` 只覆盖指令段**：`build_system_prompt` 里排在前面的是**注册段**
（`register_system_prompt` 收的段，与 worker 共用），它在任何时候都在。这条**不对称是有意的**：
覆盖整段会让本插件悄悄丢掉人格 / 记忆等全部上下文。

**空串与缺席同义**（`non_empty` 归一）：设置页清空输入框得到的是 `""`。这条对 `instruction`
尤其重要——若把 `""` 当"配了个空指令"，答话会失去**全部约束**且日志一切正常。

`Report` 的**触发**仍不在本插件：什么时候该打断用户（静默多久 / 跑了几轮 / 说过
几次）是**编排**的判断，归 `session` 的 `chat_loop/progress.rs` 与那四个
`progress_*` 旋钮。本插件只负责把给到的现状说成人话——**触发权与措辞权分开**，
才不会有"该说话时没人说话"。

## 落点与标记（归 `session`，不归本插件）

本插件只返回字符串；**写到哪、带什么标记**由调用方决定，见
[`../session/chat_loop/compose.rs`](../session/chat_loop/compose.rs)：

| 产物 | 落点 | `meta.surface` | `meta.exclude_from_context` | `meta.reason` |
|---|---|---|---|---|
| `Answered` 的答话 | 根级（对话线） | `"reply"` | **不设** —— 它就是这一轮的答复 | 上游理由码 |
| `Escalate` 的首响 | 根级（对话线） | `"reply"` | `true` —— 界面开场白，不进请求包 | 上游理由码 |
| `Report` 的汇报 | 根级（对话线） | `"reply"` | `true` —— 进度是给用户看的 | `"progress"` |

首响与汇报必须剔除的硬理由有两条：它们是面向用户的界面文本，不是"模型的对话历史"；
且首响紧跟用户消息，进了请求包会让线上出现连续两条 `assistant`（部分协议直接 400）。

`Report` 的理由码由 **`session` 自己产**（判决 `Report` 也由它判出，见
`schemas::dialog::Verdict` 的变体表）——本插件按判决分派、不看这个码，所以它
不在共享词表里，也不需要在表里占一行。

## 机制

- **无工具**是结构保证：本插件不注册任何 `Capability`，而模型的工具集来自
  `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `CapabilityVisitor::register` ——
  因此工具集里**在结构上不可能**出现本插件（不需要 CI 断言补偿）。
- **会调用模型，但不暴露工具**：生成是一次**内部请求**（静默出口，与上下文压缩摘要
  同款形状）——不注册 `Capability` 与不调用模型是两件事。
- **无状态、无副作用、不持有地址**：不实现 `VdfsProvider`，因此没有挂载视图；
  它**不写转写**——文本由 `session` 落库，转写只有一个写入者（ADR-020）。
- **可卸载**：不挂载本插件 ⇒ 路由 `NotFound` ⇒ `session` 按「缺插件」处理：
  `Answered` 拿不到措辞时**降级进工具循环**（不沉默），`Escalate` 没有首响，
  `Report` 只是"这次没说"且**不消耗汇报配额**（下一个轮边界还会再试）。
  这是**装配期**的状态（停用即不构造），不是运行期读一个 `enabled` 字段。

## 关联

- 判决：`../classify/README.md`
- 切分依据（为什么是两个插件）：`docs/plan/09-对话面插件拆分实施方案.md`
- 地址规则：`docs/design/plugin-route-address.md`

# Reply 插件（对话面：措辞）

「首响 / 答话 / 汇报」的措辞——**只输出文本，不做判决**。

判决归 [`../triage/README.md`](../triage/README.md)。本插件**不知道 `triage` 存在**：
它执行上游给出的判决结果（`schemas::dialog::Verdict`），而两条调用边都从 `session`
出发 —— 插件之间没有边。

判决与措辞分开的理由不是「拆得越细越好」，而是编排层要能**执行**判决（收尾 / 进工具
循环 / 只说一句）——执行一段自由文本只能靠字符串匹配。

## 路由

清单见 [`docs/reference/ROUTES.md`](../../../../docs/reference/ROUTES.md) §Reply 插件（**权威**）：
`reply/compose`。

## 契约

入参 `ComposeRequest`，出参是 `String`（一段面向用户的文本），定义在
[`symbio_core::schemas::dialog`](../../symbio_core/schemas/dialog.rs) ——
契约跨插件（`session` 生产，本插件消费），故落在 core（ADR-023）。

出参是文本而非枚举，与 `triage` 正好相反：**判决是给编排层执行的，措辞是给人看的。**
措辞只被展示，因此不需要结构化。

**空串 = 没有对话面文本**（平凡值）：调用方（`session`）据此**不写节点**。

`ComposeRequest.context` 是 `session` 投影好的**对话线**（`conversation_view`）——
本插件不读存储，读的是投影。它与 `triage` 拿到的是**同一份规则、同一个函数**投影出来的
同一条线，否则会出现「界面看得到、插件看不到」的错位。

## 措辞怎么做的：两条产线

```text
compose(verdict)
  ├─ Answered{from_context} → 生成（一次静默 LLM 往返）→ 空则落变体兜底
  └─ 其余 Answered / Escalate → 模板表查码 → 未知码落变体兜底     0 次 LLM 往返
```

| 产线 | 文件 | 判据 |
|---|---|---|
| 模板表 | [`templates.rs`](templates.rs) | `greeting` / `thanks` / `ack` / `empty` / `clarify` / `refuse` / `needs_work` / `unclassified` —— 这些句子的**内容**是固定的，模型只会把它们写长 |
| 生成 | [`compose.rs`](compose.rs) | 只有 `from_context`（「答案已在对话里」）——那段文本只能从对话线**组织**出来，模板给不了 |

**分派顺序是「先生成、生成不了落模板」**：`from_context` 在模板表里**没有行**
（有一条用例钉着），因此生成失败时它落到**变体兜底**，而不是落到某句与问题无关的模板。

**兜底随变体而不同**：`Answered` 的兜底是「好的。」（口吻是"能直接答"），`Escalate`
的兜底是「好，我来处理。」（本轮**会**干活）。用同一句会让用户以为要干活而本轮已经收尾。

**理由码不共享常量**：本插件持有词表**抄本**（[`reasons.rs`](reasons.rs)），与
`triage` 各持一份。共享常量会把「加一行数据」升级成「改 core」；代价是两侧可能漂移，
兜底是未知码走通用模板——**降级而不失效**。抄本一致性由 `templates.test.rs` 的一条
用例逐字比对守着（那也正是 `triage::reasons` 对仓内可见的唯一理由）。

## 配置

**没有自己的配置面**。「要不要用它」是调用方的事（`SessionConfig::reply_enabled`），
而措辞内部没有比「能生成就生成」更值得开关的分支——多一个旋钮就多一个没人 review
的运维事实。

## 落点与标记（归 `session`，不归本插件）

本插件只返回字符串；**写到哪、带什么标记**由调用方决定，见
[`../session/chat_loop/compose.rs`](../session/chat_loop/compose.rs)：

| 产物 | 落点 | `meta.surface` | `meta.exclude_from_context` |
|---|---|---|---|
| `Answered` 的答话 | 根级（对话线） | `"reply"` | **不设** —— 它就是这一轮的答复 |
| `Escalate` 的首响 | 根级（对话线） | `"reply"` | `true` —— 界面开场白，不进请求包 |

首响必须剔除的硬理由有两条：它是面向用户的界面文本；且它紧跟用户消息，进了请求包
会让线上出现连续两条 `assistant`（部分协议直接 400）。

## 机制

- **无工具**是结构保证：本插件不注册任何 `Capability`，而模型的工具集来自
  `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `CapabilityVisitor::register` ——
  因此工具集里**在结构上不可能**出现本插件（不需要 CI 断言补偿）。
- **会调用模型，但不暴露工具**：生成是一次**内部请求**（静默出口，与上下文压缩摘要
  同款形状）——不注册 `Capability` 与不调用模型是两件事。
- **无状态、无副作用、不持有地址**：不实现 `VdfsProvider`，因此没有挂载视图；
  它**不写转写**——文本由 `session` 落库，转写只有一个写入者（ADR-020）。
- **可卸载**：不挂载本插件 ⇒ 路由 `NotFound` ⇒ `session` 按「缺插件」处理：
  `Answered` 拿不到措辞时**降级进工具循环**（不沉默），`Escalate` 没有首响。
  这是**装配期**的状态（停用即不构造），不是运行期读一个 `enabled` 字段。

## 关联

- 判决：`../triage/README.md`
- 切分依据（为什么是两个插件）：`docs/plan/09-对话面插件拆分实施方案.md`
- 地址规则：`docs/design/plugin-route-address.md`

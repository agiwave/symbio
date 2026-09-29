# Reply 插件（对话面：措辞）

「首响 / 答话 / 汇报」的措辞——**只输出文本，不做判决**。

判决归 [`../triage/README.md`](../triage/README.md)。本插件**不知道 `triage` 存在**：
它执行上游给出的判决结果（`schemas::dialog::Verdict`），而两条调用边都从 `session`
出发 —— 插件之间没有边。

## 路由

清单见 [`docs/reference/ROUTES.md`](../../../../docs/reference/ROUTES.md) §Reply 插件（**权威**）：
`reply/compose`。

## 契约

入参 `ComposeRequest`，出参是 `String`（一段面向用户的文本），定义在
[`symbio_core::schemas::dialog`](../../symbio_core/schemas/dialog.rs) ——
契约跨插件（`session` 生产，本插件消费），故落在 core（ADR-023）。

出参是文本而非枚举，与 `triage` 正好相反：**判决是给编排层执行的，措辞是给人看的。**
措辞只被展示，因此不需要结构化。

## 机制

- **无工具**是结构保证：本插件不注册任何 `Capability`，而模型的工具集来自
  `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `CapabilityVisitor::register` ——
  因此工具集里**在结构上不可能**出现本插件（不需要 CI 断言补偿）。
- **无状态、无副作用、不持有地址**：不实现 `VdfsProvider`，因此没有挂载视图；
  它**不写转写**——文本由 `session` 落库，转写只有一个写入者（ADR-020）。
- **可卸载**：不挂载本插件 ⇒ 路由 `NotFound` ⇒ 没有对话面产出的文本，
  而 worker 的正文照旧（= 卸载前的行为）。这是**装配期**的状态（停用即不构造），
  不是运行期读一个 `enabled` 字段。

## 关联

- 判决：`../triage/README.md`
- 切分依据（为什么是两个插件）：`docs/plan/09-对话面插件拆分实施方案.md`
- 地址规则：`docs/design/plugin-route-address.md`

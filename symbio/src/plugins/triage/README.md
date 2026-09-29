# Triage 插件（对话面：判决）

「这一轮该直接回答、还是派给工具循环」——**只输出枚举，不产出面向用户的文本**。

措辞归 [`../reply/README.md`](../reply/README.md)。判决与措辞分开的理由不是「拆得越细越好」，
而是编排层要能**执行**判决（收尾 / 进工具循环 / 只说一句）——执行一段自由文本只能靠字符串匹配。

## 路由

清单见 [`docs/reference/ROUTES.md`](../../../../docs/reference/ROUTES.md) §Triage 插件（**权威**）：
`triage/decide`。

## 契约

入参 `DecideRequest`、出参 `Verdict`，都在
[`symbio_core::schemas::dialog`](../../symbio_core/schemas/dialog.rs) ——
契约跨插件（`session` 生产，本插件与 `reply` 消费），故落在 core（ADR-023）。

`Verdict` 是**闭集枚举**（`Answered` / `Escalate` / `Report`），不是自由文本；
其中的 `reason` 是**理由码**（数据），新增一类理由只加一行码表，不动 core。

## 机制

- **无工具**是结构保证：本插件不注册任何 `Capability`，而模型的工具集来自
  `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `CapabilityVisitor::register` ——
  因此工具集里**在结构上不可能**出现本插件（不需要 CI 断言补偿）。
- **无状态、无副作用、不持有地址**：不实现 `VdfsProvider`，因此没有挂载视图；
  它不回写转写（转写只有 `session` 一个写入者，见 ADR-020）。
- **可卸载**：不挂载本插件 ⇒ 路由 `NotFound` ⇒ 全部输入直接进工具循环（= 卸载前的行为）。
  这是**装配期**的状态（停用即不构造），不是运行期读一个 `enabled` 字段。

## 关联

- 措辞：`../reply/README.md`
- 切分依据（为什么是两个插件）：`docs/plan/09-对话面插件拆分实施方案.md`
- 地址规则：`docs/design/plugin-route-address.md`

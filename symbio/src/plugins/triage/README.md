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
`DecideRequest.context` 是 `session` 投影好的**对话线**（`conversation_view`）——
本插件不读存储，读的是投影。

## 判决怎么做的：两条产线

```text
decide(utterance)
  ├─ 规则表命中（问候 / 致谢 / 确认 / 空输入）→ Answered{reason}   0 次 LLM 往返
  ├─ 未命中 → 快速档分类（一次静默 LLM 往返，四选一）→ Answered / Escalate
  └─ 判不出来（无模型服务 / 响应不可解析）→ Escalate{unclassified}
```

| 产线 | 文件 | 判据 |
|---|---|---|
| 反射档规则表 | [`rules.rs`](rules.rs) | **归一化后逐字全等**——包含匹配会把「你好，帮我读一下 README」判成问候，从而**静默吞掉真实请求** |
| 快速档分类 | [`classify.rs`](classify.rs) | 四选一（`direct` / `clarify` / `refuse` / `work`），出口 `ExecEventSink::silent()` |

**顺序是判据的一部分**：先规则、后分类。反过来，规则短路省下的那次往返会被分类请求吃掉。

**兜底方向**：判不出来一律 `Escalate`（= 引入本插件之前的行为），**绝不** `Answered`——
后者会让一次分类故障表现成用户什么都收不到（静默，无错误信号）。

## 配置（本插件自己的 `PLUGIN.yml`）

| 键 | 默认 | 平凡值 | 平凡值下 |
|---|---|---|---|
| `rule_shortcut` | `true` | **`false`** | 规则表不生效，全部落快速档（慢但正确） |

配置归本插件而不是 `session`：它是**本插件的内部策略**，调用方不该知道判决内部有没有规则表
（`session` 侧只留「要不要请判决」的 `triage_enabled`）。

## 机制

- **无工具**是结构保证：本插件不注册任何 `Capability`，而模型的工具集来自
  `traverse(TRAVERSE_AVAILABLE_TOOLS)` → `CapabilityVisitor::register` ——
  因此工具集里**在结构上不可能**出现本插件（不需要 CI 断言补偿）。
- **会调用模型，但不暴露工具**：快速档分类是一次**内部请求**（静默出口，
  与上下文压缩摘要同款形状）——不注册 `Capability` 与不调用模型是两件事。
- **无状态、无副作用、不持有地址**：不实现 `VdfsProvider`，因此没有挂载视图；
  它不回写转写（转写只有 `session` 一个写入者，见 ADR-020）。
- **可卸载**：不挂载本插件 ⇒ 路由 `NotFound` ⇒ 全部输入直接进工具循环（= 卸载前的行为）。
  这是**装配期**的状态（停用即不构造），不是运行期读一个 `enabled` 字段。

## 关联

- 措辞：`../reply/README.md`
- 切分依据（为什么是两个插件）：`docs/plan/09-对话面插件拆分实施方案.md`
- 地址规则：`docs/design/plugin-route-address.md`

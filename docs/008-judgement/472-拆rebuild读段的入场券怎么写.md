---
kind: judgement
id: D-472-拆rebuild读段的入场券怎么写
title: 拆 rebuild 读段的入场券怎么写
status: active
created: 2026-11-12
---

# 拆 rebuild 读段的入场券怎么写

> 归属：边四第三格的判据前置——`D-471` 裁决 2 把「`SV-004` 今天不扩 `calls` 格」的前置写成
> 「把 `rebuild` 里读 `calls` 的两段与读 `tool_calls` 的一段拆成各自的函数」，并注明这一刀
> 要单独论证。本条就是那次论证：先量出现场比 `D-471` 写的前置**还差两格**，再裁决拆与扩
> **同批做**（2026-11-12 裁定）。施工另开批次，不在本条开工。

## 现场（今天的代码形状）

- `crates/symbio-sdk/src/projection/rebuild.rs` 的 `rebuild` 是三入参纯函数
  （事件切片 + 两份词表，产 `Vec<ChatNode>`）。读 `calls` 的两段与读 `tool_calls` 的
  一段**都是函数体内的局部绑定，不是独立函数**：`group_of_call` 从 `tool.requested`
  的 `root_id` 与 `calls[].id` 折出分组表；`node_of_call` 的主来源是 `tool.requested`
  的 `calls[].id` 与 `node_id`；紧随其后有一段命令式循环，用 `tool_calls[].id` 与
  `wire_id` 以 `or_insert_with` **补写同一张表**。两张表在 `nodes_of` 里合流。
  补写段不按事实种类过滤，只按载荷形状扫全事件。
- 两个数组的 `id` **同形异义**：`calls[].id` 是配对用的 wire id，`tool_calls[].id` 是
  工具卡的节点 id；`node_id` 只在前者，`wire_id` 与 `parse_error` 只在后者。写侧在
  `crates/symbio-plugin-model/src/port.rs` 的 `dispatch_calls` 与 `outcome_payload`：
  `dispatch_calls` 的基础三格由 `json!` 写出，`node_id` 与 `overflow_notice` 是**条件
  追加**的 `entry["…"]`。
- `SV-004` 的实现在 `scripts/shared-name-audit.mjs`：读点按 `fn`、整份文件、
  `match` 分支三种形状收窄，`ARTIFACT_READ_SITES` 今天只登记产物格的三个读点；取键
  正则只认 `payload.get("…")`、`payload["…"]`、`text_of(…,"…")` 这三种**顶层**形状；
  写方账本由 `topLevelKeys` 从 `artifact_payload` 的 `json!` 对象内文读出。

## 一个必须摆上桌的读数

`D-471` 说「拆完按函数收窄就能只提取 `calls` 的读点键」。按今天的实现，这句话**还差
两格**：`calls` 的元素键写作 `call.get("id")` 与 `call.get("node_id")`，不在 `SV-004`
的取键形状里；写方的 `node_id` 与 `overflow_notice` 是 `entry["…"]` 条件追加，也不在
`topLevelKeys` 读得到的 `json!` 对象里。也就是说，**拆读段只买到「按函数收窄后不混入
`wire_id`」这一半**；要让判据真的接上 `calls` 格，还得扩两种取键形状。

## 既有约束（本条不重审）

- `EX-007` / `D-247`：机制层不认业务键名，元素键不收成公共词表。
- `D-455`：不为判据另造一份事实源。
- `D-462`：登记范围是函数 / 分支，不是整份文件；一个事实只有一个 owner。
- `D-463`：读可以跨插件，折不能跨插件。
- `D-471` 裁决 3：`calls` 这一格不进词表。
- `crates/symbio-sdk/src/projection/rebuild.test.rs` 的确定性用例
  （`rebuild_is_deterministic` 与 `rebuild_with_mutations_is_deterministic`）要求
  同输入逐字同输出；`scripts/shared-name-audit.test.mjs` 的产物格回归用例盯的是
  读点恒真与写方改名。

## 裁决

1. **拆，边界按数组分。** `rebuild` 一个函数体里混读 `calls`（`tool.requested` 与
   `response`，同源键宇宙）与 `tool_calls`（另一个键宇宙）——两张表同形、`id` 同形异义、
   补写段不按事实种类过滤，**这一个函数装着两种载荷契约的读法，本身就是职责失守**。
   拆分的理由是这条**结构裁决**，独立于判据是否接得上而成立——不是「结构服从判据」，
   `D-455` 的纪律没有被动用。函数边界**按读的数组分**：读 `calls` 的段归一处（不分
   事实种类——两处 `calls` 同源同键宇宙），读 `tool_calls` 的补写段归另一处；
   `node_of_call` 的后置补写语义（`or_insert_with` 不覆盖）在拆出的边界上**显式表达**
   （返回值或第二次合并），合并顺序保持今天的语义（先折 requested 表、后补写），
   两个确定性用例是回归闸门——同输入必须逐字同输出。
2. **扩，同批扩两种取键形状。** `SV-004` 在**已登记的函数体**内补认两种键：① 元素键
   （循环变量 / 局部绑定上的 `call.get("…")`）；② 写方的条件追加键（`entry["…"]`）。
   **登记范围仍是函数 / 分支的闭集**（`D-462` 不破）——扩的是「函数体内键的取法深度」，
   不是收窄单位；正因为拆完后 `calls` 读点各归各的函数，函数体内提取的元素键才天然
   只属于 `calls` 格，不需要判据去做控制流分析（走法乙的死因不复现）。
3. **拆与扩同批。** 只拆不扩，收口时判据仍接不上 `calls` 格——那是一个半成品批次；
   只扩不拆，扩出来的形状要在混读的函数体上工作，必误收 `wire_id`。两半互为成立条件，
   **一个施工单元，一批做完**。`D-471` 的「不夹带」按裁决的纪律不按批次：它不夹带的
   是「答入场券」这一步，本条是裁决后的完整施工单元，与摘账批次「取字改走门面 +
   同批下调基线」同一条分批逻辑。

## 走法备查（裁决前的讨论稿保留）

| 走法 | 做什么 | 结局 |
|---|---|---|
| 甲 · 纯结构拆分 | 三段提成同文件私有函数，签名与输出不变 | **采纳**，与裁决 2 的扩同批（见上） |
| 乙 · 判据侧按段收窄 | 不动生产代码，静态识别循环与数组来源 | 否：`fnBody` 不解析控制流，形状一变判据恒真；与 `D-462` 的闭集不一致。拆完后其动机消失 |
| 丙 · 维持 `D-471` 现状 | 不拆不扩，`calls` 格登记为判据未覆盖 | 否：判据空转，失效形态只剩人工核对。前置既然可做，不做就是欠账 |
| 丁 · 给 `calls` 建公共词表 | — | `D-471` 已按 `EX-007` 与 `D-455` 两堵墙否掉，不重审 |

## 不做什么

不给 `tool.requested` 补 `FactDecl`；不把元素键收进任何词表或 `symbio-plugin-api` 的
词汇面（`EX-007`）；不把 `tool_calls` 的键并进 `calls` 的同一份账（两个事实的契约
不合一份）；不改 `rebuild` 的签名、输出与合并语义（确定性用例逐字同输出）；判据的
登记范围不扩出函数 / 分支闭集。施工另开批次，不在本条开工。

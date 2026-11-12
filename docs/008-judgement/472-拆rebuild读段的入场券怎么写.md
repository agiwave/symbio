---
kind: judgement
id: D-472-拆rebuild读段的入场券怎么写
title: 拆 rebuild 读段的入场券怎么写
status: draft
created: 2026-11-12
---

# 拆 rebuild 读段的入场券怎么写

> 待裁讨论稿。`D-471` 裁决 2 把「`SV-004` 今天不扩 `calls` 格」的前置写成
> 「把 `rebuild` 里读 `calls` 的两段与读 `tool_calls` 的一段拆成各自的函数（或等价的
> 按段登记机制）」，并注明这一刀要单独论证、不夹带在「答入场券」里。本条只把这一刀的
> **现场、约束、可选走法与待裁问题**摆齐：不写裁决，不动代码与判据，不登记任务。

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
`wire_id`」这一半**；要让判据真的接上 `calls` 格，还得扩两种取键形状。一件是机制层
投影面的结构改动，一件是判据的取法改动，`D-471` 的「不夹带」正好卡在这里：
这张入场券问的不是「拆不拆」，而是「拆与扩各自算什么、能不能同批」。

## 既有约束（本条不重审）

- `EX-007` / `D-247`：机制层不认业务键名，元素键不收成公共词表。
- `D-455`：不为判据另造一份事实源。
- `D-462`：登记范围是函数 / 分支，不是整份文件；一个事实只有一个 owner。
- `D-463`：读可以跨插件，折不能跨插件。
- `D-471` 裁决 3：`calls` 这一格不进词表；不拆 `rebuild` 是那一条的边界。
- `crates/symbio-sdk/src/projection/rebuild.test.rs` 的确定性用例
  （`rebuild_is_deterministic` 与 `rebuild_with_mutations_is_deterministic`）要求
  同输入逐字同输出；`scripts/shared-name-audit.test.mjs` 的产物格回归用例盯的是
  读点恒真与写方改名。

## 可选走法（都不在本条开工）

| 走法 | 做什么 | 会牵动什么 / 已被判过的墙 |
|---|---|---|
| 甲 · 纯结构拆分 | 把三段提成同文件私有函数，`rebuild` 的签名与输出不变 | `node_of_call` 是后置补写，拆完必须显式定义先后与合并语义，`or_insert_with` 的不覆盖语义要用返回值或第二次合并表达；`nodes_of` 的传参链跟着改。九个跨 crate 生产 / 测试调用点在类型上不受影响。判据**仍接不上**（见上节），买到的只是「将来能按函数登记」。 |
| 乙 · 判据侧按段收窄 | 不动生产代码，把读点登记从「按函数」扩成「按函数 + 数组源」，只提取源为 `payload.get("calls")` 的循环里的元素键 | 判据侧一次改完，机制层不动；但要静态识别循环与数组来源，`fnBody` 今天只做大括号配对、不解析控制流，形状一变判据会恒真。与 `D-462` 已定的「登记范围是函数 / 分支」闭集不一致。 |
| 丙 · 维持 `D-471` 现状 | 不拆、不扩，把 `calls` 格登记为**判据未覆盖的一格**，契约只靠「谁消费」那张表 | 判据空转。换来的是不为「让判据绿」去改机制层结构。写方改名 ⇒ 读方静默读空这个失效形态仍无机械拦截，只有 `D-471` 的表可人工核对。 |
| 丁 · 给 `calls` 建公共词表或补 `FactDecl` | — | `D-471` 已按 `EX-007` 与 `D-455` 两堵墙否掉，不重审。 |

## 待裁问题

1. 这一刀的理由是「`rebuild` 混读两个键宇宙本身就是职责失守」，还是「为了让判据
   能收窄」？前者是结构裁决；后者会撞 `D-455` 的同一条纪律（结构服从判据）。裁决
   要写清是哪一条，否则「拆得对不对」没有判据。
2. 若选甲：`node_of_call` 的两段合并按什么顺序、由谁持有？拆分后的函数边界是
   「按事实分」还是「按数组分」？这决定 `SV-004` 将来能登记的读点形状。
3. 若选乙：判据侧允许新增哪一种收窄形状，它与 `D-462` 的「函数 / 分支」闭集怎么
   相容——是扩闭集，还是另开一条判据码？
4. 若选丙：「判据未覆盖」这件事登记在哪一处 owner，`D-471` 的表要不要补一句
   明确的失效边界？
5. 拆与扩是否必须同批？`D-471` 的「不夹带」是按批次的纪律，还是按裁决的纪律？

## 不做什么

不改 `crates/symbio-sdk/src/projection/rebuild.rs`；不改
`scripts/shared-name-audit.mjs` 的取键形状与读点清单；不给 `tool.requested` 补
`FactDecl`；不把 `tool_calls` 的键并进同一份账；不为本条登记施工任务。裁决落定后
另开批次。

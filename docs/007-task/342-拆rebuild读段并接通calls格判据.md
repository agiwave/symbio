---
kind: task
id: T-342-拆rebuild读段并接通calls格判据
title: 拆 rebuild 读段并接通 calls 格判据（D-472 施工批）
status: doing
track: arch
prio: now
created: 2026-11-12
---

# 拆 rebuild 读段并接通 calls 格判据（D-472 施工批）

## 内容

`D-472` 的施工批，**拆与扩同批**（裁决 3）：

1. **拆**：把 `crates/symbio-sdk/src/projection/rebuild.rs` 的 `rebuild` 函数体内
   三段读点提成**按数组分界**的同文件私有函数——读 `calls` 的段（`tool.requested`
   与 `response` 两处，同源键宇宙）归一处；读 `tool_calls` 的补写段归另一处。
   `or_insert_with` 的不覆盖语义在拆出的边界上显式表达（返回值或第二次合并），
   合并顺序保持今天的语义；`rebuild` 的签名与输出不变。
2. **扩**：`scripts/shared-name-audit.mjs` 的取键形状扩两种——已登记函数体内的
   **元素键**（`call.get("…")`）与写方的**条件追加键**（`entry["…"]`）；登记范围
   仍是函数 / 分支闭集（`D-462` 不破）。读点登记补上 `calls` 格的读段函数，
   写方账本补 `dispatch_calls` 的五格（`id` / `name` / `arguments` +
   条件 `node_id` / `overflow_notice`）。
3. **验收**：`rebuild` 两个确定性用例同输入逐字同输出；`SV-004` 的 `calls` 格
   读点恒真、写方改名即红；`wire_id` 不混入 `calls` 的读点键集；相关回归全绿，
   全量门禁绿。

不做什么（`D-472` §不做什么）：不给 `tool.requested` 补 `FactDecl`；不把元素键收进
词表；不把 `tool_calls` 的键并进 `calls` 的账；不改签名、输出与合并语义。

where: `crates/symbio-sdk/src/projection/rebuild.rs`、`scripts/shared-name-audit.mjs`、
`docs/007-task/340-session退役线.md`（边四第三格读数同步）。

## 处理过程

- 2026-11-12：登记。`D-472` 裁决落定（拆与扩同批），本任务随之成立；尚未开工。

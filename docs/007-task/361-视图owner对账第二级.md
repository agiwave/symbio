---
kind: task
id: T-361-视图owner对账第二级
title: 视图 owner 对账第二级——先裁粒度，再落判据
status: todo
track: arch
prio: now
created: 2026-11-12
parent: T-357-架构补件SPRINT
where: crates/symbio-kernel/src/decl/、crates/symbio-sdk/src/plugin/reports.rs、docs/003-design/047-义务环判据与归属推导.md
exit: 第二级的判定粒度有裁决；按裁决落判据或写成「裁决不判 + 重开条件」；DE-047 的「数据就绪，校验缺席」句消失
---

# 视图 owner 对账第二级——先裁粒度，再落判据

## 内容

### 现场

[DE-047-义务环判据与归属推导](../003-design/047-义务环判据与归属推导.md) 的三级归属校验，今天的状态是「一级有、二级空、三级已落」：

| 级 | 判什么 | 今天 |
|---|---|---|
| 一 | 视图与所读载荷的写方**同 crate** | 有（`D-461` 靠搬），靠编译器与用例 |
| 二 | 视图 owner == 所读事实的 owner | **数据就绪，校验缺席** |
| 三 | 报表 owner == 折账的 owner | 已落（`crates/symbio-sdk/src/plugin/reports.rs::assert_report_owners` / `check_view_owners`） |

第二级的「数据就绪」是真的：`Frozen` 里三张 owner 表都在（`event_owners` / `fact_owners` /
`view_owners`，`crates/symbio-kernel/src/decl/mod.rs`），三个访问器也都在。但**没有任何一行
代码把「视图的 owner」与「它读的事实的 owner」对起来**——`fact_owners` 只用在「事实有没有
事件格位」一条上，`view_owners` 除冻结与自检外无人查。

### 判定

`D-463` 已经把「为什么不能硬判」说清了：判 `view_owner == fact_owner` 会误伤两类合法读
（`slo` 读 `turn.dispatch.tier`、所有读协议五格的视图），根因是**「事实的 owner」与
「载荷的写方」不是一回事**——`response` 由环落账（owner = 内核），`response.usage` 由 `model` 写。
所以这一条**不能直接施工**，必须先裁粒度：

| 候选粒度 | 代价 |
|---|---|
| 同 crate（与一级同口径） | 判不出「A 插件的视图读 B 插件的事实」，等于没加严 |
| 同插件（按 `crates/symbio-plugin-X` 归一） | 要回答「载荷格上的事实 owner 是谁」，而载荷写方没有登记位置 |
| 不判，只登记 | 保住「可解释」，但第二级永远空着 |

无论选哪个，都要回答同一件事：**载荷格的写方在哪登记**。为它造登记位置会撞 `EX-007`
（机制层不许出现插件 id）或 `D-455`（为判据造事实源）——这正是「裁决先于施工」的典型场景。

### exit

- `DE-047` 多一条 `### D-NNN`：第二级判到什么粒度，为什么，否掉的候选各是什么；
- 若裁「判」：判据落在 `assert_report_owners` 同一处（一条判据一个 owner），
  误伤的两类合法读有明确的豁免或改写，且豁免带决策号；
- 若裁「不判」：`DE-047` 的表格把该行改为「裁决不判（`D-NNN`）」并给**可观测的重开条件**
  （不是「以后再说」），`view_owners` 的消费者状态如实写明；
- `doc-link` / `doc-symbol` 绿；`cargo test --workspace` 绿。

### 不做什么

- 不为判据造事实源（`D-455`），不在机制层出现插件 id（`EX-007`）。
- 不把第二级做成「A 插件不许读 B 插件」——那会把 `slo` 读派发档这类合法聚合也拆掉。
  正确形式是「折的人必须是写的人，读的人必须经由折的人」，第二级只加固中间的缝。

## 处理过程

- 2026-11-12：登记。前置已完成——`reads` 已上移到行（`D-306` 裁决一），行级前件是一份
  与 owner 表可对齐的声明面；本条的输入就绪。

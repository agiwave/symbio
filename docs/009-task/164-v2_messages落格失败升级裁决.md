---
kind: task
id: T-164-v2_messages落格失败升级裁决
title: v2_messages 落格失败仍静默——头部注释承诺「b3 之后升级为错误」而 b3 批漏做，主读切 WAL 后该次编辑重建即丢
status: done
created: 2026-10-17
track: 登记卫生
prio: next
parent: T-156-架构机制化与旧代码清理批
where: symbio/src/plugins/session/v2_messages.rs · symbio/src/plugins/session/commands.rs
exit: 落格失败（非 NotTheWriter 的 AppendError）在 v2 会话里要么升级为可见错误（编辑回执报失败），要么保留日志口径但成文 ADR 说明为何可接受；当前「缓存已写成功，本次编辑仍然生效」的措辞必须与主读已切 WAL 的事实一致——切源后这句话是假的。cargo test + clippy + 门禁全绿
evidence: 取「升级为可见错误」这条出路（`AppendOutcome` 五态：`Landed` / `Nothing` / `Parked` / `NoAnchor` / `Failed`，处置由调用方定）。`chat_session::apply` 的步骤 ①（**写缓存之前**）把 `Failed` 转成 `PluginError::InternalError`，编辑回执经 `vdfs_provider` 的 `map_err` 变成失败响应；`NoAnchor` 仍是豁免（v1 遗留会话的读侧本就回落 `messages.json`）；写入期裁剪那半（发生在写缓存之后）保留出声口径并写明理由。判据：`v2_messages.test.rs::an_unopenable_fact_source_is_a_failure_not_an_exemption`（锁文件路径被目录占住 ⇒ `Failed`）+ `chat_session.test.rs::apply_reports_a_failed_fact_write_and_writes_nothing`（报错 ∧ 缓存里没有那个节点）；ADR-053 补记与 ADR-044 的边界。`cargo test -p symbio --lib` 1747→1749。
---

## 内容

父任务：[T-156-架构机制化与旧代码清理批](156-架构机制化与旧代码清理批.md "T-156-架构机制化与旧代码清理批")
依据：[ADR-049](../010-adr/049.md "ADR-049")（WAL 是唯一事实源，主读从 WAL 重建）·
[ADR-053](../010-adr/053.md "ADR-053")（消息变更入格；b3 行 = 主读切换 + 写侧收口）·
判据 J3（若某约束被违反时**不产生错误信号**，它必须做成强制或断言）

**问题**：`v2_messages.rs` 的头部注释在 b3 之前写下了这条**前瞻性承诺**——
「b3 之后这里会变成真源，那时失败就该升级为错误」。b3 批（`e4743c11` 主读收敛单源、
退役对账与旧读法）已落地，但这条升级**没有做**：代码仍把非 `NotTheWriter` 的
`AppendError` 归进 `Discarded` 并**静默忽略**（调用点 `Discarded => {}`）。

**为什么切源之后它从「无害」变成「数据缺陷」**：

| 阶段 | 主读 | `Discarded` 的后果 |
|---|---|---|
| b1/b2 | 缓存（`messages.json`） | **无害**——缓存仍兜着，编辑在缓存里生效 |
| **b3 后（现在）** | **WAL 重建** | **该次编辑在重建后丢失**，只剩一条 `plugin_warn` 日志，用户无感 |

而告警文案「缓存已写成功，本次编辑仍然生效」（第 467 行）现在是**假的**——
主读不再读缓存，这句对用户撒了谎。

**两条 `Discarded` 到达路径，只有一条是真缺口**（施工时别一刀切）：

| 路径 | 触发条件 | 是否真缺陷 |
|---|---|:-:|
| 无溯源锚（`try_append` 第 410-424 行） | v1 遗留会话 / 从未走过 v2 的会话 | **无害**——那类会话读侧本就回落 `messages.json`（ADR-049），缓存仍是真源 |
| `store.append` 真失败（第 464-471 行） | v2 会话 + 真实 IO/解析错误 | **真缺口**——WAL 是真源，编辑重建即丢 |

## 处理过程

- 2026-10-17 登记——起因是 [T-159-messages.json过时双写注释清理](159-messages.json过时双写注释清理.md) 的施工：
  逐条核实「双写」注释时发现 `v2_messages.rs` 头部有一段**已过时的阶段叙述**
  （「b1 阶段缓存仍是读侧真源」「b3 之后这里会变成真源」），顺着它查到这条**承诺未兑现**的升级。
  这是「登记前逐条核实」的直接产出——只清注释的话会漏掉这个真缺口。
- **不随 T-159 一并改**：T-159 的出口判据是「注释与 ADR-049 一致」，本条要动的是
  **行为**（失败升级为错误 vs 保留日志口径），需要一次设计裁决——
  升级会让 `patch_message` / `truncate_messages` 在落格失败时报错，而当前口径明确要求
  「附加事实失败不该把成功的动作说成失败」（与 `v2_facts::record_learning` 同款）。
  **两条口径冲突，须先裁决再动**，不是注释层面能解决的。
- 施工要点：先分清上表两条路径（无锚 vs 真失败），只对真失败那条做升级或成文豁免；
  保留日志口径时必须改掉第 467 行的假文案。

- 2026-10-19 **施工：取「升级为可见错误」这条出路**。先按任务自己给的那张表把两条到达路径分开——
  `NoAnchor`（v1 遗留会话 / 从未走过 v2 轮）保持豁免，`Failed`（事实源打不开 / 追加被拒）升级。
  实现上把旧的 `AppendOutcome { Landed, NotWriter, Discarded }` 换成五态（多出 `Nothing` 与
  `NoAnchor` / `Failed` 的区分），`Discarded` 这一格正是把两件代价相反的事压在一起的那一格。
  处置权交给调用方：`v2_messages` 只报结果，`chat_session::apply` 决定怎么办——它把 `Failed`
  变成 `PluginError::InternalError` 并在**写缓存之前**返回，于是「报失败」与「什么都没发生」
  是同一次返回的两个面，不存在写了一半的中间态。
  **口径冲突的裁决**：ADR-044 的「落格失败只记警告不冒泡」管的是轮末收束格（`chat.assistant.final`
  等，写发生在用户已看见输出之后），与「用户对列表的动作」不是同一种写；ADR-053 补记把这条边界
  与四态表一并成文，b3 那句漏做的升级至此兑现。
  **一处不升级**：写入期裁剪的 `message.removed` 在写缓存之后（裁剪决策在 `append_messages` 的
  临界区内产生），报错会让调用方重试、而重试等于把同一条用户消息再追加一遍；代价有界（被裁节点
  在下次重建后复活，列表变长而非数据丢失），故保留出声并把这个理由写在代码旁。
  同批修掉那句假话：`append_into` 的告警不再声称「缓存已写成功，本次编辑仍然生效」——就地落格
  在写缓存**之前**、补落在**之后**，同一句话对两条路径不可能都成立；现在只陈述发生了什么，
  由调用方决定怎么说。

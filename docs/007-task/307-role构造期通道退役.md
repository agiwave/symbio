---
kind: task
id: T-307-role构造期通道退役
title: `role` 构造期通道退役——档 2 把构造点迁到主体，档 3 删掉过渡入口
status: todo
created: 2026-10-24
track: impl
prio: later
deps: []
parent: T-299-冗余清偿批
where: symbio/src/symbio_core/turn/node_build.rs · symbio/src/plugins/loop/ · symbio/src/plugins/session/ · scripts/grep-audit.mjs
exit: 'D-147 的档 2 与档 3 走完：生产代码里 `role` 的直写只剩「构造期声明族」清单内的点，且每点注释写明地址由落库点 `attributed` 补；`with_role` / `set_legacy_role` 两个过渡入口删除、`role` 字段改 `skip_serializing`（wire 上不再出现）；测试夹具按「它模仿的是哪个面」归位——模仿落库面的写 `principal`、只为测残留格读法的留在清单里并注明。`grep -r "with_role\|set_legacy_role"` 零命中，S-014 正则收敛成「字段初始化 + 直写」两形态且基线降到清单条数，`node scripts/gate.mjs` 全绿。'
---

## 内容

D-147 把 `role` 从「第二份身份」撤成「构造期 kind 通道」，档 1 已经清掉三处真双写。
本任务接的是**通道本身**：它今天之所以还在，是因为构造函数手上只有「谁在说话」的族、
没有可路由地址——地址由会话侧在落库点 `attributed` 补。要让它退场，得先把地址送到
构造点，或者承认「这一族的族就是它全部的身份」。

### 档 2：按族推进（读完再动，别按文件刷）

| 族 | 生产直写 | 地址从哪来 |
|---|---|---|
| 助手正文 | 27 | 会话主体（`session_principal`），`attributed` 今天折的就是它 |
| 用户发言 | 12 | `person/local` 是常量——这族**不需要地址贯通**，是本档最便宜的一批 |
| 工具结果 | 3 | `meta.tool_name` 合成（`tool_principal`），三处都已具备合成条件 |
| 系统说明 | 1 | 心跳触发方，主体是 `session/autonomous` 还是会话主体需先裁一次 |
| 变量形态 | 2 | `patch_message` 的补丁直写与 `cli/render` 的合并——两处都是**读边界**，随档 3 一起收 |

顺序建议：工具结果（已有合成条件）→ 用户发言（常量）→ 助手正文（要打通签名）→
系统说明（要先裁主体）。每族完成时 S-014 都要实际下调一次，不许攒。

### 档 3：入口与夹具

- `with_role` / `set_legacy_role` 删除：调用点全部迁完后，两个入口没有存在理由
  （它们的价值是「可搜寻性」，而迁完之后没有要搜的东西）。
- `role` 改 `skip_serializing`：前提是**所有会发到前端的构造点都已写主体**——
  否则流式骨架帧（`io.rs` 的那一帧）会在前端丢掉族，被「缺省即助手」盖成错的类型。
- 夹具按面归位：`message_builder.test.rs` / `rebuild.test.rs` 模仿的是**落库面**
  （文件头自己写着），落库面的身份是 `principal`；而 `subagent.test.rs` 的
  `legacy_frame` 模仿的是**存量面**，那条读法的覆盖要留着。
- S-014 的判据正则同步收敛：只剩「字段初始化」与「直写」两形态时，棘轮数的就是
  清单长度，而不是三种混在一起。

## 判据

| # | 判据 | 承担者 |
|---|---|---|
| 1 | 每族完成时 S-014 基线同步下调（不是攒到最后） | `grep-audit` 基线格 |
| 2 | 两个过渡入口删除后 `grep` 零命中 | 一次 grep |
| 3 | `role` 改 `skip_serializing` 后，流式骨架帧在前端仍折出 `Assistant` | 前端折叠用例 + e2e |
| 4 | 折叠判据不变（`node_role()` 与 `messageRoleOf` 仍主体在先） | `chat_node::` 两条用例 + 前端 spec |

## 处理过程

- 2026-10-24：立。档 1（折序反转 + 三处真双写清零）已随 T-300 落地，本任务接档 2 / 档 3。

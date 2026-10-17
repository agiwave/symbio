---
kind: task
id: T-138-审批闸只覆盖local插件的能力
title: 风险声明与执行闸门分家——vdfs 的 High 风险工具不经确认直接执行
status: done
created: 2026-10-16
track: 插件身份与授权
deps: []
prio: now
parent: T-134-CLI会话可用性实测批
evidence: ADR-068 落地：判据搬出 local 插件到 `providers/collectors/approval.rs`（自由函数 `approval_gate` + 装饰器 `ApprovalGate`）；`DefaultToolVisitor::register` 收口点统一加壳 ⇒ vdfs/web/mcp/skill 的注册路径都过闸；`local/plugin.rs::route` 与 `web/plugin.rs::route` 兜底调同一判定；`PrefixedCapability::risk_for` 透传动态档；同批删 `local::SecureToolWrapper` 与 local 插件里的旧审批判定函数。测试：`approval.test.rs` 10 条（6 判据分支 + 4 wrapper 行为），e2e T78 双幕（交互首轮磁盘上文件仍在 + 批准后消失；auto 不产卡 + 文件仍在）。回归：cargo test --lib 1668 全绿，相邻 e2e t26/t27/t65 全过。
where: symbio/src/plugins/local/plugin.rs（`approval_gate` 的两个把守点）· symbio/src/plugins/vdfs/plugin.rs（`traverse` 里 `visitor.register(tool)` 裸注册）· symbio/src/plugins/vdfs/tools/spec.rs（`vdfs_delete` 声明 `risk: High`）
exit: `CapabilityMeta.risk` 成为**执行期**读到的量，而不是只有目录页读的宣传语：经任一注册入口进 ToolManager 的能力都过同一道 `approval_gate`；判据 = 一条 e2e，交互模式下 `vdfs_delete` 必须先出确认卡、批准后才删除，`auto` 模式下按 `PERMISSION_DENIED` 继续
---

## 内容

闸门本身是**一处判定、两处把守**的正确形状（`local/plugin.rs` 的 `approval_gate`：
风险由工具自己经 `Capability::risk_for` 回答，会话侧 `RISK_LEVEL` 给阈值，`approved: true` 放行）。
问题在覆盖面：**闸门长在 local 插件的注册路径上**——

| 注册入口 | 是否包 `SecureToolWrapper` | 结果 |
|---|---|---|
| `local/plugin.rs::traverse` | ✓（每个工具裹一层再 `visitor.register`） | local 的工具（shell / http_request…）会被拦 |
| `vdfs/plugin.rs:126` | ✗（`visitor.register(tool)` 裸注册） | vdfs 八个工具**不经闸门** |

于是「风险」在能力矩阵里是**声明**（`vdfs_delete` 写着 `High`、描述里还写着「不可逆，删除前务必确认路径」），
到执行期却没有任何人读它。实测：

```
[Tool] 请求发起: vdfs_delete args={"path":"tmp/junk-del.txt","recursive":false}
[Tool] 正常结束: vdfs_delete (耗时 3ms, 结果长度 81, pending=-)
```

`--mode interactive`、会话 metadata `risk_level: medium` ——`pending=-` 表示连一张卡都没产生，直接删了。
模型的描述词与系统的行为不一致，用户看到的「务必确认」是空的。

这不是「vdfs 该不该被拦」的答案已经定了——把 `risk` 接进执行期会**改变现有会话的行为**
（写文件、建目录都带 `Medium`，阈值一动就会频繁出卡），所以本任务要先立一条决策（ADR）
把「哪一族动作在什么阈值下需要确认」说清，再谈实现；实现面上优先做的是**形状**：
闸门应当长在统一分发点（能力进 ToolManager 的那一处），而不是长在某个插件的注册循环里。

## 处理过程

**裁决落点**：[ADR-068](../010-adr/068.md "ADR-068")「审批闸门住在能力注册的收口点」。

**改动面**（一处真源 + 两处入口 + 一处透传）：

1. **判定搬出** local 插件 → `symbio/src/providers/collectors/approval.rs`（新）。
   自由函数 `approval_gate(ctx, tool, args)` 是**唯一**判据；同文件加 `ApprovalGate`
   装饰器（`meta` / `name` / `risk_for` 原样透传，`execute` 先过判定再落内层）。
2. **收口点加壳** `symbio/src/providers/collectors/tool_visitor.rs::register`：
   每条进 `IndexMap<String, Arc<dyn Capability>>` 的能力都过 `ApprovalGate::new(_)`——
   这一路覆盖 vdfs / web / mcp / skill / session 心跳工具全部注册入口。
3. **local 插件的旧写法退役**：`SecureToolWrapper` 结构体 + 内联判定 +
   `risk_level_from_ctx` + `confirm_prompt_payload` 一次删；`local/plugin.rs::route`
   兜底改调 `providers::approval_gate`；`local/plugin.rs::traverse` 交裸工具给 visitor，
   注册面加壳由收口点负责。同批删 `SecurityPolicy::check_tool_approval_needed` <!-- doc-symbol-allow: 历史名——T-138 已随旧写法退役删除，本行记录删除动作 -->
   （纯比较 + `_tool_name` 都忽略；留在 local 里就是「判据住在 local」的假象入口）。
4. **web 兜底加闸** `symbio/src/plugins/web/plugin.rs::route`：注册表未命中路径下
   `http_request`（High）过去直接被 route 走；本批与 local 同形状，route 也调判定。
5. **子 Agent 改名壳透传动态风险** `symbio/src/plugins/agent/host/scope.rs`：
   `PrefixedCapability::risk_for(args) = inner.risk_for(args)`。默认实现读
   `self.meta().effective_risk()`（静态档），不透传则 shell 的按命令分级在包装层下丢失。

**判据形状**（写进 `approval.rs` 头部文档 + ADR 决策 3）：
- `risk_for(args) ≤ threshold` ⇒ 放行；
- `args.approved == true` ⇒ 放行（t27 恢复链路：approve action ⇒ 重放带 `approved:true`）；
- 拦下：`mode=auto` ⇒ `PERMISSION_DENIED` 友好错误，**不产卡**；其余 ⇒ `NEEDS_APPROVAL` 载荷交编排层建 `user_prompt` 节点。

**测试**：

- 单测（新）`symbio/src/providers/collectors/approval.test.rs` 10 条：
  覆盖 6 条判据分支（High/Medium/Low × 三种阈值 + approved:true + dynamic risk + mode=auto），
  与 4 条 wrapper 行为（meta/name/risk_for 透传 + 放行落内层 + 拦截不落内层）。
- e2e（新）[T78](../../e2e/cases/t78-approval-gate-vdfs.mjs) 双幕：
  A 幕交互 ⇒ 首轮**磁盘上文件仍在**（闸门生效的分水岭，修复前直接被删）→
  `resumeSession(…, 'approve')` → **磁盘上文件消失**（`approved:true` 穿过闸门）；
  B 幕 auto ⇒ 会话不产卡（`user_prompt(waiting_user_action)` 缺席）+ tool 结果正文
  含「权限不足…自动模式」+ **文件仍在**。

**回归**：`cargo test --lib` 1668 绿；相邻 e2e `t26 / t27 / t65` 全过（MCP 工具默认 Medium ⇒ 阈值 Medium 下不拦；
`web` 的 `risk_level: high` 会话照旧放行 http_request 的 SSRF 校验路径）。

**行为改变面**：默认阈值下 `vdfs_delete` (High) / `http_request` (High) 从此必出审批卡
（`interactive` 模式）或 `PERMISSION_DENIED`（`auto` 模式）——**这是本任务出口判据
要的形状**，不是回归。medium 阈值下写类工具（`Medium`）仍放行；把阈值调到 `low` 才会
波及 Medium 一族，那属于会话侧授权决策。

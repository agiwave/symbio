---
kind: task
id: T-138-审批闸只覆盖local插件的能力
title: 风险声明与执行闸门分家——vdfs 的 High 风险工具不经确认直接执行
status: todo
created: 2026-10-16
track: 插件身份与授权
deps: []
prio: now
parent: T-134-CLI会话可用性实测批
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

（未开始）

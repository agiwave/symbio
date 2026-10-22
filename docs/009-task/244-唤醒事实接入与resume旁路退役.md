---
kind: task
id: T-244-唤醒事实接入与resume旁路退役
title: PL-017 步 4–5 收口——审批/插话/中止唤醒进 await 行，resume.rs 旁路退役
status: todo
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-240-唤醒事件时间戳取代墙钟]
where: symbio/src/plugins/loop/resume.rs · symbio/src/plugins/loop/turn_exec.rs · symbio/src/plugins/loop/loop_rows.rs · symbio/src/symbio_core/decl/
exit: 'PL-017 §4 步 4 与步 5 落地：审批通过 / 用户补充 / 中止各落一条唤醒事实并进入 `await-*` 行的 `until`（PL-017 判据 6 三用例）；`resume.rs` 承担的恢复 / 审批 / 问答三条旁路逐条迁进行表后整文件退役（`turn_exec.rs:785,800,840` 的 `resumed` 旁路一并消失）；`close-turn` 收束行进表（owes `turn.has_final`，until 收束格），收束不再只靠执行体内联；`resumed` 与续写幕两处「跳过无债守卫」的特例由行表语义接管或有名有姓留档；唤醒接好后至少一个生产端口 `max_attempts > 1` 且 retry 路径有真实（非合成 PortDecl）用例；全量门禁与 e2e 绿。'
---

## 内容

父任务：[T-236-义务环符合性收口批](236-义务环符合性收口批.md)；父设计：[PL-017-端口细粒度化](../004-plan/017-端口细粒度化.md) §4。

2026-10-22 对账：三端口 `max_attempts` 全为 1、retry 仅合成用例；`resume.rs`
三路旁路仍在且 `resumed` 在派发器里走特殊分支——「外部事件唯一进环通道」仍是
内存旁路而非事实。步 4/5 是 PL-017 已声明未收口的两步，本任务销掉。

## 验收口径

1. PL-017 判据 6 三场景各一条用例：审批落 `approval.*` 唤醒 await-tools；插话落 `user.message` 唤醒 await-turn；中止落收束格并关账。
2. `resume.rs` 的三条私有路径全部消失（文件删除或 0 旁路），`turn_exec.rs` 无 `resumed` 分支——grep 断言进门禁或测试。
3. `close-turn` 行在表且 `chat.assistant.final` 经它收口（行名与事实对进 facts）。
4. 至少一个生产端口 `max_attempts > 1`，retry 用例使用真实 `PortDecl`（非合成）——销「重试路径只有合成覆盖」。
5. CLI 复验（含一次审批卡中断→唤醒→续跑的实测会话）+ 门禁 + e2e 全绿。

## 处理过程

- 2026-10-22 登记（PL-016 三面对账：方案 → 代码 → CLI 实证，见 T-236 内容节）。

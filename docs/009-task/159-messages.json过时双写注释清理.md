---
kind: task
id: T-159-messages.json过时双写注释清理
title: messages.json 写侧已停（ADR-049）而 10+ 处注释仍声称「双写进 messages.json」——假事实注释清理
status: done
created: 2026-10-17
evidence: 全仓 grep「双写」清零——假事实 10 处改写为现行语义（chat_loop.rs:821/979、v2_exec.rs:64/409/1605、chat_loop/state.rs:336、orchestrator/consume.rs:363、error.rs::PluginError::Aborted、v2_exec.test.rs:8/644）+ 变更史措辞 6 处去「双写期」化（turn_runner.rs:186/221、chat_loop.rs:578、rebuild.rs:290、v2_exec.rs:1032、symbio_core/event/mod.rs:100）+ 测试残留 1 处（v2_exec.test.rs:663）；store/mod.rs 的 v1 回落说明保留；v2_messages.rs 告警文案与主读已切 WAL 的事实对齐（升级裁决另立 T-164）；cargo test 1726 通过 + clippy -D warnings 全绿
track: 登记卫生
prio: now
parent: T-156-架构机制化与旧代码清理批
where: symbio/src/plugins/session/chat_loop.rs · symbio/src/plugins/session/v2_exec.rs · symbio/src/plugins/session/chat_loop/state.rs · symbio/src/plugins/session/orchestrator/consume.rs · symbio/src/plugins/session/round_hook.rs · symbio/src/plugins/session/turn_runner.rs
exit: 全仓 `grep -rn '双写进.*messages\.json\|双写.*messages\.json' symbio/src` 的生产代码命中点每条要么**删除**要么改写为真实语义（「随 partial 入格，事实源是 WAL」）；只描述 v1 遗留读侧回落的注释**保留**并显式标注 ADR-049；cargo test + clippy + 门禁全绿
---

## 内容

父任务：[T-156-架构机制化与旧代码清理批](156-架构机制化与旧代码清理批.md)
依据：[ADR-049](../010-adr/049.md "ADR-049")（WAL 是会话唯一事实源，SessionStore 降为清单）·
判据 J3（注释描述的行为与实际不符 = 静默的假事实）

**问题**：这是一次**判据修正**的产物——登记本批时的初判是「`messages.json` 双写是没删干净的旧路」，
逐条核实后发现**写侧早已停写且有测试守着**，真实欠账是**注释还在描述那个已经不存在的行为**。

两类注释必须区分处置：

| 类别 | 例 | 处置 |
|---|---|---|
| **假事实**（声称双写仍在发生） | `chat_loop.rs:821`「随 `partial` 一路带到 `persist_failure`，双写进 `messages.json`」 | **改写**为真实语义：事实经 `partial` 入格（WAL 是事实源），`persist_failure` 定稿的是转写镜像 |
| **合法的 v1 回落说明** | `store/mod.rs:524`「有 WAL 就从它重建；WAL 不在才回落 `messages.json`」 | **保留**——这是 ADR-049 的有意设计（v1 时代落的 `messages.json` 仍是那批会话的唯一记录） |

**为什么这条是「清理旧代码」而非「改逻辑」**：不动任何执行路径，只让注释与 ADR-049 一致。
它的风险恰恰在于**看起来像死代码**——若按初判直接删 `partial` 传递链路，会把「半截产出必须入格」
这个正确行为删掉（`v2_exec.rs:1605` 明确写了：不入格，用户中断前看到的那半句话就消失）。

## 处理过程

- **逐条判定而非批量替换**：`grep` 命中 69 处，绝大多数是合法的（v1 回落说明 / 测试注释 /
  描述「切源后会怎样」的前瞻性说明）。只清**声称双写在进行时**的那批。
- **不要动 `partial` 参数链路**：它是中止时半截产出的事实承载（`failure.rs:96-101` 的参数文档
  写得是对的），欠的只是下游几处「双写进 messages.json」的措辞。
- 参照 [T-153-旧文档与注释清理](153-旧文档与注释清理.md) 的同款方法：先分类（三个「轮 / Turn」是三个东西），
  再只动确实过时的那类。本条同理——先分「假事实 vs v1 回落说明」，再动前者。

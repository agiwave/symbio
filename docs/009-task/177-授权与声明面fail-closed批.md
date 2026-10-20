---
kind: task
id: T-177-授权与声明面fail-closed批
title: 授权与声明面 fail-closed 批——把两处 fail-open 默认翻成「少一行是一条拒绝」
status: doing
created: 2026-10-20
track: plan
prio: now
deps: []
where: symbio/src/symbio_core/authz.rs · symbio/src/symbio_core/governance/mod.rs · symbio/src/plugins/loop/tools/tool_executor/decls.rs · docs/010-adr/099.md
exit: '`matrix_for` 对未登记的 `session/<id>` 不再派生主智能体能力集（未登记 = 拒，窄行成为唯一入口）；`GrantsOverlay::is_revoked` 认不出的能力名告警并按已收回判；工具自声明行为位漏声明有基线判据钉住（工具在册而基线无行 ⇒ 红）；ADR-099 单文档登记这次语义收窄的理由与被推翻的旧口径出处；`authz.test.rs` 派生用例改写且保留「少一行是一条拒绝」反向用例；全量门禁与 e2e 全绿'
---

## 内容

父设计：[[PL-015]] §4.2（漏改无声的扩充点）· [[PL-014]] §2.2（一源）· [ADR-069](../010-adr/069.md "ADR-069")（主体派生）

**起因**：一份架构质量审计（2026-10-20，临时分析稿）点名三处「接线了但判据方向反了」，
核验后属实，且都是**一处默认值养着一整类问题**：

| 处 | 现状 | 谁会红 |
|---|---|---|
| `authz.rs` `matrix_for` 派生分支 | 未登记的 `session/<id>` 拿到主智能体**整套**能力（含 `reply.*`） | 无人：新角色忘配窄行 ⇒ 静默超权 |
| `governance` `is_revoked` | 认不出的能力名 ⇒ 未收回（放行） | 无人：载荷笔误变成一次静默放行 |
| `decls.rs` 行为位自声明 | 工具漏声明 ⇒ 三道闸不命中 | 只有 e2e 兜，基线不管 |

**裁决**（2026-10-20 与维护者当面裁决）：授权默认**翻 fail-closed**，立
[ADR-099](../010-adr/099.md "ADR-099")。被推翻的旧口径是
[`authz.rs`](../../symbio/src/symbio_core/authz.rs "authz.rs") 派生分支的
「身份分层 ≠ 权限分层（S08 §4 平凡值）」——它保证的是「子智能体写得了收束格」这条
**可用性**，代价是把「少一行」变成超权而不是拒绝。新口径：可用性由**窄行登记**承载
（谁要写收束格，谁登记一行），安全性由**默认拒绝**承载。

## 处理过程

- 2026-10-20 立批（当日开工）。三处各自独立成提交：① 授权默认翻向 + ADR；
  ② 撤销判据同向；③ 行为位基线判据。三处都不新增机制键、不新增 trait。

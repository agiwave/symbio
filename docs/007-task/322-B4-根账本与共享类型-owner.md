---
kind: task
id: T-322-B4-根账本与共享类型-owner
title: Phase B4：根账本只降不升，共享类型登记生产者 owner
status: todo
created: 2026-10-26
track: impl
prio: next
deps: [T-321-B3-装配清单与身份由-manifest-生成]
parent: T-316-内核边界与扩展局部性收口
where: symbio/src/symbio_core/mod.rs · docs/004-ref/ · scripts/core-export-audit.mjs
exit: '`core/mod.rs` 手改 = 0（生成或消除）；只为「跨模块可达」而存的根导出清零；共享类型在 manifest 有「生产者 → 消费方」owner；根账本条目数只降不升。'
---

## 内容

1. 根账本按 manifest 生成（或随 kernel 门面收窄而消失）；`--check` 防漂移。
2. 删除「core 内没人用、只为插件可达」的导出；C-003 的 0 / 单消费方存量继续降。
3. 共享类型登记生产者 owner；`contract` 模块不提前造（D-180），拆 crate 时才出现。

## 处理过程

- 2026-10-26：立任务。B4 不搬代码，只收窄可达面与登记 owner。

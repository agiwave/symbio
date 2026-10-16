---
kind: task
id: T-151-Typing占位节点
title: Typing 占位——前端本地合成、同父非-typing 兄弟到达即 removed
status: todo
created: 2026-10-16
track: frontend
deps: [T-147-ChatNode协议改名与字段瘦身]
prio: next
parent: T-145-前端机制化批
where: tauri/src/composables/useChatConnection.ts · tauri/src/registry/messageTypes.ts · tauri/src/components/
exit: 协议 `NodeKind::Typing` 已上线（在 T-147 里）；前端按"父在 streaming 且尚无子"合成一个 typing 兄弟；同父下一非-typing 兄弟到达即把 typing 就地标 `status=removed`；单测覆盖合成 / 替换 / 父收束三态；后端**不发** typing 帧（若将来要显式提示，走 `meta.typing_hint`，本任务不做）
---

## 内容

父设计：[PL-013 §7](../004-plan/013-前端机制化.md "PL-013-前端机制化")

用户诉求「Typing 临时 / 叶子节点；同级别下一个节点出现后自动替换」。协议侧 `Removed` 终态今天就有，
本任务只在**前端加一条合成规则**——不改后端。

## 处理过程

（未开始）

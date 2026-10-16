---
kind: task
id: T-151-Typing占位节点
title: Typing 占位——前端本地合成、同父非-typing 兄弟到达即 removed
status: done
created: 2026-10-16
evidence: tauri/src/registry/messageTypes.ts::synthesizeTypingChildren · tauri/src/registry/__tests__/messageTypes.spec.ts · tauri/src/composables/useChatConnection.ts::visibleMessages · tauri/src/components/message/TurnGroupNode.vue
track: frontend
deps: [T-147-ChatNode协议改名与字段瘦身]
prio: next
parent: T-145-前端机制化批
where: tauri/src/registry/messageTypes.ts · tauri/src/composables/useChatConnection.ts · tauri/src/components/message/TurnGroupNode.vue
exit: 协议 `NodeKind::Typing` 已上线（在 T-147 里）；前端按"父在 streaming 且尚无子"合成一个 typing 兄弟；同父下一非-typing 兄弟到达即把 typing 就地标 `status=removed`；单测覆盖合成 / 替换 / 父收束三态；后端**不发** typing 帧（若将来要显式提示，走 `meta.typing_hint`，本任务不做）
---

## 内容

父设计：[PL-013 §7](../004-plan/013-前端机制化.md "PL-013-前端机制化")

用户诉求「Typing 临时 / 叶子节点；同级别下一个节点出现后自动替换」。协议侧 `Removed` 终态今天就有，
本任务只在**前端加一条合成规则**——不改后端。

## 处理过程

后端不发 typing 帧，「生成中」占位完全在前端。既有代码里这个占位已由两个互斥来源实现并
测试：`TurnGroupNode.isPending`（回合 streaming 且无子 → `TurnPending`）与 `ModelChatPanel
showTyping`（会话在跑但流里无在途节点时兜底）。本任务把**「该不该有占位」这条判据**从组件
内联上收到注册表（唯一来源、纯函数可测），并按 exit 让转写树里真的出现一个 typing 节点。

### 落点

- **`messageTypes.ts::synthesizeTypingChildren(visible)`**（纯函数）：从可见扁平表为
  **streaming 且无真实子的助手回合骨架**（`turn` = skeleton+run）合成一个 typing 派生子
  （稳定 id `typingIdOf(host)`、`type=typing`、`status=streaming`）。判据 `isTypingHost`
  也在此。**只认 run 不认 tool**——工具有自己的在途信号（运行中/秒数），挂「生成中」是错位。
  三态全由这一个谓词推出：合成（空回合 streaming）／替换（真实子到达即不为「无子」）／
  收束（不再 streaming）。移除 ⟺ 谓词转假（派生节点从不落地，无需在 store 标 `status=removed`）。
- **`useChatConnection.visibleMessages`**：可见过滤后 `push` 合成 typing 子——随
  `messageTree` 建进树，从不写 store（派生态）。
- **`TurnGroupNode`**：等待态判据改用「剔除 typing 派生子后的真实子数」，
  于是合成节点在树里存在、而 Turn 的呈现仍是那份共享的 `TurnPending`（视觉零变化，
  既有 streamFlow / MessageNode 形态① 断言照绿）。

### 验证

`messageTypes.spec.ts` 追加 typing 合成 6 例（合成 / 替换 / 收束 / 正文叶子不合成 /
tool 不合成 / 已有 typing 子不算真实子 + `typingIdOf`·`isTypingHost`）；
`useChatConnection.spec.ts` 追加 3 例（streaming 空回合 → 树里挂 typing / 真实子到达让位 /
收束消失）。`vitest` 全绿（914）、`vue-tsc` 净、`protocol-mirror-audit` 0 错（typing 节点
只用既有线字段，无新增）。

### 遗留（浏览器 QA）

`typing` 呈现槽当前 `renderer: 'fallback'`（[T-147c-ii] 起有意如此，专属渲染器落地前走兜底）。
本次经 TurnGroupNode 复用 `TurnPending` 出图，故回合内观感不变；若要让 typing 节点在
**任意**消费者处都用专属渲染器（而非 fallback），属新增 Vue 渲染器 + 观感核对，需真实浏览器
验证，本环境不可无头确证，留作后续 polish。

---
kind: task
id: T-024-model端口迁出
title: model 端口迁出：provider 由 model 插件注册，loop 只按端口名调用
status: todo
track: arch
prio: next
created: 2026-11-02
parent: T-001-能力全面拆分
deps: [T-023-世界端口注册面]
where: `crates/symbio-plugin-model/src`, `crates/symbio-plugin-loop/src/chat_loop/decide.rs`, `crates/symbio-plugin-loop/src/turn_exec/call.rs`
exit: `symbio-plugin-model` 在 `traverse` 里注册 `port.model` 的 `PortProvider`；`loop` 内 `capability_model_resolve` + `ProviderLlmAdapter::for_turn` 的具体装配清零，只按端口名取提供者；端口缺席 ⇒ 落一条失败事实并中止该往返（不再把 `resolve_provider = None` 伪装成 `Escalate{unclassified}`）；停用 model 插件 ⇒ 端口行消失、调用响亮失败、不静默回退；装 / 不装插件时端到端行为逐字不变；`EL-001` 的 16 行与 `EL-002` 的 1 条出参通道不变；迁移提交挂 `Migration: model-port`，机制面新增行为 0
---

# model 端口迁出：provider 由 model 插件注册，loop 只按端口名调用

## 内容

`D-298` 的 M1：`model` 端口的**世界实现**从 `loop` 迁到 `symbio-plugin-model`。`loop` 保留
环机制——`PortDecl`、`Call` 幂等键、`request/sent/response/failed` 记账、租约与重试读数；
不再知道 provider 怎么解析、怎么装配适配器。

今天的具体装配在两处：`chat_loop/decide.rs` 的 `resolve_provider`（`capability_model_resolve`）
与 `ProviderLlmAdapter::for_turn`，`turn_exec/call.rs` 的同形装配。迁出后这两处只问端口名
`model` 的提供者。

## 处理过程

1. `symbio-plugin-model` 在 `traverse` 里把 provider 包成 `PortProvider` 并 `put` 进
   `port.model`；模型族（生效位 / 目录）原样保留，两者不是同一份事实。
2. 同一提交删掉 `loop` 里的 `resolve_provider` 与适配器装配，换成按端口名取提供者；
   `loop_rows` 的 `announce-model` / `dispatch-model` / `await-model` 三行与 `PortDecl` 不动。
3. 缺席分支**先写用例**：0 行 ⇒ 落失败事实 + 中止；断言日志里不出现
   `Escalate{unclassified}`，也不出现「本轮当作没打过模型」的静默路径。
4. 跑端到端与停用 / 启用 model 插件的反向用例；`EX-007` 核对机制面新增行为 0。
5. 与 `T-014` 对齐：`EL-001` / `EL-002` 读数不动；若工具路径读点因本批连带变化，只许降，
   并在同一提交改 `EL-003` 基线（本批预期不变）。

## 边界

- 不迁 `batch` / `parallel` / `gates` / `breaker`；它们不是模型端口的事。
- 不改 `PortDecl` 的租约 / 重试数字；迁出的是「谁装配 provider」，不是环的记账。
- 不建新的模型代理插件；提供者就住在 `symbio-plugin-model`。

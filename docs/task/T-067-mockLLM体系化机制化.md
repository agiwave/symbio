---
kind: task
id: T-067-mockLLM体系化机制化
title: mockLLM 体系化 / 机制化完善：真实 LLM 服务与真实网络环境的异常形态注入
status: done
created: 2026-10-13
track: quality
deps: []
prio: now
exit: mock-llm 的异常形态收成分级机制（场景字段配置驱动，不为单个用例手写特判）：响应流中断 / 429 限流语义 / 响应内容不规范 / 网络延迟与停顿每类可注入；每类配一条 e2e 用例钉「注入会红 / 受影响行为按纪律收敛」；既有用例不返红；正常形态逐字节行为不变
evidence: e2e/mock-llm.mjs（新增注入字段 latencyMs / chunkJitterMs / stallAfter+stallMs / dropAfter / malformed+malformedAfter，file 头写清注入语义；plan 级 rateLimit: {limit, windowMs} 滑动窗口 429 限流（retry-after + x-ratelimit-*，被拒不占配额）；/_requests 增记响应 status）；e2e/helpers.mjs MockLlm 第三选项 rateLimit 透传；四条黑盒用例各钉一类：t49-stream-drop（断流轮可见失败 + 恢复轮照常）、t50-ratelimit-429（双探针占配额、限流后冷却自动恢复）、t51-malformed-stream（三种脏帧不打破对话）、t52-network-timing（延迟/抖动/停顿下完整回复）；验证：e2e 门禁 49/49（含既有 45 条不返红，2026-10-13）
---

## 内容

[e2e/README](../../e2e/README.md) 的 mock-llm 已是 OpenAI Chat 兼容的流式 mock（SSE、场景编排
`match` / `afterTool` / `once` / `times` / `chunks` / `failTimes` / 静态 `status`、
逻辑请求指纹、`/_requests` 读回），但对照「尽量模拟真实 LLM 服务与真实网络环境」
的目标，异常形态还有整套缺口——真实世界的 LLM 服务坏起来不止 HTTP 500 一种：

| 缺口 | 真实形态 | 现状 |
|---|---|---|
| 响应流中断 | SSE 半途断开（无终止帧、无 `[DONE]`）、连接 destroys、headers 后停顿 | 无机制（`T5` 只有 HTTP 层错误） |
| 429 限流语义 | 配额窗口内超额→ 429 + `retry-after` / `x-ratelimit-*` 头；窗口滑动后恢复 | 只有静态 `errorStatus: 429` 头面，无限流语义 |
| 响应内容不规范 | `data:` 行坏 JSON、截断的 chunk JSON、杂散字节、非规范 SSE 行 | 无注入口 |
| 网络环境拟真 | 首字节延迟、慢流（抖动）、中期停顿（idle stall） | 只有恒定 `chunkDelayMs` |

纪律约束：

- **正常形态零漂移**——既有 45 条用例依赖 `chunks` / `toolCalls` / `reasoning` /
  逐字节 id 后缀这些行为，新增注入一律加字段，不改默认路径；
- **沿用既有口径**——「逻辑请求」指纹（重放沿用首次结论）、场景选择优先级等
  文件头契约不推翻，异常注入接入同一套匹配规则；
- 每类注入**独立可红**：配一条用例先证明「开关翻掉会红」（接线判据纪律，
  见 [e2e/README](../../e2e/README.md) · 新增「接线判据」用例时另加一条纪律）。

## 处理过程

- 2026-10-13 用户定调黑盒优先：mock 与 e2e 建设不过度依赖项目内部实现细节，站在用户角度建判据——注入的是**真实服务 / 真实网络形态**，断言走**用户可见出口**（CLI 成败 / 转写读出口 / 可见错误信息）。内部阅读只在「用户会看见什么」需要定口径时做参考，不作为判据本身。

- 2026-10-13 登记（[GO-005](../goal/GO-005-全面e2e测试体系.md "GO-005-全面e2e测试体系") 的两个支撑任务之一）。同日转 doing 后收口：mock 注入机制（流中断 / 429 / 不规范帧 / 延迟与停顿）落地，四条黑盒用例背靠机制补齐，e2e 门禁 49/49。

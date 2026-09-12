# 工具失败回传与 Turn 终态机制

> 历史实施记录见 `docs/archive/implementation-logs/`。
> 本文与 `../README.md` 的六大压缩策略互补：那里讲"上下文怎么裁"，这里讲"失败怎么传播、终态怎么定"。

## 机制一：工具失败是信息性的，不中断循环

工具执行失败 ≠ 会话失败。失败信息作为工具结果回传给模型，由模型决定下一步。三条防线：

1. **失败结果定格 Completed**：工具结果节点以失败内容正常落库，Turn 不因工具失败进入 Failed。
2. **父 ToolCall 同标 Completed**：并携带 `meta.failure_kind = "error"`，前端可区分展示。
3. **Failed 仅用于 Turn 根节点**：只有会话级失败（编排崩溃、存储不可用等）才把整个 Turn 标为 Failed。

## 机制二：Turn 只有 Completed / Failed 两种终态

- **persist_failure 精确语义**：仅当"持久化本身失败"才走 Failed 分支，业务失败不污染终态。
- **RetryTurn 删除-重建**：重试即删除失败 Turn 后重建，不留半截中间态。
- **用户手动中止**：走 ABORTED 分支（非 Failed），保留已完成的部分结果。
- **压缩阶段失败就地降级**：压缩失败不冒泡、不改变 Turn 终态，本轮按未压缩上下文继续。

## "继续会话"的中断可见性（三层分工）

| 层 | 职责 |
|----|------|
| 存储层 | 全量保存，包括 Failed Turn 的原始记录 |
| 会话层 | 不过滤 Failed，按序返回完整历史 |
| 请求视图层（build_request_view） | 对中断点附加说明文案 + 占位工具结果，并把 `success` 推导为 false 提示模型 |

## 关联

- 请求视图剪裁与压缩分层：`../README.md` 策略⑥ + `docs/design/context-compression-design.md`（L0-L6 总览）

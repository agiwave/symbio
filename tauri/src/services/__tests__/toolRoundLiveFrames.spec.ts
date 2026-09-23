/**
 * 工具轮实时帧序列复现测试（node 环境）
 *
 * 复现并钉住用户报告的症状：「工具调用结果一直缺失，会话一直往下走时工具一直处于
 * 进行中；刷新后正确」——即**实时模式下丢信息**。
 *
 * 方法：按后端真实变更序（与 `turn.rs::emit_message` / `emit_delta` / `emit_state`、
 * `chat_loop/state.rs`、`tool_executor.rs` 的广播点逐条对齐；经 ADR-025 迁移后，
 * 这些广播点发的是 **VDFS 变更**，前端由 `stores/sessionTranscriptSync` 消费）驱动
 * 真实 store 的落地口，然后断言终态：
 *
 *   1. 结果子节点（role=tool）必须在 store 里；
 *   2. ToolCall 父节点终态必须是 completed；
 *   3. 树上 ToolCall 必须挂上结果子节点。
 *
 * 落地语义（**语义全在字段上**）：`delta` 追加 / `content` 替换 /
 * `status=removed` 移除 / 其余字段合并。变更序（一轮「思考 → 工具调用 → 结果」）：
 *
 * | # | 落地帧                                             | 后端发射点                  |
 * |---|---------------------------------------------------|-----------------------------|
 * | 1 | Turn root（streaming + meta.turn）                | `emit_streaming_start`      |
 * | 2 | Reasoning（身份 + 首段正文）                      | 首条 `apply`                |
 * | 3 | Reasoning `delta` ×N                              | `delta` → `updated`+`delta` |
 * | 4 | ToolCall（身份 + 首段参数）                       | `apply`                     |
 * | 5 | ToolCall `delta`（参数增量）                      | 同上（与 Text 同构）        |
 * | 6 | Reasoning `status=completed`                      | `finalize_assistant_turn`   |
 * | 7 | ToolCall（+meta.started_at）                      | `emit_tool_running`         |
 * | 8 | 结果子节点（completed，非流式一次给全）           | `process_tool_calls_async`  |
 * | 9 | ToolCall（completed）                             | `emit_parent_finalized`     |
 * |10 | Turn root `status=completed`                      | `finalize_turn_root`        |
 *
 * 两条结构约束（都由本文件的断言钉住，勿再退回）：
 * - **Turn root 的终态在最后**：它是组合节点，终态必须**跟随子树**（#9 之后），
 *   不得出现在 LLM 流结束那一刻（那样容器会先于其中的工具调用完成）；
 * - **工具参数是窄增量**：只有首条（身份字段出现）带正文，之后是 `delta` 追加。
 *
 * 注意状态迁移（#6/#7/#9/#10）**只带身份 + 状态，不带正文**——落地是**合并**，
 * 这正是要锚定的语义（整条替换会让这种部分帧把 `content` / `type` 一并抹掉）。
 *
 * 传输层（VDFS 变更的解包、读/流协调、resync）由
 * `stores/__tests__/sessionTranscriptSync.spec.ts` 单独钉住；本文件只关心
 * **落地之后** store 的结果。
 */

import { describe, expect, it, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

import { useSessionsStore } from '@/stores/sessions'
import type { ChatMessage } from '@/schemas/chat_message'

const SID = 'sess1'
const TURN = 'turn1'
const REASON = 'reason1'
const TC = 'tc1' // ToolCall 节点 id（accumulator 的 node_id，8 字符 short_id 形态）
const RESULT = 'r1' // 结果子节点 id（uuid）

/** 驱动一条落地帧：与 `sessionTranscriptSync` 消费端交给 store 的形状一致 */
function feed(message: ChatMessage): void {
  useSessionsStore().applyTranscriptMessages(SID, [message])
}

beforeEach(() => {
  setActivePinia(createPinia())
})

describe('工具轮实时帧序列 → store 终态', () => {
  it('结果子节点落 store、ToolCall 父节点终态 completed', () => {
    // ── 1. Turn root（emit_streaming_start：streaming + meta.turn）──
    feed({ id: TURN, role: 'assistant', type: 'turn', status: 'streaming', meta: { turn: 0 } })

    // ── 2. Reasoning 首帧（身份 + 首段正文）──
    feed({
      id: REASON,
      role: 'assistant',
      type: 'reasoning',
      parent_id: TURN,
      status: 'streaming',
      content: '第一段思考',
    })

    // ── 3. Reasoning 增量 ×2（窄载荷）──
    feed({ id: REASON, delta: '，继续' })
    feed({ id: REASON, delta: '思考。' })

    // ── 4. ToolCall 首帧（身份字段 + 首个参数片段）──
    feed({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'local/read',
      status: 'streaming',
      content: '{"path":"',
    })

    // ── 5. ToolCall 参数渐增（**窄增量**：与 Text / Reasoning 同构）──
    feed({ id: TC, delta: 'a.md"}' })

    // ── 6. Reasoning 终态（finalize：状态帧，不带正文）──
    feed({
      id: REASON,
      role: 'assistant',
      type: 'reasoning',
      parent_id: TURN,
      status: 'completed',
    })

    // ── 7. ToolCall 进入执行（emit_tool_running：状态帧 + meta）──
    feed({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'local/read',
      status: 'streaming',
      meta: { started_at: 1726900000000 },
    })

    // ── 8. 结果子节点（process_tool_calls_async；非流式工具一次给全）──
    feed({
      id: RESULT,
      role: 'tool',
      type: 'text',
      parent_id: TC,
      status: 'completed',
      content: '文件内容……',
    })

    // ── 9. ToolCall 终态（emit_parent_finalized：状态帧 + 结果 meta）──
    feed({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'local/read',
      status: 'completed',
      meta: { started_at: 1726900000000, success: true },
    })

    // ── 10. Turn root 终态（finalize_turn_root：**子树收敛之后**才发出）──
    // 此前它在 `finalize_assistant_turn`（LLM 流结束那一刻）发出，比子树早整整
    // 一个执行窗口——容器"已完成"而其中的工具调用仍在运行。现在移到 `close_turn`
    // 归来之后；发出的即 `build_assistant_messages` 构建的那条（pending 落库的
    // 同一份），因此不带流式占位期的 `meta.turn`。
    feed({ id: TURN, role: 'assistant', type: 'turn', status: 'completed' })

    const store = useSessionsStore()
    const msgs = store.getSessionMessages(SID)
    const tc = msgs.find((m) => m.id === TC)
    const result = msgs.find((m) => m.id === RESULT)
    const reason = msgs.find((m) => m.id === REASON)

    expect(tc, 'ToolCall 父节点必须在 store').toBeTruthy()
    expect(tc!.status, 'ToolCall 父节点必须收到终态').toBe('completed')
    expect(tc!.meta?.success, '执行结果标记必须落到节点上').toBe(true)
    expect(tc!.content, 'ToolCall 参数必须由 delta 拼出完整 JSON').toBe('{"path":"a.md"}')
    expect(result, '结果子节点必须落 store（症状：一直缺失）').toBeTruthy()
    expect(result!.parent_id).toBe(TC)
    expect(result!.role).toBe('tool')
    expect(result!.status).toBe('completed')
    expect(reason!.content, '状态帧不得吞掉已累积的正文').toBe('第一段思考，继续思考。')
  })

  it('流式工具响应：增量追加到 role=tool 节点（不得被覆盖成最后一片）', () => {
    // ToolCall 已成形（调用子智能体的工具）
    feed({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'agent/run',
      status: 'streaming',
      content: '{}',
    })

    // 子会话顶层节点：`tool_executor` 把 parent 锚定到 ToolCall、role 归 Tool 后转发
    feed({ id: RESULT, role: 'tool', type: 'turn', parent_id: TC, status: 'streaming' })

    // 响应逐片到达（后端透传子会话的 `delta`，不是全量重发）
    feed({ id: RESULT, delta: '分析中' })
    feed({ id: RESULT, delta: '……结论：' })
    feed({ id: RESULT, delta: '共 10 个文件。' })

    // 终态（状态帧，不带正文）
    feed({ id: RESULT, status: 'completed' })

    const store = useSessionsStore()
    const result = store.getSessionMessages(SID).find((m) => m.id === RESULT)
    expect(result, '流式响应节点必须落 store').toBeTruthy()
    // 回归：`mergeMessagePatch` 曾按 `role=tool` 判定"流式帧全量重发"，
    // 逐片响应因此只剩最后一片（工具卡片有请求、响应是空的）。
    expect(result!.content, '增量必须逐片拼出完整响应').toBe('分析中……结论：共 10 个文件。')
    expect(result!.status).toBe('completed')
  })
})

describe('合帧落到 store：一批只推一次版本号', () => {
  it('一批 3 帧 → 版本号只 +1，且正文逐条拼出', () => {
    const store = useSessionsStore()
    const v0 = store.transcriptVersion

    store.applyTranscriptMessages(SID, [
      { id: 'm1', role: 'assistant', type: 'text', content: 'a' },
      { id: 'm1', delta: 'b' },
      { id: 'm1', delta: 'c' },
    ])

    expect(store.transcriptVersion, '一批只提交一次（否则视图要重建 N 次树）').toBe(v0 + 1)
    expect(store.getSessionMessages(SID).find((m) => m.id === 'm1')?.content).toBe('abc')
  })

  it('单帧入口与批量入口语义一致（单帧只是批长为 1）', () => {
    const store = useSessionsStore()
    const v0 = store.transcriptVersion

    store.applyTranscriptMessage(SID, { id: 'x1', role: 'assistant', type: 'text', content: 'hi' })
    expect(store.transcriptVersion).toBe(v0 + 1)
    expect(store.getSessionMessages(SID).find((m) => m.id === 'x1')?.content).toBe('hi')
  })

  it('一批里全是「删除不存在的节点」时不提交（幂等收口不白拷贝）', () => {
    const store = useSessionsStore()
    const v0 = store.transcriptVersion

    store.applyTranscriptMessages(SID, [
      { id: 'nope1', status: 'removed' },
      { id: 'nope2', status: 'removed' },
    ])

    expect(store.transcriptVersion, '无操作的一批不得推进版本号').toBe(v0)
  })
})

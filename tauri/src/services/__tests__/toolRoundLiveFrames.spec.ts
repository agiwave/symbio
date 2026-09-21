/**
 * 工具轮实时帧序列复现测试（node 环境）
 *
 * 复现并钉住用户报告的症状：「工具调用结果一直缺失，会话一直往下走时工具一直处于
 * 进行中；刷新后正确」——即**实时模式下丢信息**。
 *
 * 方法：按后端真实帧序（与 `turn.rs::emit_update` / `emit_append`、
 * `chat_loop/state.rs`、`tool_executor.rs` 的广播点逐帧对齐）驱动真实 store
 * （经 `transcriptStream`），然后断言终态：
 *
 *   1. 结果子节点（role=tool）必须在 store 里；
 *   2. ToolCall 父节点终态必须是 completed；
 *   3. 树上 ToolCall 必须挂上结果子节点。
 *
 * 帧序列（一轮「思考 → 工具调用 → 结果」）——**一条流、单调 seq**：
 *
 * | # | 帧                                    | 后端发射点                  |
 * |---|---------------------------------------|-----------------------------|
 * | 1 | upsert Turn root (streaming)          | `emit_streaming_start`      |
 * | 2 | upsert Reasoning (streaming)          | 首帧全量 `emit_update`      |
 * | 3 | append Reasoning ×N                   | `emit_append`（窄载荷）     |
 * | 4 | upsert ToolCall (streaming, 身份+参数) | ToolCall 首帧（快照）       |
 * | 5 | append ToolCall (参数增量)            | `emit_append`（与 Text 同构）|
 * | 6 | upsert Reasoning (completed)          | `finalize_assistant_turn`   |
 * | 7 | upsert ToolCall (streaming+meta)      | `emit_tool_running`         |
 * | 8 | upsert 结果子节点 (completed)         | `process_tool_calls_async`  |
 * | 9 | upsert ToolCall (completed)           | `emit_parent_finalized`     |
 * |10 | upsert Turn root (completed)          | `finalize_turn_root`        |
 *
 * 两条结构约束（都由本文件的断言钉住，勿再退回）：
 * - **Turn root 的终态在最后**：它是组合节点，终态必须**跟随子树**（#9 之后），
 *   不得出现在 LLM 流结束那一刻（那样容器会先于其中的工具调用完成）；
 * - **工具参数是窄增量**：只有首帧（身份字段出现）是快照，之后是 `append`。
 */

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const mocks = vi.hoisted(() => ({
  connectPlugin: vi.fn(),
}))

vi.mock('@/services/plugin', () => ({
  connectPlugin: mocks.connectPlugin,
  callPlugin: vi.fn(),
  setLastWorkdir: vi.fn(),
  getLastWorkdir: vi.fn(),
}))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import { useSessionsStore } from '@/stores/sessions'
import {
  applyNodeEvent,
  handleStreamFrame,
  startTranscriptStream,
  stopTranscriptStream,
  type NodeEvent,
  type TranscriptStreamSink,
} from '../transcriptStream'
import { SESSION_STREAM } from '@/constants/pluginPaths'
import type { ChatMessage } from '@/schemas/chat_message'

const SID = 'sess1'
const TURN = 'turn1'
const REASON = 'reason1'
const TC = 'tc1' // ToolCall 节点 id（accumulator 的 node_id，8 字符 short_id 形态）
const RESULT = 'r1' // 结果子节点 id（uuid）

/** 造一帧事件；`seq` 由帧序自动续上（与后端单调计数器同形） */
let seq = 0
function ev(op: NodeEvent['op'], body: Omit<NodeEvent, 'session_id' | 'seq' | 'op'>): NodeEvent {
  seq += 1
  return { session_id: SID, seq, op, ...body }
}

/** 与 `MainLayout` 的接线**逐字一致**：协议操作 → store 动作 */
function storeSink(): TranscriptStreamSink {
  const store = useSessionsStore()
  return {
    upsert: (sid, m) => store.putMessage(sid, m),
    append: (sid, id, delta) => store.appendMessage(sid, id, delta),
    remove: (sid, id) => store.removeMessageById(sid, id),
    reload: async (sid) => {
      await store.loadMessages(sid)
    },
  }
}

/** 记录落地动作的假 sink（协议层测试不牵动 store） */
function spySink() {
  const calls: string[] = []
  const reloaded: string[] = []
  const sink: TranscriptStreamSink = {
    upsert: (sid, m: ChatMessage) => calls.push(`upsert:${sid}:${m.id}`),
    append: (sid, id, d) => calls.push(`append:${sid}:${id}:${d}`),
    remove: (sid, id) => calls.push(`remove:${sid}:${id}`),
    reload: (sid) => {
      reloaded.push(sid)
    },
  }
  return { sink, calls, reloaded }
}

beforeEach(() => {
  vi.resetAllMocks()
  setActivePinia(createPinia())
  seq = 0
  stopTranscriptStream()
  mocks.connectPlugin.mockResolvedValue({ connectionId: 'c1', close: vi.fn(async () => {}) })
})

describe('工具轮实时帧序列 → store 终态', () => {
  it('结果子节点落 store、ToolCall 父节点终态 completed', async () => {
    await startTranscriptStream(storeSink())

    // 帧必须经 `startTranscriptStream` 建立的那条连接到达（信封覆盖在内）
    expect(mocks.connectPlugin.mock.calls[0]?.[0], '必须订阅会话转写流').toBe(SESSION_STREAM)
    const onFrame = mocks.connectPlugin.mock.calls[0]?.[2] as (e: unknown) => void
    expect(onFrame, 'startTranscriptStream 必须注册帧回调').toBeTruthy()
    const emit = (e: NodeEvent) => onFrame({ type: 'transcript_event', data: e })

    // ── 1. Turn root（emit_streaming_start：streaming + meta.turn）──
    emit(ev('upsert', {
      message: { id: TURN, role: 'assistant', type: 'turn', status: 'streaming', meta: { turn: 0 } },
    }))

    // ── 2. Reasoning 首帧全量 ──
    emit(ev('upsert', {
      message: {
        id: REASON,
        role: 'assistant',
        type: 'reasoning',
        parent_id: TURN,
        status: 'streaming',
        content: '第一段思考',
      },
    }))

    // ── 3. Reasoning 增量 ×2（窄载荷）──
    emit(ev('append', { message_id: REASON, delta: '，继续' }))
    emit(ev('append', { message_id: REASON, delta: '思考。' }))

    // ── 4. ToolCall 首帧（身份字段 + 首个参数片段，**完整快照**）──
    emit(ev('upsert', {
      message: {
        id: TC,
        role: 'assistant',
        type: 'tool_call',
        parent_id: TURN,
        name: 'local/read',
        status: 'streaming',
        content: '{"path":"',
      },
    }))

    // ── 5. ToolCall 参数渐增（**窄增量**：与 Text / Reasoning 同构）──
    emit(ev('append', { message_id: TC, delta: 'a.md"}' }))

    // ── 6. Reasoning 终态（finalize：全量快照）──
    emit(ev('upsert', {
      message: {
        id: REASON,
        role: 'assistant',
        type: 'reasoning',
        parent_id: TURN,
        status: 'completed',
        content: '第一段思考，继续思考。',
      },
    }))

    // ── 7. ToolCall 进入执行（emit_tool_running）──
    emit(ev('upsert', {
      message: {
        id: TC,
        role: 'assistant',
        type: 'tool_call',
        parent_id: TURN,
        name: 'local/read',
        status: 'streaming',
        content: '{"path":"a.md"}',
        meta: { started_at: 1726900000000 },
      },
    }))

    // ── 8. 结果子节点（process_tool_calls_async；非流式工具一次给全）──
    emit(ev('upsert', {
      message: {
        id: RESULT,
        role: 'tool',
        type: 'text',
        parent_id: TC,
        status: 'completed',
        content: '文件内容……',
      },
    }))

    // ── 9. ToolCall 终态（emit_parent_finalized）──
    emit(ev('upsert', {
      message: {
        id: TC,
        role: 'assistant',
        type: 'tool_call',
        parent_id: TURN,
        name: 'local/read',
        status: 'completed',
        content: '{"path":"a.md"}',
        meta: { started_at: 1726900000000, success: true },
      },
    }))

    // ── 10. Turn root 终态（finalize_turn_root：**子树收敛之后**才发出）──
    // 此前它在 `finalize_assistant_turn`（LLM 流结束那一刻）发出，比子树早整整
    // 一个执行窗口——容器"已完成"而其中的工具调用仍在运行（抓包 seq 5 < seq 8）。
    // 现在移到 `close_turn` 归来之后；发出的快照即 `build_assistant_messages`
    // 构建的那条（pending 落库的同一份），因此不带流式占位期的 `meta.turn`。
    emit(ev('upsert', {
      message: { id: TURN, role: 'assistant', type: 'turn', status: 'completed' },
    }))

    const store = useSessionsStore()
    const msgs = store.getSessionMessages(SID)
    const tc = msgs.find((m) => m.id === TC)
    const result = msgs.find((m) => m.id === RESULT)
    const reason = msgs.find((m) => m.id === REASON)

    expect(tc, 'ToolCall 父节点必须在 store').toBeTruthy()
    expect(tc!.status, 'ToolCall 父节点必须收到终态').toBe('completed')
    expect(tc!.meta?.success, '执行结果标记必须落到节点上').toBe(true)
    expect(tc!.content, 'ToolCall 参数必须由 append 拼出完整 JSON').toBe('{"path":"a.md"}')
    expect(result, '结果子节点必须落 store（症状：一直缺失）').toBeTruthy()
    expect(result!.parent_id).toBe(TC)
    expect(result!.role).toBe('tool')
    expect(result!.status).toBe('completed')
    expect(reason!.content, '增量帧必须逐步拼出完整正文').toBe('第一段思考，继续思考。')
  })

  it('流式工具响应：增量追加到 role=tool 节点（不得被覆盖成最后一片）', async () => {
    await startTranscriptStream(storeSink())
    const onFrame = mocks.connectPlugin.mock.calls[0]?.[2] as (e: unknown) => void
    const emit = (e: NodeEvent) => onFrame({ type: 'transcript_event', data: e })

    // ToolCall 已成形（调用子智能体的工具）
    emit(ev('upsert', {
      message: {
        id: TC,
        role: 'assistant',
        type: 'tool_call',
        parent_id: TURN,
        name: 'agent/run',
        status: 'streaming',
        content: '{}',
      },
    }))

    // 子会话顶层节点：`tool_executor` 把 parent 锚定到 ToolCall、role 归 Tool 后转发
    emit(ev('upsert', {
      message: { id: RESULT, role: 'tool', type: 'turn', parent_id: TC, status: 'streaming' },
    }))

    // 响应逐片到达（后端**透传子会话的 `Append`**，不是全量重发）
    emit(ev('append', { message_id: RESULT, delta: '分析中' }))
    emit(ev('append', { message_id: RESULT, delta: '……结论：' }))
    emit(ev('append', { message_id: RESULT, delta: '共 10 个文件。' }))

    // 终态快照（全量）
    emit(ev('upsert', {
      message: {
        id: RESULT,
        role: 'tool',
        type: 'turn',
        parent_id: TC,
        status: 'completed',
        content: '分析中……结论：共 10 个文件。',
      },
    }))

    const store = useSessionsStore()
    const result = store.getSessionMessages(SID).find((m) => m.id === RESULT)
    expect(result, '流式响应节点必须落 store').toBeTruthy()
    // 回归：`mergeMessagePatch` 曾按 `role=tool` 判定"流式帧全量重发"，
    // 逐片响应因此只剩最后一片（工具卡片有请求、响应是空的）。
    expect(result!.content, '增量必须逐片拼出完整响应').toBe('分析中……结论：共 10 个文件。')
    expect(result!.status).toBe('completed')
  })
})

describe('顺序判定与恢复（丢帧可检测）', () => {
  /** 只接 sink、不建连接：`startTranscriptStream` 在首个 await 前已完成接线 */
  function wire(sink: TranscriptStreamSink): void {
    void startTranscriptStream(sink)
  }

  it('未接线时帧被丢弃（无落地目标，不抛错）', () => {
    expect(() =>
      handleStreamFrame({
        type: 'transcript_event',
        data: { session_id: SID, seq: 1, op: 'upsert', message: { id: 'm1' } },
      }),
    ).not.toThrow()
  })

  it('跳号 = 已知有损：本帧不落地，改为整份重读', () => {
    const spy = spySink()
    wire(spy.sink)
    applyNodeEvent({ session_id: SID, seq: 1, op: 'upsert', message: { id: 'm1' } })
    // 2 丢了：3 到达时必须整份重读，而不是把 3 补上去（缺的一帧不可凭空补）
    applyNodeEvent({ session_id: SID, seq: 3, op: 'upsert', message: { id: 'm3' } })
    expect(spy.calls).toEqual(['upsert:sess1:m1'])
    expect(spy.reloaded, '跳号必须触发整份重读').toEqual([SID])
  })

  it('重复帧（seq 不大于已应用序号）丢弃——增量重复应用会叠字', () => {
    const spy = spySink()
    wire(spy.sink)
    applyNodeEvent({ session_id: SID, seq: 1, op: 'upsert', message: { id: 'm1' } })
    applyNodeEvent({ session_id: SID, seq: 1, op: 'upsert', message: { id: 'm1' } })
    applyNodeEvent({ session_id: SID, seq: 1, op: 'append', message_id: 'm1', delta: 'x' })
    expect(spy.calls).toEqual(['upsert:sess1:m1'])
    expect(spy.reloaded).toEqual([])
  })

  it('reset：本地整份作废并从存储重读', () => {
    const spy = spySink()
    wire(spy.sink)
    applyNodeEvent({ session_id: SID, seq: 1, op: 'upsert', message: { id: 'm1' } })
    applyNodeEvent({ session_id: SID, seq: 2, op: 'reset' })
    expect(spy.calls).toEqual(['upsert:sess1:m1'])
    expect(spy.reloaded).toEqual([SID])
  })

  it('remove 删单条；warn 不落消息层（会话级状态归会话节点）', () => {
    const spy = spySink()
    wire(spy.sink)
    applyNodeEvent({ session_id: SID, seq: 1, op: 'remove', message_id: 'm9' })
    applyNodeEvent({ session_id: SID, seq: 2, op: 'warn', warning: '上一轮被截断' })
    expect(spy.calls).toEqual(['remove:sess1:m9'])
    expect(spy.reloaded).toEqual([])
  })

  it('resync 标记：整份重读全部已知会话', () => {
    const spy = spySink()
    wire(spy.sink)
    applyNodeEvent({ session_id: 'a', seq: 1, op: 'upsert', message: { id: 'm' } })
    applyNodeEvent({ session_id: 'b', seq: 1, op: 'upsert', message: { id: 'm' } })
    handleStreamFrame({ type: 'transcript_resync', data: null })
    expect([...spy.reloaded].sort()).toEqual(['a', 'b'])
  })

  it('多会话各自独立的序号游标（互不干扰）', () => {
    const spy = spySink()
    wire(spy.sink)
    applyNodeEvent({ session_id: 'a', seq: 1, op: 'upsert', message: { id: 'm1' } })
    applyNodeEvent({ session_id: 'b', seq: 7, op: 'upsert', message: { id: 'm2' } })
    applyNodeEvent({ session_id: 'a', seq: 2, op: 'upsert', message: { id: 'm3' } })
    expect(spy.calls).toEqual(['upsert:a:m1', 'upsert:b:m2', 'upsert:a:m3'])
    expect(spy.reloaded).toEqual([])
  })
})

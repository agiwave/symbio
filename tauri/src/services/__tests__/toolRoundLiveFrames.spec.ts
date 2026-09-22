/**
 * 工具轮实时帧序列复现测试（node 环境）
 *
 * 复现并钉住用户报告的症状：「工具调用结果一直缺失，会话一直往下走时工具一直处于
 * 进行中；刷新后正确」——即**实时模式下丢信息**。
 *
 * 方法：按后端真实帧序（与 `turn.rs::emit_message` / `emit_delta` / `emit_state`、
 * `chat_loop/state.rs`、`tool_executor.rs` 的广播点逐帧对齐）驱动真实 store
 * （经 `transcriptStream`），然后断言终态：
 *
 *   1. 结果子节点（role=tool）必须在 store 里；
 *   2. ToolCall 父节点终态必须是 completed；
 *   3. 树上 ToolCall 必须挂上结果子节点。
 *
 * 帧协议（**帧 = 一条 `ChatMessage`**，语义全在字段上）：`delta` 追加 /
 * `content` 替换 / `status=removed` 移除 / 其余字段合并。帧序（一轮
 * 「思考 → 工具调用 → 结果」）——**一条流、单调 seq**：
 *
 * | # | 帧                                                | 后端发射点                  |
 * |---|---------------------------------------------------|-----------------------------|
 * | 1 | Turn root（streaming + meta.turn）                | `emit_streaming_start`      |
 * | 2 | Reasoning（身份 + 首段正文）                      | 首帧 `emit_message`         |
 * | 3 | Reasoning `delta` ×N                              | `emit_delta`（窄载荷）      |
 * | 4 | ToolCall（身份 + 首段参数）                       | `emit_message`              |
 * | 5 | ToolCall `delta`（参数增量）                      | `emit_delta`（与 Text 同构）|
 * | 6 | Reasoning `status=completed`（`state_frame`）     | `finalize_assistant_turn`   |
 * | 7 | ToolCall `state_frame`（+meta.started_at）        | `emit_tool_running`         |
 * | 8 | 结果子节点（completed，非流式一次给全）           | `process_tool_calls_async`  |
 * | 9 | ToolCall `state_frame`（completed）               | `emit_parent_finalized`     |
 * |10 | Turn root `status=completed`                      | `finalize_turn_root`        |
 *
 * 两条结构约束（都由本文件的断言钉住，勿再退回）：
 * - **Turn root 的终态在最后**：它是组合节点，终态必须**跟随子树**（#9 之后），
 *   不得出现在 LLM 流结束那一刻（那样容器会先于其中的工具调用完成）；
 * - **工具参数是窄增量**：只有首帧（身份字段出现）带正文，之后是 `delta` 追加。
 *
 * 注意状态帧（#6/#7/#9/#10）**只带身份 + 状态，不带正文**（与后端 `state_frame`
 * 同构）——落地是**合并**，这正是要锚定的新语义（旧协议 `upsert` 整条替换会让
 * 这种部分帧把 `content` / `type` 一并抹掉）。
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
  flushPendingFrames,
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
function ev(message: ChatMessage): NodeEvent {
  seq += 1
  return { session_id: SID, seq, message }
}

/** 与 `MainLayout` 的接线**逐字一致**：一批帧 → store 动作 */
function storeSink(): TranscriptStreamSink {
  const store = useSessionsStore()
  return {
    messages: (sid, ms) => store.applyTranscriptMessages(sid, ms),
    reload: async (sid) => {
      await store.loadMessages(sid)
    },
  }
}

/** 记录落地动作的假 sink（协议层测试不牵动 store） */
function spySink() {
  const calls: string[] = []
  const reloaded: string[] = []
  /** 每一次 `messages` 调用 = 一次提交（合帧的收益就落在这个长度上） */
  const batches: Array<{ sid: string; ids: string[] }> = []
  const sink: TranscriptStreamSink = {
    messages: (sid, ms) => {
      batches.push({ sid, ids: ms.map((m) => m.id) })
      for (const m of ms) calls.push(`message:${sid}:${m.id}`)
    },
    reload: (sid) => {
      reloaded.push(sid)
    },
  }
  return { sink, calls, reloaded, batches }
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
    const emit = (e: NodeEvent) => {
      onFrame({ type: 'transcript_event', data: e })
      // 合帧是**提交批量化**（性能），不是语义变化：测试按帧断言，故每帧后立即冲刷。
      flushPendingFrames()
    }

    // ── 1. Turn root（emit_streaming_start：streaming + meta.turn）──
    emit(ev({ id: TURN, role: 'assistant', type: 'turn', status: 'streaming', meta: { turn: 0 } }))

    // ── 2. Reasoning 首帧（身份 + 首段正文）──
    emit(ev({
      id: REASON,
      role: 'assistant',
      type: 'reasoning',
      parent_id: TURN,
      status: 'streaming',
      content: '第一段思考',
    }))

    // ── 3. Reasoning 增量 ×2（窄载荷）──
    emit(ev({ id: REASON, delta: '，继续' }))
    emit(ev({ id: REASON, delta: '思考。' }))

    // ── 4. ToolCall 首帧（身份字段 + 首个参数片段）──
    emit(ev({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'local/read',
      status: 'streaming',
      content: '{"path":"',
    }))

    // ── 5. ToolCall 参数渐增（**窄增量**：与 Text / Reasoning 同构）──
    emit(ev({ id: TC, delta: 'a.md"}' }))

    // ── 6. Reasoning 终态（finalize：状态帧，不带正文）──
    emit(ev({
      id: REASON,
      role: 'assistant',
      type: 'reasoning',
      parent_id: TURN,
      status: 'completed',
    }))

    // ── 7. ToolCall 进入执行（emit_tool_running：状态帧 + meta）──
    emit(ev({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'local/read',
      status: 'streaming',
      meta: { started_at: 1726900000000 },
    }))

    // ── 8. 结果子节点（process_tool_calls_async；非流式工具一次给全）──
    emit(ev({
      id: RESULT,
      role: 'tool',
      type: 'text',
      parent_id: TC,
      status: 'completed',
      content: '文件内容……',
    }))

    // ── 9. ToolCall 终态（emit_parent_finalized：状态帧 + 结果 meta）──
    emit(ev({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'local/read',
      status: 'completed',
      meta: { started_at: 1726900000000, success: true },
    }))

    // ── 10. Turn root 终态（finalize_turn_root：**子树收敛之后**才发出）──
    // 此前它在 `finalize_assistant_turn`（LLM 流结束那一刻）发出，比子树早整整
    // 一个执行窗口——容器"已完成"而其中的工具调用仍在运行（抓包 seq 5 < seq 8）。
    // 现在移到 `close_turn` 归来之后；发出的即 `build_assistant_messages`
    // 构建的那条（pending 落库的同一份），因此不带流式占位期的 `meta.turn`。
    emit(ev({ id: TURN, role: 'assistant', type: 'turn', status: 'completed' }))

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

  it('流式工具响应：增量追加到 role=tool 节点（不得被覆盖成最后一片）', async () => {
    await startTranscriptStream(storeSink())
    const onFrame = mocks.connectPlugin.mock.calls[0]?.[2] as (e: unknown) => void
    const emit = (e: NodeEvent) => {
      onFrame({ type: 'transcript_event', data: e })
      // 合帧是**提交批量化**（性能），不是语义变化：测试按帧断言，故每帧后立即冲刷。
      flushPendingFrames()
    }

    // ToolCall 已成形（调用子智能体的工具）
    emit(ev({
      id: TC,
      role: 'assistant',
      type: 'tool_call',
      parent_id: TURN,
      name: 'agent/run',
      status: 'streaming',
      content: '{}',
    }))

    // 子会话顶层节点：`tool_executor` 把 parent 锚定到 ToolCall、role 归 Tool 后转发
    emit(ev({ id: RESULT, role: 'tool', type: 'turn', parent_id: TC, status: 'streaming' }))

    // 响应逐片到达（后端**透传子会话的 `delta`**，不是全量重发）
    emit(ev({ id: RESULT, delta: '分析中' }))
    emit(ev({ id: RESULT, delta: '……结论：' }))
    emit(ev({ id: RESULT, delta: '共 10 个文件。' }))

    // 终态（状态帧，不带正文）
    emit(ev({ id: RESULT, status: 'completed' }))

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

  /**
   * 驱动一帧并**立即落地**。
   *
   * `transcriptStream` 在生产里把 ~48ms 窗口内的帧攒批提交（性能优化，不改帧语义）；
   * 本块按帧断言，故每帧后显式冲刷，等价于「窗口 = 0」。
   */
  function feed(e: NodeEvent): void {
    applyNodeEvent(e)
    flushPendingFrames()
  }

  it('未接线时帧被丢弃（无落地目标，不抛错）', () => {
    expect(() =>
      handleStreamFrame({
        type: 'transcript_event',
        data: { session_id: SID, seq: 1, message: { id: 'm1' } },
      }),
    ).not.toThrow()
  })

  it('跳号 = 已知有损：本帧不落地，改为整份重读', () => {
    const spy = spySink()
    wire(spy.sink)
    feed({ session_id: SID, seq: 1, message: { id: 'm1' } })
    // 2 丢了：3 到达时必须整份重读，而不是把 3 补上去（缺的一帧不可凭空补）
    feed({ session_id: SID, seq: 3, message: { id: 'm3' } })
    expect(spy.calls).toEqual(['message:sess1:m1'])
    expect(spy.reloaded, '跳号必须触发整份重读').toEqual([SID])
  })

  it('重复帧（seq 不大于已应用序号）丢弃——增量重复应用会叠字', () => {
    const spy = spySink()
    wire(spy.sink)
    feed({ session_id: SID, seq: 1, message: { id: 'm1' } })
    feed({ session_id: SID, seq: 1, message: { id: 'm1' } })
    feed({ session_id: SID, seq: 1, message: { id: 'm1', delta: 'x' } })
    expect(spy.calls).toEqual(['message:sess1:m1'])
    expect(spy.reloaded).toEqual([])
  })

  it('删除 = 状态迁移为 removed（帧照常落地，由 store 就地移除）', () => {
    const spy = spySink()
    wire(spy.sink)
    feed({ session_id: SID, seq: 1, message: { id: 'm1' } })
    // 协议里没有 remove 操作：删除就是一帧 `status=removed` 的消息
    feed({ session_id: SID, seq: 2, message: { id: 'm9', status: 'removed' } })
    expect(spy.calls).toEqual(['message:sess1:m1', 'message:sess1:m9'])
    expect(spy.reloaded).toEqual([])
  })

  it('resync 标记：整份重读全部已知会话', () => {
    const spy = spySink()
    wire(spy.sink)
    feed({ session_id: 'a', seq: 1, message: { id: 'm' } })
    feed({ session_id: 'b', seq: 1, message: { id: 'm' } })
    handleStreamFrame({ type: 'transcript_resync', data: null })
    expect([...spy.reloaded].sort()).toEqual(['a', 'b'])
  })

  it('多会话各自独立的序号游标（互不干扰）', () => {
    const spy = spySink()
    wire(spy.sink)
    feed({ session_id: 'a', seq: 1, message: { id: 'm1' } })
    feed({ session_id: 'b', seq: 7, message: { id: 'm2' } })
    feed({ session_id: 'a', seq: 2, message: { id: 'm3' } })
    expect(spy.calls).toEqual(['message:a:m1', 'message:b:m2', 'message:a:m3'])
    expect(spy.reloaded).toEqual([])
  })
})

/**
 * 合帧：把窗口内的帧攒成**一次** sink 提交。
 *
 * 这是纯性能机制（把「每帧一次 O(历史条数) 提交」收成「每窗口一次」），
 * 因此测试要钉住的恰恰是**它没有改变帧语义**：帧仍逐条按序落地、`seq` 仍逐帧判定、
 * 重读前仍会丢弃待落地帧。
 */
describe('合帧（提交批量化，不改变帧语义）', () => {
  it('未到窗口不提交；冲刷后同会话的帧合成一次提交且顺序不变', () => {
    const spy = spySink()
    void startTranscriptStream(spy.sink)

    applyNodeEvent({ session_id: SID, seq: 1, message: { id: 'm1' } })
    applyNodeEvent({ session_id: SID, seq: 2, message: { id: 'm1', delta: 'a' } })
    applyNodeEvent({ session_id: SID, seq: 3, message: { id: 'm1', delta: 'b' } })
    expect(spy.batches.length, '未到合帧窗口不得提交').toBe(0)

    flushPendingFrames()
    expect(spy.batches, '三帧 → 一次提交，且保持到达顺序').toEqual([
      { sid: SID, ids: ['m1', 'm1', 'm1'] },
    ])
  })

  it('按会话分批：不同会话各自成批（互不合并）', () => {
    const spy = spySink()
    void startTranscriptStream(spy.sink)

    applyNodeEvent({ session_id: 'a', seq: 1, message: { id: 'm1' } })
    applyNodeEvent({ session_id: 'b', seq: 1, message: { id: 'm2' } })
    applyNodeEvent({ session_id: 'a', seq: 2, message: { id: 'm3' } })
    flushPendingFrames()

    expect(spy.batches).toEqual([
      { sid: 'a', ids: ['m1', 'm3'] },
      { sid: 'b', ids: ['m2'] },
    ])
  })

  it('跳号时**丢弃**待落地帧：权威快照已包含它们，再应用就是重复追加', () => {
    const spy = spySink()
    void startTranscriptStream(spy.sink)

    applyNodeEvent({ session_id: SID, seq: 1, message: { id: 'm1' } })
    applyNodeEvent({ session_id: SID, seq: 2, message: { id: 'm1', delta: 'x' } })
    // 3 丢了：到达的是 5 ⇒ 整份重读，且上面那两条待落地帧必须一并丢弃
    applyNodeEvent({ session_id: SID, seq: 5, message: { id: 'm5' } })

    expect(spy.batches, '跳号前的待落地帧不得再落地').toEqual([])
    expect(spy.reloaded, '跳号必须触发整份重读').toEqual([SID])
  })

  it('resync 标记同样丢弃待落地帧', () => {
    const spy = spySink()
    void startTranscriptStream(spy.sink)

    applyNodeEvent({ session_id: SID, seq: 1, message: { id: 'm1' } })
    handleStreamFrame({ type: 'transcript_resync', data: null })

    expect(spy.batches).toEqual([])
    expect(spy.reloaded).toEqual([SID])
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

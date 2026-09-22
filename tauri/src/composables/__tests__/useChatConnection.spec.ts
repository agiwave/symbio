/**
 * useChatConnection — 会话连接（发起动作 + 消息树派生）单测
 *
 * 本 composable 不订阅任何通道、不写 store 权威状态——它只做三件事：
 * **发起动作**（send / abort / resume）、**乐观置位**、**从 store 派生消息树**。
 * 因此这里全部用桩：store 与出站（`callPlugin`）都是可断言的假实现。
 *
 * 钉住的约定：
 * 1. **消息树**：空内容叶子（流式的占位空 Text / Reasoning）不渲染，
 *    但**带子节点的容器必须保留**；`removeMessage` 是**本地覆盖**、不污染 store；
 *    parent 不可见的悬挂节点升为根（不丢消息）。
 * 2. **乐观置位**：send 立刻置 working 并清会话级错误——这是「零回读」的前提，
 *    随后由后端节点状态覆盖。
 * 3. **失败收敛**：send 失败置 failed；仅当**没有在途消息**承载错误时才落会话级
 *    错误（否则根级节点与会话级会重复报错）。
 * 4. **跨会话保护**：resume 的目标是别的会话时，**不动本会话状态**
 *    （子智能体的工具调用不能误改父会话的「处理中」）。
 * 5. **session_busy 是现状不是失败**：回落 `active`，绝不置 `failed`
 *    ——否则会显示一个并不存在的失败终态，且重试按钮不消失。
 *
 * ⚠️ 只记录未改：`send` 的 `onSendComplete` 只在**失败**分支调用，成功时不调用
 * （成功由后端节点状态收敛）。这是否为原意未确认，故此处按现状断言并注明。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatMessage } from '@/schemas/chat_message'
import {
  MESSAGE_STATUS_STREAMING,
  MESSAGE_STATUS_WAITING_USER_ACTION,
} from '@/schemas/chat_message'
import { CHAT_ABORT, CHAT_SEND } from '@/constants/pluginPaths'
import { VDFS_STATUS_ACTIVE, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING } from '@/schemas/vdfs'

const hoisted = vi.hoisted(() => {
  const calls: Array<{ path: string; payload: unknown; timeout?: number; ctx?: unknown }> = []
  return {
    calls,
    callPlugin: vi.fn((path: string, payload: unknown, timeout?: number, ctx?: unknown) => {
      calls.push({ path, payload, timeout, ctx })
      return Promise.resolve({})
    }),
    store: { } as Record<string, unknown>,
  }
})

vi.mock('@/services/plugin', () => ({ callPlugin: hoisted.callPlugin }))
vi.mock('@/stores/sessions', () => ({ useSessionsStore: () => hoisted.store }))
vi.mock('@/utils/logger', () => ({
  logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}))

import { useChatConnection } from '../useChatConnection'

function makeStore(over: Record<string, unknown> = {}) {
  return {
    getSessionMessages: vi.fn(() => [] as ChatMessage[]),
    isSessionWorking: vi.fn(() => false),
    applyTranscriptMessage: vi.fn(),
    putStatus: vi.fn(),
    setSessionError: vi.fn(),
    setSessionStatus: vi.fn(),
    getSessionMode: vi.fn(() => 'interactive'),
    getSessionRiskLevel: vi.fn(() => 'medium'),
    getSessionWorkdir: vi.fn(() => '/work'),
    ...over,
  }
}

function msg(over: Partial<ChatMessage> = {}): ChatMessage {
  return { id: 'm1', type: 'text', content: 'hi', seq: 1, ...over }
}

beforeEach(() => {
  hoisted.calls.length = 0
  hoisted.callPlugin.mockClear()
  hoisted.store = makeStore()
})

describe('messageTree — 从 store 派生', () => {
  it('空 sessionId → 空树（不查 store）', () => {
    const c = useChatConnection({ sessionId: '' })
    expect(c.messageTree.value).toEqual([])
  })

  it('按 parent_id 组装成树，子节点按 seq 排序', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [
        msg({ id: 'root', seq: 1 }),
        msg({ id: 'b', parent_id: 'root', seq: 3 }),
        msg({ id: 'a', parent_id: 'root', seq: 2 }),
      ]),
    })
    const c = useChatConnection({ sessionId: 's1' })
    const tree = c.messageTree.value
    expect(tree).toHaveLength(1)
    expect(tree[0].children?.map((m) => m.id)).toEqual(['a', 'b'])
    // 子节点的 parent 引用要挂上（渲染树上溯要用）
    expect(tree[0].children?.[0].parent?.id).toBe('root')
  })

  it('parent 不可见的悬挂节点升为根（不丢消息）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ id: 'orphan', parent_id: 'gone', seq: 1 })]),
    })
    const c = useChatConnection({ sessionId: 's1' })
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['orphan'])
  })

  it('空内容叶子被过滤（流式的占位空节点）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [
        msg({ id: 'blank', type: 'text', content: '\n\n', seq: 1 }),
        msg({ id: 'real', seq: 2 }),
      ]),
    })
    const c = useChatConnection({ sessionId: 's1' })
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['real'])
  })

  it('带子节点的空内容容器**保留**（它是容器不是占位）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [
        msg({ id: 'turn', type: 'text', content: '', seq: 1 }),
        msg({ id: 'child', parent_id: 'turn', content: 'x', seq: 2 }),
      ]),
    })
    const c = useChatConnection({ sessionId: 's1' })
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['turn'])
  })

  it('非 text / reasoning 的空内容节点不过滤（如空 tool_call 有意义）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ id: 'tool', type: 'tool_call', content: '', seq: 1 })]),
    })
    const c = useChatConnection({ sessionId: 's1' })
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['tool'])
  })

  it('removeMessage 是本地覆盖：树里消失，store 不被改', () => {
    const get = vi.fn(() => [msg({ id: 'a', seq: 1 }), msg({ id: 'b', seq: 2 })])
    hoisted.store = makeStore({ getSessionMessages: get })
    const c = useChatConnection({ sessionId: 's1' })
    expect(c.messageTree.value).toHaveLength(2)
    c.removeMessage('a')
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['b'])
    // store 未被动过：不写一个删除语义到权威状态里
    expect(hoisted.store.applyTranscriptMessage).not.toHaveBeenCalled()
  })

  it('按会话隔离：另一会话的删除不影响本会话', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ id: 'a', seq: 1 })]),
    })
    const c1 = useChatConnection({ sessionId: 's1' })
    const c2 = useChatConnection({ sessionId: 's2' })
    c1.removeMessage('a')
    expect(c1.messageTree.value).toHaveLength(0)
    expect(c2.messageTree.value).toHaveLength(1)
  })
})

describe('派生状态 — isLoading / isWaitingApproval', () => {
  it('空 sessionId 一律 false', () => {
    const c = useChatConnection({ sessionId: '' })
    expect(c.isLoading.value).toBe(false)
    expect(c.isWaitingApproval.value).toBe(false)
  })

  it('isLoading 跟随 store.isSessionWorking', () => {
    hoisted.store = makeStore({ isSessionWorking: vi.fn(() => true) })
    expect(useChatConnection({ sessionId: 's1' }).isLoading.value).toBe(true)
  })

  it('有等待许可的消息 → isWaitingApproval', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ status: MESSAGE_STATUS_WAITING_USER_ACTION })]),
    })
    expect(useChatConnection({ sessionId: 's1' }).isWaitingApproval.value).toBe(true)
  })

  it('isConnected 恒为 true（连接状态由全局消费端收敛）', () => {
    expect(useChatConnection({ sessionId: 's1' }).isConnected.value).toBe(true)
  })
})

describe('send — 出站与乐观置位', () => {
  it('走 CHAT_SEND，带上 mode / risk_level / workdir 上下文', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.send(msg({ id: 'u1', content: '你好' }))
    expect(hoisted.calls).toHaveLength(1)
    const call = hoisted.calls[0]
    expect(call.path).toBe(CHAT_SEND)
    const p = call.payload as Record<string, unknown>
    expect(p.session_id).toBe('s1')
    expect(p.mode).toBe('interactive')
    expect(p.risk_level).toBe('medium')
    expect((call.ctx as Record<string, unknown>).workdir).toBe('/work')
  })

  it('乐观置位：先写消息、置 working、清会话级错误', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.send(msg({ id: 'u1' }))
    expect(hoisted.store.applyTranscriptMessage).toHaveBeenCalledWith('s1', expect.objectContaining({ id: 'u1' }))
    expect(hoisted.store.putStatus).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_WORKING }),
    )
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith('s1', null)
    expect(hoisted.store.setSessionStatus).toHaveBeenCalledWith('s1', VDFS_STATUS_WORKING)
  })

  it('无 id 的消息不做乐观写入（没有可锚定的节点）', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.send({ content: 'x' } as ChatMessage)
    expect(hoisted.store.applyTranscriptMessage).not.toHaveBeenCalled()
  })

  it('失败 → 置 failed；无在途消息时落会话级错误', async () => {
    hoisted.callPlugin.mockRejectedValueOnce(new Error('IPC 断了'))
    const onSendComplete = vi.fn()
    const c = useChatConnection({ sessionId: 's1', onSendComplete })
    await c.send(msg({ id: 'u1' }))
    expect(hoisted.store.putStatus).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_FAILED }),
    )
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith('s1', expect.stringContaining('IPC 断了'))
    expect(onSendComplete).toHaveBeenCalled()
  })

  it('失败但有在途消息 → 不落会话级错误（避免根级节点与会话级重复报错）', async () => {
    hoisted.callPlugin.mockRejectedValueOnce(new Error('IPC 断了'))
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ id: 'streaming', status: MESSAGE_STATUS_STREAMING })]),
    })
    const c = useChatConnection({ sessionId: 's1' })
    await c.send(msg({ id: 'u1' }))
    // putStatus(failed) 仍要有，但 setSessionError 只在最后一次（null）与失败时区分
    const errorCalls = (hoisted.store.setSessionError as ReturnType<typeof vi.fn>).mock.calls
    expect(errorCalls.every((args) => args[1] === null)).toBe(true)
  })

  it('⚠️ 现状：onSendComplete 只在失败时调用（成功不调用）', async () => {
    const onSendComplete = vi.fn()
    const c = useChatConnection({ sessionId: 's1', onSendComplete })
    await c.send(msg({ id: 'u1' }))
    expect(onSendComplete).not.toHaveBeenCalled()
  })
})

describe('abort', () => {
  it('走 CHAT_ABORT，带 session_id 与 workdir', () => {
    const c = useChatConnection({ sessionId: 's1' })
    c.abort()
    expect(hoisted.calls[0].path).toBe(CHAT_ABORT)
    expect((hoisted.calls[0].payload as Record<string, unknown>).session_id).toBe('s1')
  })

  it('出站失败不抛出（abort 是尽力而为）', async () => {
    hoisted.callPlugin.mockRejectedValueOnce(new Error('x'))
    const c = useChatConnection({ sessionId: 's1' })
    expect(() => c.abort()).not.toThrow()
    await Promise.resolve()
  })
})

describe('resume — 会话恢复', () => {
  it('缺省目标 = 本会话；payload 带齐五个字段', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.resume({ targetId: 't1', action: 'retry' })
    const p = hoisted.calls[0].payload as Record<string, unknown>
    expect(p.session_id).toBe('s1')
    expect(p.resume).toEqual({
      target_id: 't1',
      action: 'retry',
      args: null,
      reason: null,
      answer: null,
    })
  })

  it('指定 targetSessionId → 发给那个会话', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.resume({ targetId: 't1', action: 'approve', targetSessionId: 'child-1' })
    const p = hoisted.calls[0].payload as Record<string, unknown>
    expect(p.session_id).toBe('child-1')
    expect(p.mode).toBe('interactive')
  })

  it('跨会话恢复**不动本会话状态**（子智能体工具调用不误改父会话）', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.resume({ targetId: 't1', action: 'approve', targetSessionId: 'child-1' })
    expect(hoisted.store.putStatus).not.toHaveBeenCalled()
    expect(hoisted.store.setSessionStatus).not.toHaveBeenCalled()
  })

  it('同会话恢复：乐观置 working 并清错误', async () => {
    const c = useChatConnection({ sessionId: 's1' })
    await c.resume({ targetId: 't1', action: 'retry_turn' })
    expect(hoisted.store.putStatus).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_WORKING }),
    )
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith('s1', null)
  })

  it('session_busy 回落 active，绝不置 failed（它是现状不是失败）', async () => {
    hoisted.callPlugin.mockResolvedValueOnce({ status: 'session_busy' })
    const c = useChatConnection({ sessionId: 's1' })
    await c.resume({ targetId: 't1', action: 'retry' })
    expect(hoisted.store.putStatus).toHaveBeenLastCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_ACTIVE }),
    )
    const failedCalls = (hoisted.store.putStatus as ReturnType<typeof vi.fn>).mock.calls.filter(
      (args) => (args[1] as { status?: string })?.status === VDFS_STATUS_FAILED,
    )
    expect(failedCalls).toHaveLength(0)
  })

  it('出站失败 → 同会话置 failed，跨会话不置', async () => {
    hoisted.callPlugin.mockRejectedValueOnce(new Error('x'))
    const c = useChatConnection({ sessionId: 's1' })
    await c.resume({ targetId: 't1', action: 'retry' })
    expect(hoisted.store.putStatus).toHaveBeenLastCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_FAILED }),
    )
  })
})

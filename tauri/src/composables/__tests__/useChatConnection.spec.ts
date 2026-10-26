/**
 * useChatConnection — 会话连接（发起动作 + 消息树派生）单测
 *
 * 本 composable 不订阅任何通道、不写 store 权威状态——它只做三件事：
 * **发起动作**（send / abort / resume）、**乐观置位**、**从 store 派生消息树**。
 * 因此这里全部用桩：store、VDFS 出站（`writeVdfs` / `runVdfsAction`）、地址方案
 * 都是可断言的假实现。
 *
 * 钉住的约定：
 * 1. **消息树**：空内容叶子（流式的占位空 body / notice）不渲染，
 *    但**带子节点的容器必须保留**；`removeMessage` 是**本地覆盖**、不污染 store；
 *    parent 不可见的悬挂节点升为根（不丢消息）。
 * 2. **乐观置位**：send 立刻置 working 并清会话级错误——这是「零回读」的前提，
 *    随后由后端节点状态覆盖。
 * 2b. **不做乐观回显**：发言只是往**会话地址**的收件箱写一条
 *    （`write(<A>/inbox/<消息 id>)`），落库发生在"被消费那一刻"。前端抢先
 *    生成节点会让"已发出"与"已处理"在界面上无法区分，因此 "消息出现在流里"
 *    只能由后端的权威帧说了算。
 * 2c. **输入面只有地址，没有协议路由**（本文件的核心约定）：三个动作的落点
 *    分别是 `<A>/inbox/<iid>` / `<A>` + `<A>/inbox` / `<A>/message/<mid>`。
 *    「住在哪个空间」由地址承载——这正是子智能体空间里发消息不再跑到父智能体
 *    的原因（路由是 address-less 的，恒落在根实例上）。
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
import type { ChatNode } from '@/schemas/chat_node'
import {
  NODE_STATUS_STREAMING,
  NODE_STATUS_WAITING_USER_ACTION,
  NODE_KIND_BODY,
  NODE_KIND_SKELETON,
  NODE_KIND_TYPING,
} from '@/schemas/chat_node'
import { typingIdOf } from '@/registry/messageTypes'
import { VDFS_STATUS_ACTIVE, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING } from '@/schemas/vdfs'

/** 地址方案（运行期数据；测试里给一份固定的） */
const SCHEME = { mountDir: '/root/session', messagesSeg: 'message', inboxSeg: 'inbox' }
/** 本会话的地址 = `<挂载目录>/<id>` */
const ADDR = '/root/session/s1'

const hoisted = vi.hoisted(() => {
  return {
    /** writeVdfs 的调用记录 */
    writes: [] as Array<{ path: string; text: string; opts?: unknown }>,
    /** runVdfsAction 的调用记录 */
    actions: [] as Array<{ path: string; action: string; payload?: unknown; ctx?: unknown }>,
    writeVdfs: vi.fn(),
    runVdfsAction: vi.fn(),
    store: {} as Record<string, unknown>,
    /** useReleasedSkeletonsStore 的桩：本文件只关心 expandSkeleton 会摘除释放标记 */
    released: {} as Record<string, unknown>,
    /** logger 的桩（判据 5 的告警断言走它） */
    logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
  }
})

vi.mock('@/services/vdfs', () => ({
  writeVdfs: hoisted.writeVdfs,
  runVdfsAction: hoisted.runVdfsAction,
}))
vi.mock('@/services/vdfsScheme', () => ({
  // 忠实于生产契约：`ensureSessionScheme(mountDir)` 返回的 `mountDir`
  // 就是请求的那个（省略参数 = 默认挂载目录那份）。
  ensureSessionScheme: vi.fn(async (mountDir?: string) => ({
    mountDir: mountDir ?? '/root/session',
    messagesSeg: 'message',
    inboxSeg: 'inbox',
  })),
}))
vi.mock('@/stores/sessions', () => ({ useSessionsStore: () => hoisted.store }))
// 纯函数（subtreeOf / statsOf / forgetSession）保持真实实现——断言要的就是
// 它们的真实产物；被桩掉的只有 store（useReleasedSkeletonsStore）。
vi.mock('@/stores/skeletonLazy', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/stores/skeletonLazy')>()
  return { ...actual, useReleasedSkeletonsStore: () => hoisted.released }
})
vi.mock('@/utils/logger', () => ({ logger: hoisted.logger }))

import { useChatConnection, type UseChatConnectionOptions } from '../useChatConnection'

function makeStore(over: Record<string, unknown> = {}) {
  return {
    getSessionMessages: vi.fn(() => [] as ChatNode[]),
    isSessionWorking: vi.fn(() => false),
    applyTranscriptMessage: vi.fn(),
    applyTranscriptMessages: vi.fn(),
    removeMessages: vi.fn(),
    putStatus: vi.fn(),
    setSessionError: vi.fn(),
    setSessionStatus: vi.fn(),
    getSessionMode: vi.fn(() => 'interactive'),
    getSessionRiskLevel: vi.fn(() => 'medium'),
    getSessionWorkdir: vi.fn(() => '/work'),
    /** 会话 → 挂载目录的寻址镜像（跨会话恢复时用） */
    mountDirOf: vi.fn(() => SCHEME.mountDir),
    ...over,
  }
}

/** 建一个连接（默认就是本会话；测别的会话时显式覆盖） */
function conn(over: Partial<UseChatConnectionOptions> = {}) {
  return useChatConnection({ sessionId: 's1', sessionAddr: ADDR, ...over })
}

function msg(over: Partial<ChatNode> = {}): ChatNode {
  return { id: 'm1', type: NODE_KIND_BODY, content: 'hi', seq: 1, ...over }
}

beforeEach(() => {
  hoisted.writes.length = 0
  hoisted.actions.length = 0
  hoisted.writeVdfs.mockReset()
  hoisted.runVdfsAction.mockReset()
  hoisted.logger.warn.mockReset()
  hoisted.logger.error.mockReset()
  hoisted.writeVdfs.mockImplementation(async (path: string, text: string, opts?: unknown) => {
    hoisted.writes.push({ path, text, opts })
    return { created: true }
  })
  hoisted.runVdfsAction.mockImplementation(
    async (path: string, action: string, payload?: unknown, ctx?: unknown) => {
      hoisted.actions.push({ path, action, payload, ctx })
      return { action, ok: true, message: 'ok' }
    },
  )
  hoisted.store = makeStore()
  hoisted.released = { markExpanded: vi.fn(), markReleased: vi.fn(), isReleased: vi.fn(() => false), summaryOf: vi.fn() }
})

describe('messageTree — 从 store 派生', () => {
  it('空 sessionId → 空树（不查 store）', () => {
    const c = conn({ sessionId: '', sessionAddr: '' })
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
    const c = conn()
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
    const c = conn()
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['orphan'])
  })

  it('空内容叶子被过滤（流式的占位空节点）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [
        msg({ id: 'blank', type: NODE_KIND_BODY, content: '\n\n', seq: 1 }),
        msg({ id: 'real', seq: 2 }),
      ]),
    })
    const c = conn()
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['real'])
  })

  it('带子节点的空内容容器**保留**（它是容器不是占位）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [
        msg({ id: 'turn', type: NODE_KIND_SKELETON, content: '', seq: 1 }),
        msg({ id: 'child', parent_id: 'turn', content: 'x', seq: 2 }),
      ]),
    })
    const c = conn()
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['turn'])
  })

  it('非正文 / 折叠叶的空内容节点不过滤（如空骨架有意义）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ id: 'tool', type: NODE_KIND_SKELETON, content: '', seq: 1 })]),
    })
    const c = conn()
    expect(c.messageTree.value.map((m) => m.id)).toEqual(['tool'])
  })

  it('removeMessage 是本地覆盖：树里消失，store 不被改', () => {
    const get = vi.fn(() => [msg({ id: 'a', seq: 1 }), msg({ id: 'b', seq: 2 })])
    hoisted.store = makeStore({ getSessionMessages: get })
    const c = conn()
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
    const c1 = conn()
    const c2 = conn({ sessionId: 's2', sessionAddr: '/root/session/s2' })
    c1.removeMessage('a')
    expect(c1.messageTree.value).toHaveLength(0)
    expect(c2.messageTree.value).toHaveLength(1)
  })
})

describe('typing 占位合成（[T-151] 端到端：合成挂进 messageTree）', () => {
  // 根级、正在 streaming 的骨架就是 typing 的宿主（形状判据：无父 + skeleton）
  const runHost = (status: ChatNode['status']): ChatNode => ({
    id: 'run1',
    type: NODE_KIND_SKELETON,
    status,
    seq: 1,
  })

  it('streaming 空骨架 → 树里出现一条 typing 子', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [runHost(NODE_STATUS_STREAMING)]),
    })
    const tree = conn().messageTree.value
    expect(tree).toHaveLength(1)
    expect(tree[0].children?.map((c) => c.id)).toEqual([typingIdOf('run1')])
    expect(tree[0].children?.[0].type).toBe(NODE_KIND_TYPING)
  })

  it('真实子到达 → typing 让位（替换），不再合成', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [
        runHost(NODE_STATUS_STREAMING),
        msg({ id: 'txt', parent_id: 'run1', seq: 2, status: NODE_STATUS_STREAMING }),
      ]),
    })
    const tree = conn().messageTree.value
    expect(tree[0].children?.map((c) => c.id)).toEqual(['txt'])
  })

  it('骨架收束 → typing 消失（父收束）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [runHost('completed')]),
    })
    const tree = conn().messageTree.value
    expect(tree[0].children ?? []).toHaveLength(0)
  })
})

describe('派生状态 — isLoading / isWaitingApproval', () => {
  it('空 sessionId 一律 false', () => {
    const c = conn({ sessionId: '', sessionAddr: '' })
    expect(c.isLoading.value).toBe(false)
    expect(c.isWaitingApproval.value).toBe(false)
  })

  it('isLoading 跟随 store.isSessionWorking', () => {
    hoisted.store = makeStore({ isSessionWorking: vi.fn(() => true) })
    expect(conn().isLoading.value).toBe(true)
  })

  it('有等待许可的消息 → isWaitingApproval', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ status: NODE_STATUS_WAITING_USER_ACTION })]),
    })
    expect(conn().isWaitingApproval.value).toBe(true)
  })

  it('isConnected 恒为 true（连接状态由全局消费端收敛）', () => {
    expect(conn().isConnected.value).toBe(true)
  })
})

describe('send — 写收件箱条目 + 乐观置位', () => {
  it('写 `<A>/inbox/<消息 id>`，正文就是那条消息，ctx 带 workdir', async () => {
    const c = conn()
    await c.send(msg({ id: 'u1', content: '你好' }))
    expect(hoisted.writes).toHaveLength(1)
    const w = hoisted.writes[0]
    // 落点是**地址**：条目 id 就是消息 id（地址即身份）
    expect(w.path).toBe(`${ADDR}/inbox/u1`)
    expect(JSON.parse(w.text)).toMatchObject({ id: 'u1', content: '你好' })
    // workdir 走调用上下文：后端把它作为 start_turn 回退链的第一档
    expect((w.opts as { ctx: Record<string, unknown> }).ctx).toEqual({
      workdir: '/work',
      session_id: 's1',
    })
    // 发言不再是协议路由
    expect(hoisted.actions).toHaveLength(0)
  })

  it('子智能体空间的会话：写它**自己那个空间**的收件箱（不落回根空间）', async () => {
    const subAddr = '/root/agent/reviewer/session/sub-1'
    const c = conn({ sessionId: 'sub-1', sessionAddr: subAddr })
    await c.send(msg({ id: 'u1' }))
    expect(hoisted.writes[0].path).toBe(`${subAddr}/inbox/u1`)
  })

  it('乐观置位：置 working + 清会话级错误，但**不写消息**', async () => {
    const c = conn()
    await c.send(msg({ id: 'u1' }))
    expect(hoisted.store.putStatus).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_WORKING }),
    )
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith('s1', null)
    expect(hoisted.store.setSessionStatus).toHaveBeenCalledWith('s1', VDFS_STATUS_WORKING)
    // 发言只是入队：消息何时出现在流里由**后端消费**决定（权威帧），前端不抢先生成
    expect(hoisted.store.applyTranscriptMessage).not.toHaveBeenCalled()
  })

  it('地址不可解析 → 不发请求，如实置 failed + 会话级错误', async () => {
    const onSendComplete = vi.fn()
    // 既没有调用方给的地址，store 的寻址镜像也查不到 ⇒ 无从寻址
    hoisted.store = makeStore({ mountDirOf: vi.fn(() => undefined) })
    const c = conn({ sessionAddr: '', onSendComplete })
    await c.send(msg({ id: 'u1' }))
    expect(hoisted.writes).toHaveLength(0)
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith(
      's1',
      expect.stringContaining('地址方案未就绪'),
    )
    expect(onSendComplete).toHaveBeenCalled()
  })

  it('失败 → 置 failed；无在途消息时落会话级错误', async () => {
    hoisted.writeVdfs.mockRejectedValueOnce(new Error('IPC 断了'))
    const onSendComplete = vi.fn()
    const c = conn({ onSendComplete })
    await c.send(msg({ id: 'u1' }))
    expect(hoisted.store.putStatus).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_FAILED }),
    )
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith('s1', expect.stringContaining('IPC 断了'))
    expect(onSendComplete).toHaveBeenCalled()
  })

  it('失败但有在途消息 → 不落会话级错误（避免根级节点与会话级重复报错）', async () => {
    hoisted.writeVdfs.mockRejectedValueOnce(new Error('IPC 断了'))
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [msg({ id: 'streaming', status: NODE_STATUS_STREAMING })]),
    })
    const c = conn()
    await c.send(msg({ id: 'u1' }))
    // putStatus(failed) 仍要有，但 setSessionError 只在最后一次（null）与失败时区分
    const errorCalls = (hoisted.store.setSessionError as ReturnType<typeof vi.fn>).mock.calls
    expect(errorCalls.every((args) => args[1] === null)).toBe(true)
  })

  it('⚠️ 现状：onSendComplete 只在失败时调用（成功不调用）', async () => {
    const onSendComplete = vi.fn()
    const c = conn({ onSendComplete })
    await c.send(msg({ id: 'u1' }))
    expect(onSendComplete).not.toHaveBeenCalled()
  })
})

describe('abort — 两个动作，作用在两个对象上', () => {
  it('中止在途轮次 + 清空排队（成对，缺一个就会「点了停止它又跑起来」）', async () => {
    const c = conn()
    c.abort()
    await vi.waitFor(() => expect(hoisted.actions).toHaveLength(2))
    const [abortCall, clearCall] = hoisted.actions
    expect(abortCall.path).toBe(ADDR)
    expect(abortCall.action).toBe('abort')
    expect(clearCall.path).toBe(`${ADDR}/inbox`)
    expect(clearCall.action).toBe('clear')
    expect((abortCall.ctx as Record<string, unknown>).session_id).toBe('s1')
  })

  it('子智能体空间：两个动作都打在子空间地址上', async () => {
    const subAddr = '/root/agent/reviewer/session/sub-1'
    const c = conn({ sessionId: 'sub-1', sessionAddr: subAddr })
    c.abort()
    await vi.waitFor(() => expect(hoisted.actions).toHaveLength(2))
    expect(hoisted.actions.map((a) => a.path)).toEqual([subAddr, `${subAddr}/inbox`])
  })

  it('出站失败不抛出（abort 是尽力而为）', async () => {
    hoisted.runVdfsAction.mockRejectedValue(new Error('x'))
    const c = conn()
    expect(() => c.abort()).not.toThrow()
    await vi.waitFor(() => expect(hoisted.runVdfsAction).toHaveBeenCalled())
  })

  it('一个失败不影响另一个发出（停在途 / 清排队互不依赖）', async () => {
    hoisted.runVdfsAction
      .mockRejectedValueOnce(new Error('abort 失败'))
      .mockResolvedValueOnce({ action: 'clear', ok: true, message: 'ok' })
    const c = conn()
    c.abort()
    // 两个请求都发出去了（一个 rejected、一个 resolved）——`allSettled` 的意义所在
    await vi.waitFor(() => expect(hoisted.runVdfsAction).toHaveBeenCalledTimes(2))
    expect(hoisted.runVdfsAction.mock.calls.map((call) => call[1])).toEqual(['abort', 'clear'])
  })
})

describe('resume — 会话恢复', () => {
  it('缺省目标 = 本会话；动作落在 `<A>/message/<targetId>` 上', async () => {
    const c = conn()
    await c.resume({ targetId: 't1', action: 'retry' })
    expect(hoisted.actions).toHaveLength(1)
    const a = hoisted.actions[0]
    expect(a.path).toBe(`${ADDR}/message/t1`)
    // 动作词就是 ResumeAction 的线上词形，不另造一套
    expect(a.action).toBe('retry')
    expect(a.payload).toEqual({
      args: null,
      reason: null,
      answer: null,
      mode: 'interactive',
      risk_level: 'medium',
    })
    // 恢复不是入队：它必须落在当时那条消息上（入队会丢锚点）
    expect(hoisted.writes).toHaveLength(0)
  })

  it('payload 的 args / reason / answer 原样带给后端', async () => {
    const c = conn()
    await c.resume({
      targetId: 't1',
      action: 'supply',
      args: { q: 1 },
      reason: '不对',
      answer: { a: 2 },
    })
    expect(hoisted.actions[0].payload).toMatchObject({
      args: { q: 1 },
      reason: '不对',
      answer: { a: 2 },
    })
  })

  it('指定 targetSessionId → 落到那个会话的地址（挂载目录按 store 的镜像反查）', async () => {
    const c = conn()
    await c.resume({ targetId: 't1', action: 'approve', targetSessionId: 'child-1' })
    expect(hoisted.store.mountDirOf).toHaveBeenCalledWith('child-1')
    expect(hoisted.actions[0].path).toBe('/root/session/child-1/message/t1')
  })

  it('跨会话恢复**不动本会话状态**（子智能体工具调用不误改父会话）', async () => {
    const c = conn()
    await c.resume({ targetId: 't1', action: 'approve', targetSessionId: 'child-1' })
    expect(hoisted.store.putStatus).not.toHaveBeenCalled()
    expect(hoisted.store.setSessionStatus).not.toHaveBeenCalled()
  })

  it('同会话恢复：乐观置 working 并清错误', async () => {
    const c = conn()
    await c.resume({ targetId: 't1', action: 'retry_subtree' })
    expect(hoisted.store.putStatus).toHaveBeenCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_WORKING }),
    )
    expect(hoisted.store.setSessionError).toHaveBeenCalledWith('s1', null)
  })

  it('session_busy 回落 active，绝不置 failed（它是现状不是失败）', async () => {
    hoisted.runVdfsAction.mockResolvedValueOnce({
      action: 'retry',
      ok: false,
      message: '会话正在处理中，无法 retry',
      data: { session_id: 's1', status: 'session_busy' },
    })
    const c = conn()
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
    hoisted.runVdfsAction.mockRejectedValueOnce(new Error('x'))
    const c = conn()
    await c.resume({ targetId: 't1', action: 'retry' })
    expect(hoisted.store.putStatus).toHaveBeenLastCalledWith(
      's1',
      expect.objectContaining({ status: VDFS_STATUS_FAILED }),
    )
  })
})

describe('expandSkeleton — 折叠骨架按需重取（[T-150] 只读）', () => {
  it('动作落在 `<A>/message/<mid>`，payload 带 depth，拿回的子树走唯一落地口装回转写', async () => {
    hoisted.runVdfsAction.mockResolvedValueOnce({
      action: 'expand_skeleton',
      ok: true,
      message: 'ok',
      data: { session_id: 's1', root: 't1', nodes: [{ id: 'c1', parent_id: 't1' }] },
    })
    const c = conn()
    await c.expandSkeleton('t1', 'all')
    // mockResolvedValueOnce 会替换掉记录用的基础实现，故从 mock.calls 取调用参数
    const [path, action, payload] = hoisted.runVdfsAction.mock.calls[0] as [
      string,
      string,
      Record<string, unknown>,
    ]
    expect(path).toBe(`${ADDR}/message/t1`)
    expect(action).toBe('expand_skeleton')
    expect(payload).toEqual({ depth: 'all' })
    // 只读：不动运行态
    expect(hoisted.store.putStatus).not.toHaveBeenCalled()
    expect(hoisted.store.setSessionStatus).not.toHaveBeenCalled()
    // 装回转写 + 摘除释放标记
    expect(hoisted.store.applyTranscriptMessages).toHaveBeenCalledWith('s1', [
      { id: 'c1', parent_id: 't1' },
    ])
    expect(hoisted.released.markExpanded).toHaveBeenCalledWith('s1', 't1')
  })

  it('未生效（ok:false 或无 nodes）→ 不装回、不摘标记', async () => {
    hoisted.runVdfsAction.mockResolvedValueOnce({
      action: 'expand_skeleton',
      ok: false,
      message: '骨架已不在',
    })
    const c = conn()
    await c.expandSkeleton('ghost')
    expect(hoisted.store.applyTranscriptMessages).not.toHaveBeenCalled()
    expect(hoisted.released.markExpanded).not.toHaveBeenCalled()
  })

  it('出站异常不抛出（尽力而为）', async () => {
    hoisted.runVdfsAction.mockRejectedValueOnce(new Error('IPC 断了'))
    const c = conn()
    await expect(c.expandSkeleton('t1')).resolves.toBeUndefined()
  })

  it('判据 5（反向）：取回 0 条而账本记着 N>0 ⇒ 保持释放态 + 告警', async () => {
    hoisted.runVdfsAction.mockResolvedValueOnce({
      action: 'expand_skeleton',
      ok: true,
      message: 'ok',
      data: { session_id: 's1', root: 't1', nodes: [] },
    })
    hoisted.released.summaryOf = vi.fn(() => ({ count: 3, byType: { body: 3 }, running: 0 }))
    const c = conn()
    await c.expandSkeleton('t1')
    // 不装回（本来也没有节点）、**不摘标记**——保持折叠态，用户可重试
    expect(hoisted.store.applyTranscriptMessages).not.toHaveBeenCalled()
    expect(hoisted.released.markExpanded).not.toHaveBeenCalled()
    expect(hoisted.logger.warn).toHaveBeenCalledWith(
      'useChatConnection',
      expect.stringContaining('取回 0 条，但账本记着 3 条'),
    )
  })

  it('取回 0 条而账本计数也是 0 ⇒ 正常的「本来就没子节点」，照常摘标记', async () => {
    hoisted.runVdfsAction.mockResolvedValueOnce({
      action: 'expand_skeleton',
      ok: true,
      message: 'ok',
      data: { session_id: 's1', root: 't1', nodes: [] },
    })
    hoisted.released.summaryOf = vi.fn(() => ({ count: 0, byType: {}, running: 0 }))
    const c = conn()
    await c.expandSkeleton('t1')
    expect(hoisted.released.markExpanded).toHaveBeenCalledWith('s1', 't1')
  })
})

describe('skeletonLazy — 折叠释放 / 展开重取（[] 前端半写侧）', () => {
  /** 一棵已完成骨架带两个后代（一个流式中） */
  const flat = (): ChatNode[] => [
    { id: 'skel', type: NODE_KIND_SKELETON, status: 'completed', seq: 1 },
    { id: 'c1', parent_id: 'skel', type: NODE_KIND_BODY, status: 'completed', seq: 2 },
    { id: 'c2', parent_id: 'skel', type: NODE_KIND_BODY, status: NODE_STATUS_STREAMING, seq: 3 },
  ]

  it('收起已终态骨架 ⇒ 摘掉后代（留 root）、账本记 stats', () => {
    hoisted.store = makeStore({ getSessionMessages: vi.fn(() => flat()) })
    const c = conn()
    c.skeletonLazy.onCollapse('skel', false)
    expect(hoisted.store.removeMessages).toHaveBeenCalledWith('s1', ['c1', 'c2'])
    expect(hoisted.released.markReleased).toHaveBeenCalledWith('s1', 'skel', {
      count: 2,
      byType: { body: 2 },
      running: 1,
    })
  })

  it('流式 / 待审批中的骨架**不释放**（不丢流式态）', () => {
    for (const status of [NODE_STATUS_STREAMING, NODE_STATUS_WAITING_USER_ACTION]) {
      hoisted.store = makeStore({
        getSessionMessages: vi.fn(() => [
          { id: 'skel', type: NODE_KIND_SKELETON, status, seq: 1 },
          { id: 'c1', parent_id: 'skel', type: NODE_KIND_BODY, seq: 2 },
        ]),
      })
      conn().skeletonLazy.onCollapse('skel', false)
      expect(hoisted.store.removeMessages).not.toHaveBeenCalled()
      expect(hoisted.released.markReleased).not.toHaveBeenCalled()
    }
  })

  it('无后代的骨架收起 ⇒ 不入账（没什么可释放）', () => {
    hoisted.store = makeStore({
      getSessionMessages: vi.fn(() => [{ id: 'skel', type: NODE_KIND_SKELETON, seq: 1 }]),
    })
    conn().skeletonLazy.onCollapse('skel', false)
    expect(hoisted.released.markReleased).not.toHaveBeenCalled()
  })

  it('展开时账本说释放着 ⇒ 触发按需重取；未释放 ⇒ 不发请求', async () => {
    hoisted.released.isReleased = vi.fn((_sid: string, mid: string) => mid === 'skel')
    const c = conn()
    c.skeletonLazy.onCollapse('skel', true)
    await vi.waitFor(() => expect(hoisted.runVdfsAction).toHaveBeenCalledTimes(1))
    expect(hoisted.runVdfsAction.mock.calls[0][1]).toBe('expand_skeleton')

    hoisted.runVdfsAction.mockClear()
    c.skeletonLazy.onCollapse('other', true)
    await Promise.resolve()
    expect(hoisted.runVdfsAction).not.toHaveBeenCalled()
  })

  it('isReleased / summaryOf 按本会话 id 问账本（组件不持会话 id）', () => {
    const c = conn()
    c.skeletonLazy.isReleased('skel')
    expect(hoisted.released.isReleased).toHaveBeenCalledWith('s1', 'skel')
    c.skeletonLazy.summaryOf('skel')
    expect(hoisted.released.summaryOf).toHaveBeenCalledWith('s1', 'skel')
  })
})

// ==================== 分支（[D-173] 并存两支 + 一个选中态）====================

/** 一条成员根：`selected` 决定它进不进可见集。 */
function branchRoot(id: string, candidates: string[], index: number, selected: boolean): ChatNode {
  return msg({
    id,
    seq: index + 1,
    type: NODE_KIND_SKELETON,
    content: '',
    meta: {
      branch: { group: candidates[0], candidates, index, total: candidates.length, selected },
    },
  })
}

describe('可见集 — 未选中支连同子树不渲染，但一条都没少', () => {
  /** 交出去的树上**所有**可见 id（根 + 逐层子），排序后好比对。 */
  const visibleIds = (tree: ChatNode[]): string[] =>
    tree.flatMap(m => [m.id, ...visibleIds(m.children ?? [])]).sort()

  /** 两支并存：t1（旧，未选中）带一条结果，t2（新，在读）带一条结果。 */
  function twoBranches(): ChatNode[] {
    return [
      msg({ id: 'u1', seq: 0, parent_id: undefined }),
      branchRoot('t1', ['t1', 't2'], 0, false),
      msg({ id: 'r1', seq: 2, parent_id: 't1', content: '第一次的答复' }),
      branchRoot('t2', ['t1', 't2'], 1, true),
      msg({ id: 'r2', seq: 4, parent_id: 't2', content: '重试的答复' }),
    ]
  }

  it('只画在读的那一支（含它的子孙）；共同祖先留着', () => {
    hoisted.store = makeStore({ getSessionMessages: vi.fn(twoBranches) })
    expect(visibleIds(conn().messageTree.value)).toEqual(['r2', 't2', 'u1'])
  })

  it('切过来之后画另一支——**数据一条没少**：藏的是可见集，不是 store', () => {
    const all = twoBranches()
    // 后端换的是标注（选择事实落库后重发组内帧），前端只是重算可见集
    const flipped = all.map(m => {
      const b = (m.meta as Record<string, any>)?.branch
      if (!b) return m
      return { ...m, meta: { ...m.meta, branch: { ...b, selected: m.id === 't1' } } }
    })
    hoisted.store = makeStore({ getSessionMessages: vi.fn(() => flipped) })
    expect(visibleIds(conn().messageTree.value)).toEqual(['r1', 't1', 'u1'])
    // 「看不见」≠「不存在」：集合本身一条没少，也没有谁被摘掉
    expect(flipped).toHaveLength(5)
    expect(hoisted.store.removeMessages).not.toHaveBeenCalled()
  })

  it('没有分支的会话：可见集与原来一致（收敛排在空壳叶子之前不能改变它）', () => {
    const flat = [msg({ id: 'u1', seq: 0 }), msg({ id: 't1', seq: 1, parent_id: 'u1' })]
    hoisted.store = makeStore({ getSessionMessages: vi.fn(() => flat) })
    expect(visibleIds(conn().messageTree.value)).toEqual(['t1', 'u1'])
  })
})

describe('switchBranch — 一个动作，不动运行态', () => {
  it('落在 `<A>/message/<mid>`，载荷只有 {to}（目标候选 id 原样交回后端）', async () => {
    await conn().switchBranch('t2', 't1')
    expect(hoisted.actions).toHaveLength(1)
    const a = hoisted.actions[0]
    expect(a.path).toBe(`${ADDR}/message/t2`)
    expect(a.action).toBe('switch_branch')
    expect(a.payload).toEqual({ to: 't1' })
    expect(a.ctx).toEqual({ workdir: '/work', session_id: 's1' })
  })

  it('不置 working / failed：换个读法不是跑了一轮', async () => {
    const c = conn()
    await c.switchBranch('t2', 't1')
    expect(hoisted.store.putStatus).not.toHaveBeenCalled()
    expect(hoisted.store.setSessionStatus).not.toHaveBeenCalled()
    expect(hoisted.store.setSessionError).not.toHaveBeenCalled()
    expect(c.isLoading.value).toBe(false)
    // 也不经 `apply`：改的是选择格，不是任何一条消息
    expect(hoisted.store.applyTranscriptMessage).not.toHaveBeenCalled()
    expect(hoisted.writes).toHaveLength(0)
  })

  it('后端说没改成 ⇒ 出声（跨组被拒时界面停在原来那一支，只有这句话能补上差异）', async () => {
    hoisted.runVdfsAction.mockResolvedValueOnce({
      action: 'switch_branch',
      ok: false,
      message: 'm3 与 m1 不在同一分支组',
    })
    await conn().switchBranch('m1', 'm3')
    expect(hoisted.logger.warn).toHaveBeenCalled()
  })

  it('出站异常不抛出（尽力而为）', async () => {
    hoisted.runVdfsAction.mockRejectedValueOnce(new Error('IPC 断了'))
    await expect(conn().switchBranch('t2', 't1')).resolves.toBeUndefined()
    expect(hoisted.logger.error).toHaveBeenCalled()
  })
})

// @vitest-environment happy-dom
/**
 * 流式端到端复现测试：真实 store → 转写流收敛 → 树构建（复刻
 * useChatConnection.messageTree 的分形逻辑）→ MessageNode 渲染。
 *
 * 目的：复现"长会话流模式下 Reason 块不结束 / 后续内容显示进 Reason / Turn 不显示"
 * 的纯前端可见结果，并作为修复的回归锚。
 *
 * 帧按**新协议**驱动（帧 = 一条 `ChatMessage`，语义全在字段上：`delta` 追加 /
 * `content` 替换 / `status=removed` 移除 / 其余字段合并），与 `MainLayout` 的真实
 * 接线同路（`transcriptStream` → store）。
 *
 * 注意**不再有整条替换的 `upsert`**：状态迁移帧只带身份 + 状态（与后端
 * `state_frame` 同构），落地是**合并**——这正是要锚定的新语义（旧协议下这种部分帧
 * 会把 `type` / `parent_id` 一并抹掉，节点随即失去渲染语义）。
 */
import { describe, expect, it, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, h, nextTick } from 'vue'
import MessageNode from '../MessageNode.vue'
import { useSessionsStore } from '@/stores/sessions'
import { useChatConnection } from '@/composables/useChatConnection'
import type { ChatMessage } from '@/schemas/chat_message'
import {
  applyNodeEvent,
  flushPendingFrames,
  startTranscriptStream,
  stopTranscriptStream,
  type NodeEvent,
  type TranscriptStreamSink,
} from '@/services/transcriptStream'

/**
 * 驱动一帧并**立即落地**。
 *
 * `transcriptStream` 在生产里把 ~48ms 窗口内的帧攒批提交（性能优化，不改变帧语义）；
 * 本测试按帧断言，故每帧后显式冲刷，等价于「窗口 = 0」。
 */
function applyFrame(e: NodeEvent): void {
  applyNodeEvent(e)
  flushPendingFrames()
}

vi.mock('@/services/plugin', () => ({
  connectPlugin: vi.fn(async () => ({ connectionId: 'c1', close: vi.fn(async () => {}) })),
  callPlugin: vi.fn(),
  setLastWorkdir: vi.fn(),
  getLastWorkdir: vi.fn(),
}))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

const SID = 's1'

/** 帧序号：后端单调计数器（跳号会被判为丢帧，测试里逐帧递增） */
let seq = 0

/** 造一帧：载荷就是一条 `ChatMessage`（帧自给自足） */
function frame(message: ChatMessage): NodeEvent {
  seq += 1
  return { session_id: SID, seq, message }
}

/** 造一帧**增量**（流式热路径）：只带 `delta`，追加到目标节点正文尾部 */
function append(messageId: string, delta: string): NodeEvent {
  seq += 1
  return { session_id: SID, seq, message: { id: messageId, delta } }
}

/** 与 `MainLayout` 的接线逐字一致 */
function storeSink(): TranscriptStreamSink {
  const store = useSessionsStore()
  return {
    messages: (sid, ms) => store.applyTranscriptMessages(sid, ms),
    applySessionState: (sid, node) => store.applySessionState(sid, node),
    reload: async (sid) => {
      await store.loadMessages(sid)
    },
  }
}

/** 与后端 `emit_streaming_start` / 首帧全量 / finalize 一致的**完整消息**构造器 */
function turn(id: string, status: ChatMessage['status'] = 'streaming'): ChatMessage {
  return { id, role: 'assistant', type: 'turn', status }
}
function reasoning(
  id: string,
  parent: string,
  content: string,
  status: ChatMessage['status'] = 'streaming',
): ChatMessage {
  return { id, parent_id: parent, role: 'assistant', type: 'reasoning', status, content }
}
function text(
  id: string,
  parent: string,
  content: string,
  status: ChatMessage['status'] = 'streaming',
): ChatMessage {
  return { id, parent_id: parent, role: 'assistant', type: 'text', status, content }
}
function toolCall(
  id: string,
  parent: string,
  name: string,
  args: string,
  status: ChatMessage['status'] = 'streaming',
): ChatMessage {
  return { id, parent_id: parent, role: 'assistant', type: 'tool_call', name, status, content: args }
}
/**
 * 状态帧：只带身份 + 状态，**不带正文**（与后端 `state_frame` 同构）。
 *
 * 旧协议下这种部分帧会把 `type` / `parent_id` / `content` 一并抹掉；新协议下是
 * **合并**——这里刻意用最小帧，正是要钉住「状态迁移不吞身份与正文」这条不变式。
 */
function settled(m: ChatMessage, status: ChatMessage['status'] = 'completed'): ChatMessage {
  return { id: m.id, status }
}

/** 复刻 useChatConnection.messageTree（同源逻辑，锚定其行为） */
function buildTree(all: ChatMessage[]): ChatMessage[] {
  const rawChildren: Record<string, ChatMessage[]> = {}
  all.forEach((msg) => {
    if (msg.parent_id) {
      if (!rawChildren[msg.parent_id]) rawChildren[msg.parent_id] = []
      rawChildren[msg.parent_id].push(msg)
    }
  })
  const parentsWithChildren = new Set(Object.keys(rawChildren).filter((pid) => rawChildren[pid].length > 0))
  const isEmptyContentNode = (msg: ChatMessage): boolean => {
    const t = msg.type || 'text'
    if (t !== 'text' && t !== 'reasoning') return false
    const c = msg.content
    const text = typeof c === 'string' ? c : c && typeof c === 'object' && 'text' in c ? (c as { text: string }).text : ''
    return !text || text.trim().length === 0
  }
  const isEmptyLeaf = (msg: ChatMessage) => !parentsWithChildren.has(msg.id) && isEmptyContentNode(msg)
  const visible = all.filter((m) => !isEmptyLeaf(m))
  const childrenMap: Record<string, ChatMessage[]> = {}
  visible.forEach((msg) => {
    if (msg.parent_id) {
      if (!childrenMap[msg.parent_id]) childrenMap[msg.parent_id] = []
      childrenMap[msg.parent_id].push(msg)
    }
  })
  const rootMessages = visible.filter((msg) => !msg.parent_id || !visible.find((m) => m.id === msg.parent_id))
  const sortFn = (a: ChatMessage, b: ChatMessage) => (a.seq ?? a.timestamp ?? 0) - (b.seq ?? b.timestamp ?? 0)
  rootMessages.sort(sortFn)
  Object.values(childrenMap).forEach((children) => children.sort(sortFn))
  const buildNode = (msg: ChatMessage, parent?: ChatMessage): ChatMessage => {
    const node: ChatMessage = { ...msg }
    if (parent) node.parent = parent
    if (childrenMap[node.id]) node.children = childrenMap[node.id].map((c) => buildNode(c, node))
    return node
  }
  return rootMessages.map((m) => buildNode(m))
}

function mountTree(tree: ChatMessage[]) {
  return mount(
    {
      setup() {
        return () => tree.map((n) => h(MessageNode, { node: n, depth: 0, key: n.id }))
      },
    },
    { global: { provide: { resume: () => {} } } },
  )
}

describe('流式端到端：Turn/Reason 在多轮工具场景的可见性', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    seq = 0
    stopTranscriptStream()
    // 只接 sink（不建连接）：`applyNodeEvent` 就是连接回调内部的同一条路径
    void startTranscriptStream(storeSink())
  })

  it('多轮工具：每轮 Turn 最终 Completed，无残留「正在思考」骨架', () => {
    const feed = (e: NodeEvent) => applyFrame(e)

    // ── Turn 1：思考 → 文本 → 工具（结果在下一轮前）──
    const t1 = turn('T1')
    const r1 = reasoning('r1', 'T1', '想法')
    const x1 = text('x1', 'T1', '正文')
    const tc1 = toolCall('tc1', 'T1', 'read_file', '{"path":"a.rs"}')
    feed(frame(t1))
    feed(frame(r1))
    feed(frame(settled(r1)))
    feed(frame(x1))
    feed(frame(settled(x1)))
    feed(frame(tc1))

    // ── 工具结果 + Turn1 完成（后端 finalize 顺序：先子节点后根）──
    feed(frame({ id: 'res1', parent_id: 'tc1', role: 'tool', type: 'text', status: 'completed', content: '"ok"' }))
    feed(frame(settled(tc1)))
    feed(frame(settled(t1)))

    // ── Turn 2：纯文本收尾 ──
    const t2 = turn('T2')
    const r2 = reasoning('r2', 'T2', '再想')
    const x2 = text('x2', 'T2', '结论')
    feed(frame(t2))
    feed(frame(r2))
    feed(frame(settled(r2)))
    feed(frame(x2))
    feed(frame(settled(x2)))
    feed(frame(settled(t2)))

    const store = useSessionsStore()
    const tree = buildTree(store.getSessionMessages(SID))
    const w = mountTree(tree)

    // 1) 无任何等待骨架残留（每轮 Turn 均已有子节点）
    expect(w.findAll('.turn-pending').length).toBe(0)
    // 2) 两轮 Turn 均以 turn-group 呈现（透明容器）
    expect(w.findAll('.turn-group').length).toBe(2)
    // 3) 正文可见
    expect(w.text()).toContain('正文')
    expect(w.text()).toContain('结论')
  })

  it('流式中途（Reasoning streaming）：骨架消失、思考单行显示「思考中…」', () => {
    applyFrame(frame(turn('T1')))
    applyFrame(frame(reasoning('r1', 'T1', '部分思考')))

    const store = useSessionsStore()
    const tree = buildTree(store.getSessionMessages(SID))
    const w = mountTree(tree)
    // Turn 已有子节点 → 不显示等待骨架
    expect(w.findAll('.turn-pending').length).toBe(0)
    // 思考行头存在且是"思考中…"态
    const head = w.find('.node-head')
    expect(head.exists()).toBe(true)
    expect(head.text()).toContain('思考中')
  })

  it('增量帧（delta）拼进正文——流式热路径', () => {
    applyFrame(frame(turn('T1')))
    applyFrame(frame(text('x1', 'T1', '第一段')))
    applyFrame(append('x1', '，第二段'))

    const store = useSessionsStore()
    const msg = store.getSessionMessages(SID).find((m) => m.id === 'x1')
    expect(msg?.content, '增量必须逐步拼出完整正文').toBe('第一段，第二段')
  })

  it('状态帧不吞身份与正文（合并语义）', () => {
    applyFrame(frame(turn('T1')))
    applyFrame(frame(text('x1', 'T1', '正文')))
    // 只带 id + status 的最小状态帧：type / parent_id / content 都必须原样保留
    applyFrame(frame(settled({ id: 'x1' })))

    const store = useSessionsStore()
    const msg = store.getSessionMessages(SID).find((m) => m.id === 'x1')
    expect(msg?.status).toBe('completed')
    expect(msg?.type, '状态帧不得抹掉节点类型').toBe('text')
    expect(msg?.parent_id, '状态帧不得抹掉归属').toBe('T1')
    expect(msg?.content, '状态帧不得抹掉正文').toBe('正文')
  })

  it('空 Turn 未收帧（等待 LLM 首 token）：骨架存在且属于当前轮', () => {
    // 上一轮已完成
    const t0 = turn('T0')
    const x0 = text('x0', 'T0', '上轮')
    applyFrame(frame(t0))
    applyFrame(frame(x0))
    applyFrame(frame(settled(x0)))
    applyFrame(frame(settled(t0)))
    // 新一轮 Turn 已广播、尚无子节点
    applyFrame(frame(turn('T1')))

    const store = useSessionsStore()
    const tree = buildTree(store.getSessionMessages(SID))
    const w = mountTree(tree)
    expect(w.findAll('.turn-pending').length).toBe(1)
  })
})

/**
 * 渲染层回归：**增量的目标是「已有子节点」时，DOM 必须跟着动**。
 *
 * 上面那些用例只断言 store 里的消息（`getSessionMessages`），因此
 * `messageTree` 的节点复用签名若只看「自身字段 + 直接子节点 id」，这个缺陷
 * 完全逃得过测试网：store 里内容是对的，界面却停在旧内容。用户实测症状正是
 * 「增量始终不刷新；正文结束后（根 Turn 状态迁移那一刻）才突然完整」。
 *
 * 根因是**渲染失效口径不完整**：容器的渲染结果全部来自子树（Turn / ToolCall
 * 自身无正文），所以「已有子节点的正文增长」也必须使祖先的节点对象换新；
 * 否则 `:node` 引用不变 ⇒ Vue 判定 props 未变 ⇒ 整棵子树跳过更新。
 * 下面用**真实 composable + 真实渲染**（挂载方式与 `ModelChatPanel` 同构）钉住。
 */
describe('渲染层回归：增量必须进 DOM（不只进 store）', () => {
  /** 与 `ModelChatPanel` 的消费方式同构：`messageTree` → `MessageNode` v-for */
  const Panel = defineComponent({
    props: { sessionId: { type: String, required: true } },
    setup(props) {
      const chat = useChatConnection({ sessionId: props.sessionId })
      return () =>
        chat.messageTree.value.map((n) => h(MessageNode, { node: n, depth: 0, key: n.id }))
    },
  })

  beforeEach(() => {
    setActivePinia(createPinia())
    seq = 0
    stopTranscriptStream()
    void startTranscriptStream(storeSink())
  })

  it('父容器（Turn）已有子节点时，子节点的增量必须重渲染', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame({ id: 'T1', role: 'assistant', type: 'turn', status: 'streaming' }))
    applyFrame(
      frame({
        id: 'X1',
        parent_id: 'T1',
        role: 'assistant',
        type: 'text',
        status: 'streaming',
        content: '正文开头',
      }),
    )
    await nextTick()
    expect(w.text()).toContain('正文开头')

    // 这一帧只改叶子内容，不动任何容器的字段——旧的签名口径下它**看不见**
    applyFrame(append('X1', '，正文继续'))
    await nextTick()
    const store = useSessionsStore()
    expect(
      store.getSessionMessages(SID).find((m) => m.id === 'X1')?.content,
      'store 必须先累积（"累加正确"与"渲染刷新"是两件事）',
    ).toBe('正文开头，正文继续')
    expect(w.text(), '子节点增量必须进 DOM').toContain('正文继续')
  })

  it('思考（Reasoning）是独立节点，其增量同样必须重渲染', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame({ id: 'T1', role: 'assistant', type: 'turn', status: 'streaming' }))
    applyFrame(
      frame({
        id: 'R1',
        parent_id: 'T1',
        role: 'assistant',
        type: 'reasoning',
        status: 'streaming',
        content: '第一段思考',
      }),
    )
    await nextTick()
    applyFrame(append('R1', '，第二段思考'))
    await nextTick()
    // 分派类名由 registry 给出 ⇒ 钉住「思考是独立节点，没有被并进正文」
    expect(w.find('.type-reasoning').exists(), '思考必须是独立节点').toBe(true)
    // 折叠态的一行摘要由节点 content 渲染，它必须同步增长
    expect(w.text(), '思考的增量必须进 DOM').toContain('第二段思考')
  })

  it('子节点终态（状态迁移）必须立刻反映，不等祖先的结构变化', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame({ id: 'T1', role: 'assistant', type: 'turn', status: 'streaming' }))
    applyFrame(
      frame({
        id: 'R1',
        parent_id: 'T1',
        role: 'assistant',
        type: 'reasoning',
        status: 'streaming',
        content: '思考内容',
      }),
    )
    await nextTick()
    expect(w.text()).toContain('思考中')
    // 最小状态帧（后端 `state_frame` 同构）：不带正文，落地是合并
    applyFrame(frame({ id: 'R1', status: 'completed' }))
    await nextTick()
    expect(w.text(), '定稿后不再显示「思考中」').not.toContain('思考中')
    expect(w.text(), '状态帧不得把思考正文抹掉').toContain('思考内容')
  })
})

/**
 * 渲染层回归：**工具调用**的可见性（对照 e2e 实测帧序列）。
 *
 * e2e 抓到的真实帧序（相对时刻，mock `chunkDelayMs: 400`）：
 *
 * ```
 * +168ms  turn        streaming            ← Turn 根
 * +170ms  tool_call   streaming content="" ← 工具调用节点首帧（名字已知，参数尚无）
 * +570ms  delta       {"text":"m           ← 参数窄增量
 * +984ms  delta       ock 回显内容"}       ← 参数窄增量（至此参数完整）
 * +1798ms tool_call   streaming meta.started_at ← **只改 meta 的状态帧**
 * +1838ms tool result completed            ← 结果子节点
 * +1838ms tool_call   completed
 * ```
 *
 * 三条钉住的约定：
 * 1. 工具行**在首帧即出现**（不等参数、不等执行完）——这是"流模式"的直接体现；
 * 2. 运行中参数**在收起态就能看到**（`ToolCall` 的参数在自身 content 里，回落自身
 *    正文作单行摘要）——否则运行中一行空白，看起来像"跑完才显示"；
 * 3. **只改 `meta` 的帧必须进 DOM**（`started_at` ⇒ 「运行中 5s」的秒数）——
 *    节点复用签名若不含 `meta`，"还在跑"与"卡死了"就无从区分。
 */
describe('渲染层回归：工具调用按帧即时可见（对照 e2e 实测帧序）', () => {
  const Panel = defineComponent({
    props: { sessionId: { type: String, required: true } },
    setup(props) {
      const chat = useChatConnection({ sessionId: props.sessionId })
      return () =>
        chat.messageTree.value.map((n) => h(MessageNode, { node: n, depth: 0, key: n.id }))
    },
  })

  beforeEach(() => {
    setActivePinia(createPinia())
    seq = 0
    stopTranscriptStream()
    void startTranscriptStream(storeSink())
  })

  it('工具行在首帧即出现，参数增量在收起态可见（不等执行完）', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame({ id: 'U1', role: 'user', type: 'text', content: '调用工具' }))
    applyFrame(frame(turn('T1')))
    await nextTick()
    expect(w.findAll('.turn-pending').length, '尚无子节点 → 等待骨架').toBe(1)

    // 首帧：名字已知、参数为空
    applyFrame(frame(toolCall('TC1', 'T1', 'mcp__mockserv__echo', '')))
    await nextTick()
    expect(w.findAll('.type-tool_call').length, '工具行必须在首帧就出现').toBe(1)
    expect(w.text()).toContain('mcp__mockserv__echo')
    expect(w.findAll('.turn-pending').length, '有子节点后骨架消失').toBe(0)

    // 参数窄增量：收起态的单行摘要必须跟着长
    applyFrame(append('TC1', '{"text":"m'))
    await nextTick()
    applyFrame(append('TC1', 'ock 回显内容"}'))
    await nextTick()
    expect(
      w.find('.type-tool_call .node-preview').text(),
      '运行中参数必须可见（否则看起来像"跑完才显示"）',
    ).toContain('mock 回显内容')
  })

  it('只改 meta 的状态帧必须进 DOM（运行中秒数）', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame(turn('T1')))
    applyFrame(frame(toolCall('TC1', 'T1', 'read_file', '{"path":"a.rs"}')))
    await nextTick()
    expect(w.find('.tag-elapsed').exists(), '尚无 started_at → 不编一个时长').toBe(false)

    // 后端在"工具开始执行那一刻"单独发一条**只带 meta** 的帧
    applyFrame(
      frame({
        id: 'TC1',
        parent_id: 'T1',
        role: 'assistant',
        type: 'tool_call',
        name: 'read_file',
        status: 'streaming',
        meta: { started_at: Date.now() - 5000 },
      } as ChatMessage),
    )
    await nextTick()
    expect(
      w.find('.tag-elapsed').exists(),
      'meta 不进签名 ⇒ 缓存节点被复用 ⇒ 这个秒数永远不会出现',
    ).toBe(true)
    expect(w.find('.tag-elapsed').text()).toMatch(/^\d+s$/)
  })
})

/**
 * 渲染层回归：**收起态单行摘要在流式中跟末端走**（走马灯），定稿后回到开头（摘要）。
 *
 * 收起态的摘要是**静态一行**：内容取自哪一端，决定了这一行在流式期间会不会动。
 * 取开头则整条流式过程中它**一个字都不变**——用户看不到「还在长」还是「卡住了」，
 * 也永远追不上早已流过去的正文/思考，观感就是"折叠起来一直是开头那句"。
 *
 * 两条约定必须同时成立（缺一即为半成品）：
 * 1. 文本取**末端**（最新）、并把 `liveEdge` 一并交给渲染器；
 * 2. 渲染器据此在**左端**裁剪（`.node-preview.live`）——只改文本不改裁剪方向，
 *    可见区里剩下的是**中间**那一段，最新内容照样被右端省略号吃掉。
 */
describe('渲染层回归：收起态摘要在流式中显示最新内容', () => {
  const Panel = defineComponent({
    props: { sessionId: { type: String, required: true } },
    setup(props) {
      const chat = useChatConnection({ sessionId: props.sessionId })
      return () =>
        chat.messageTree.value.map((n) => h(MessageNode, { node: n, depth: 0, key: n.id }))
    },
  })

  /** 超过摘要上限：两端截断结果必然不同，才有"取哪一端"可言 */
  const OPEN = '开头：先看协议；'
  const CLOSE = '末尾：结论已定。'

  beforeEach(() => {
    setActivePinia(createPinia())
    seq = 0
    stopTranscriptStream()
    void startTranscriptStream(storeSink())
  })

  it('思考（始终单行）流式中跟着末端走，定稿后回到开头', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame(turn('T1')))
    applyFrame(frame(reasoning('R1', 'T1', OPEN + '甲'.repeat(80))))
    await nextTick()

    // 流式增量：摘要必须跟着长，且显示的是**新到的那一段**
    applyFrame(append('R1', CLOSE))
    await nextTick()
    const live = w.find('.type-reasoning .node-preview')
    expect(live.exists(), '思考默认单行，摘要必须存在').toBe(true)
    expect(live.classes(), '流式中必须在左端裁剪（最新内容才留在可见区）').toContain('live')
    expect(live.text(), '流式中必须能看到最新内容').toContain(CLOSE)
    expect(live.text(), '流式中不该还停在开头那句').not.toContain(OPEN)

    // 定稿：内容不再变，摘要回到开头（概述），裁剪方向也随之回到右端
    applyFrame(frame({ id: 'R1', status: 'completed' }))
    await nextTick()
    const settled = w.find('.type-reasoning .node-preview')
    expect(settled.classes(), '定稿后不再左端裁剪').not.toContain('live')
    expect(settled.text(), '定稿后的摘要是概述（开头）').toContain(OPEN)
    expect(settled.text()).not.toContain(CLOSE)
  })

  it('工具行（默认单行）运行中的参数摘要同样跟末端走', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyFrame(frame(turn('T1')))
    applyFrame(frame(toolCall('TC1', 'T1', 'write_file', '{"path":"/a/b.rs","body":"')))
    await nextTick()
    applyFrame(append('TC1', '甲'.repeat(80) + '"}'))
    await nextTick()

    const preview = w.find('.type-tool_call .node-preview')
    expect(preview.classes(), '运行中的工具行同样在左端裁剪').toContain('live')
    expect(preview.text(), '参数已流到本地 ⇒ 收起态就该看到最新一段').toContain('"}')

    // 工具执行结束：内容定稿 → 回到开头（`{"path":...` 才是这次调用的身份）
    applyFrame(frame({ id: 'TC1', status: 'completed' }))
    await nextTick()
    const settled = w.find('.type-tool_call .node-preview')
    expect(settled.classes()).not.toContain('live')
    expect(settled.text()).toContain('"path"')
  })
})

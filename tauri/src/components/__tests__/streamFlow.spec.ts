// @vitest-environment happy-dom
/**
 * 流式端到端复现测试：真实 store → 转写流收敛 → 树构建（复刻
 * useChatConnection.messageTree 的分形逻辑）→ MessageNode 渲染。
 *
 * 目的：复现"长会话流模式下 Reason 块不结束 / 后续内容显示进 Reason / Turn 不显示"
 * 的纯前端可见结果，并作为修复的回归锚。
 *
 * 帧按**协议词汇**驱动（`NodeOp`：`upsert` 完整快照 / `append` 增量），与
 * `MainLayout` 的真实接线同路（`transcriptStream` → store）。
 *
 * 注意 `upsert` 是**整条替换**（协议 S23 无补丁语义）：状态迁移帧同样必须是
 * **完整消息**——只带 `status` 的部分帧会把 `type` / `parent_id` 一并抹掉，
 * 节点随即失去渲染语义（这正是旧「补丁」路径的病根）。
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
  startTranscriptStream,
  stopTranscriptStream,
  type NodeEvent,
  type TranscriptStreamSink,
} from '@/services/transcriptStream'

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

/** 造一条 `upsert`（完整消息快照）——创建与状态迁移用的是同一种帧 */
function upsert(message: ChatMessage): NodeEvent {
  seq += 1
  return { session_id: SID, seq, op: 'upsert', message }
}

function append(messageId: string, delta: string): NodeEvent {
  seq += 1
  return { session_id: SID, seq, op: 'append', message_id: messageId, delta }
}

/** 与 `MainLayout` 的接线逐字一致 */
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
/** 终态帧：完整消息 + 新状态（**不是**只带 status 的部分帧） */
function settled(m: ChatMessage, status: ChatMessage['status'] = 'completed'): ChatMessage {
  return { ...m, status }
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
    const feed = (e: NodeEvent) => applyNodeEvent(e)

    // ── Turn 1：思考 → 文本 → 工具（结果在下一轮前）──
    const t1 = turn('T1')
    const r1 = reasoning('r1', 'T1', '想法')
    const x1 = text('x1', 'T1', '正文')
    const tc1 = toolCall('tc1', 'T1', 'read_file', '{"path":"a.rs"}')
    feed(upsert(t1))
    feed(upsert(r1))
    feed(upsert(settled(r1)))
    feed(upsert(x1))
    feed(upsert(settled(x1)))
    feed(upsert(tc1))

    // ── 工具结果 + Turn1 完成（后端 finalize 顺序：先子节点后根）──
    feed(upsert({ id: 'res1', parent_id: 'tc1', role: 'tool', type: 'text', status: 'completed', content: '"ok"' }))
    feed(upsert(settled(tc1)))
    feed(upsert(settled(t1)))

    // ── Turn 2：纯文本收尾 ──
    const t2 = turn('T2')
    const r2 = reasoning('r2', 'T2', '再想')
    const x2 = text('x2', 'T2', '结论')
    feed(upsert(t2))
    feed(upsert(r2))
    feed(upsert(settled(r2)))
    feed(upsert(x2))
    feed(upsert(settled(x2)))
    feed(upsert(settled(t2)))

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
    applyNodeEvent(upsert(turn('T1')))
    applyNodeEvent(upsert(reasoning('r1', 'T1', '部分思考')))

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

  it('增量帧（append）拼进正文——流式热路径', () => {
    applyNodeEvent(upsert(turn('T1')))
    applyNodeEvent(upsert(text('x1', 'T1', '第一段')))
    applyNodeEvent(append('x1', '，第二段'))

    const store = useSessionsStore()
    const msg = store.getSessionMessages(SID).find((m) => m.id === 'x1')
    expect(msg?.content, '增量必须逐步拼出完整正文').toBe('第一段，第二段')
  })

  it('空 Turn 未收帧（等待 LLM 首 token）：骨架存在且属于当前轮', () => {
    // 上一轮已完成
    const t0 = turn('T0')
    const x0 = text('x0', 'T0', '上轮')
    applyNodeEvent(upsert(t0))
    applyNodeEvent(upsert(x0))
    applyNodeEvent(upsert(settled(x0)))
    applyNodeEvent(upsert(settled(t0)))
    // 新一轮 Turn 已广播、尚无子节点
    applyNodeEvent(upsert(turn('T1')))

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
 * 「append 始终不刷新；正文结束后（根 Turn 状态迁移那一刻）才突然完整」。
 *
 * 根因是**渲染失效口径不完整**：容器的渲染结果全部来自子树（Turn / ToolCall
 * 自身无正文），所以「已有子节点的正文增长」也必须使祖先的节点对象换新；
 * 否则 `:node` 引用不变 ⇒ Vue 判定 props 未变 ⇒ 整棵子树跳过更新。
 * 下面用**真实 composable + 真实渲染**（挂载方式与 `ModelChatPanel` 同构）钉住。
 */
describe('渲染层回归：append 增量必须进 DOM（不只进 store）', () => {
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

  it('父容器（Turn）已有子节点时，子节点的 append 必须重渲染', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyNodeEvent(upsert({ id: 'T1', role: 'assistant', type: 'turn', status: 'streaming' }))
    applyNodeEvent(
      upsert({
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
    applyNodeEvent(append('X1', '，正文继续'))
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
    applyNodeEvent(upsert({ id: 'T1', role: 'assistant', type: 'turn', status: 'streaming' }))
    applyNodeEvent(
      upsert({
        id: 'R1',
        parent_id: 'T1',
        role: 'assistant',
        type: 'reasoning',
        status: 'streaming',
        content: '第一段思考',
      }),
    )
    await nextTick()
    applyNodeEvent(append('R1', '，第二段思考'))
    await nextTick()
    // 分派类名由 registry 给出 ⇒ 钉住「思考是独立节点，没有被并进正文」
    expect(w.find('.type-reasoning').exists(), '思考必须是独立节点').toBe(true)
    // 折叠态的一行摘要由节点 content 渲染，它必须同步增长
    expect(w.text(), '思考的增量必须进 DOM').toContain('第二段思考')
  })

  it('子节点终态（状态迁移）必须立刻反映，不等祖先的结构变化', async () => {
    const w = mount(Panel, { props: { sessionId: SID } })
    applyNodeEvent(upsert({ id: 'T1', role: 'assistant', type: 'turn', status: 'streaming' }))
    applyNodeEvent(
      upsert({
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
    applyNodeEvent(
      upsert({
        id: 'R1',
        parent_id: 'T1',
        role: 'assistant',
        type: 'reasoning',
        status: 'completed',
        content: '思考内容',
      }),
    )
    await nextTick()
    expect(w.text(), '定稿后不再显示「思考中」').not.toContain('思考中')
  })
})

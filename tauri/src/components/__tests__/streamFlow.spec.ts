// @vitest-environment happy-dom
/**
 * 流式端到端复现测试：真实 store → patchMessage 合并 → 树构建（复刻
 * useChatConnection.messageTree 的分形逻辑）→ MessageNode 渲染。
 *
 * 目的：复现"长会话流模式下 Reason 块不结束 / 后续内容显示进 Reason / Turn 不显示"
 * 的纯前端可见结果，并作为修复的回归锚。
 */
import { describe, expect, it, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import MessageNode from '../MessageNode.vue'
import { useSessionsStore } from '@/stores/sessions'
import type { ChatMessage } from '@/services/model'

/** 与后端 emit_streaming_start / dispatch_protocol_event / finalize 完全一致的帧构造器 */
function turnStart(id: string): Partial<ChatMessage> {
  return { id, parent_id: undefined, role: 'assistant', type: 'turn', status: 'streaming' }
}
function reasoningDelta(id: string, turn: string, delta: string): Partial<ChatMessage> {
  return { id, parent_id: turn, role: 'assistant', type: 'reasoning', status: 'streaming', content: delta }
}
function textDelta(id: string, turn: string, delta: string): Partial<ChatMessage> {
  return { id, parent_id: turn, role: 'assistant', type: 'text', status: 'streaming', content: delta }
}
function toolCall(id: string, turn: string, name: string, args: string): Partial<ChatMessage> {
  return { id, parent_id: turn, role: 'assistant', type: 'tool_call', name, status: 'streaming', content: args }
}
function status(id: string, st: ChatMessage['status']): Partial<ChatMessage> {
  return { id, status: st }
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

import { h } from 'vue'

describe('流式端到端：Turn/Reason 在多轮工具场景的可见性', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('多轮工具：每轮 Turn 最终 Completed，无残留「正在思考」骨架', () => {
    const store = useSessionsStore()
    const sid = 's1'
    const feed = (p: Partial<ChatMessage>) => store.patchMessage(sid, p as ChatMessage)

    // ── Turn 1：思考 → 文本 → 工具（结果在下一轮前）──
    feed(turnStart('T1'))
    feed(reasoningDelta('r1', 'T1', '想法'))
    feed(status('r1', 'completed'))
    feed(textDelta('x1', 'T1', '正文'))
    feed(status('x1', 'completed'))
    feed(toolCall('tc1', 'T1', 'read_file', '{"path":"a.rs"}'))

    // ── 工具结果 + Turn1 完成（后端 finalize 顺序：先子节点后根）──
    feed({ id: 'res1', parent_id: 'tc1', role: 'tool', type: 'text', status: 'completed', content: '"ok"' })
    feed(status('tc1', 'completed'))
    feed(status('T1', 'completed'))

    // ── Turn 2：纯文本收尾 ──
    feed(turnStart('T2'))
    feed(reasoningDelta('r2', 'T2', '再想'))
    feed(status('r2', 'completed'))
    feed(textDelta('x2', 'T2', '结论'))
    feed(status('x2', 'completed'))
    feed(status('T2', 'completed'))

    const msgs = store.getSessionMessages(sid)
    const tree = buildTree(msgs)
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
    const store = useSessionsStore()
    const sid = 's2'
    store.patchMessage(sid, turnStart('T1') as ChatMessage)
    store.patchMessage(sid, reasoningDelta('r1', 'T1', '部分思考') as ChatMessage)

    const tree = buildTree(store.getSessionMessages(sid))
    const w = mountTree(tree)
    // Turn 已有子节点 → 不显示等待骨架
    expect(w.findAll('.turn-pending').length).toBe(0)
    // 思考行头存在且是"思考中…"态
    const head = w.find('.node-head')
    expect(head.exists()).toBe(true)
    expect(head.text()).toContain('思考中')
  })

  it('空 Turn 未收帧（等待 LLM 首 token）：骨架存在且属于当前轮', () => {
    const store = useSessionsStore()
    const sid = 's3'
    // 上一轮已完成
    store.patchMessage(sid, turnStart('T0') as ChatMessage)
    store.patchMessage(sid, textDelta('x0', 'T0', '上轮') as ChatMessage)
    store.patchMessage(sid, status('x0', 'completed') as ChatMessage)
    store.patchMessage(sid, status('T0', 'completed') as ChatMessage)
    // 新一轮 Turn 已广播、尚无子节点
    store.patchMessage(sid, turnStart('T1') as ChatMessage)

    const tree = buildTree(store.getSessionMessages(sid))
    const w = mountTree(tree)
    expect(w.findAll('.turn-pending').length).toBe(1)
  })
})

// @vitest-environment happy-dom
/**
 * MessageNode 组件级测试 —— 会话流响应体验重设计（Turn 响应分组 + 两级重试分派）
 *
 * 覆盖设计要点（见 MessageNode.vue 文件头注释）：
 * 1. 根级 Turn 三形态：等待骨架 / 透明分组（子节点直排）/ 组级错误条
 * 2. 思考/工具默认单行（流式中不展开），例外：工具内含待审批子节点
 * 3. 工具三段式：请求 / 过程（子会话流，无则隐藏）/ 结果
 * 4. 重试两级分派的发射语义：Turn 失败 → retry(turn id)；工具失败 → retry(tool id)
 */
import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import MessageNode from '../MessageNode.vue'
import type { ChatMessage } from '@/services/model'

/** 构造合法 ChatMessage（缺省：assistant/text/completed） */
function msg(p: Partial<ChatMessage> & { id: string }): ChatMessage {
  return { role: 'assistant', type: 'text', content: '', status: 'completed', ...p }
}

function mountNode(node: ChatMessage, extraProps: Record<string, unknown> = {}) {
  return mount(MessageNode, {
    props: { node, ...extraProps },
    global: { provide: { resume: vi.fn() } },
  })
}

describe('MessageNode：根级 Turn 响应分组（三形态）', () => {
  it('形态①：Turn 无子节点且流式中 → 等待骨架，无节点头部', () => {
    const w = mountNode(msg({ id: 't1', type: 'turn', status: 'streaming', children: [] }))
    expect(w.find('.turn-pending').exists()).toBe(true)
    expect(w.text()).toContain('正在思考')
    expect(w.find('.node-head').exists()).toBe(false)
  })

  it('形态②：子节点出现 → 容器隐藏，思考带行头，正文直出展开', () => {
    const w = mountNode(
      msg({
        id: 't1',
        type: 'turn',
        status: 'streaming',
        children: [
          msg({ id: 'r1', type: 'reasoning', status: 'completed', content: '想了一下', parent_id: 't1' }),
          msg({ id: 'x1', type: 'text', status: 'streaming', content: '你好世界', parent_id: 't1' }),
        ],
      }),
    )
    expect(w.find('.turn-pending').exists()).toBe(false)
    // Turn 自身无头部：唯一的头部是思考行（正文内联，无头部）
    const heads = w.findAll('.node-head')
    expect(heads.length).toBe(1)
    expect(heads[0].text()).toContain('思考')
    expect(heads[0].classes()).not.toContain('thinking') // 已完成不再脉动
    // 正文 markdown 直出且可见（首个 markdown 体是思考的，正文在其中之一）
    const bodies = w.findAll('.markdown-body')
    expect(bodies.some((b) => b.text().includes('你好世界'))).toBe(true)
  })

  it('形态③：Turn 失败 → 组级错误条 + 重试（发射 Turn id，供 retry_turn）', async () => {
    const w = mountNode(
      msg({
        id: 't1',
        type: 'turn',
        status: 'failed',
        error: '请求失败',
        children: [msg({ id: 'x1', type: 'text', status: 'failed', content: '', error: '请求失败', parent_id: 't1' })],
      }),
    )
    // 组级错误条存在且携带错误文本；叶子级错误框被抑制（不重复报错）
    const boxes = w.findAll('.error-box')
    expect(boxes.length).toBe(1)
    expect(boxes[0].text()).toContain('请求失败')
    await boxes[0].find('button.retry').trigger('click')
    expect(w.emitted('retry')?.[0]).toEqual(['t1'])
  })

  it('形态③变体：Turn 失败且无子节点 → 错误条而非等待骨架（请求即败场景）', () => {
    const w = mountNode(msg({ id: 't1', type: 'turn', status: 'failed', error: 'bad', children: [] }))
    expect(w.find('.turn-pending').exists()).toBe(false)
    expect(w.find('.error-box').exists()).toBe(true)
  })
})

describe('MessageNode：思考节点始终单行', () => {
  it('流式中：行头带 thinking 动效类 + 「思考中…」，内容折叠', () => {
    const w = mountNode(msg({ id: 'r1', type: 'reasoning', status: 'streaming', content: '生成中的思考' }), {
      parentType: 'turn',
    })
    const head = w.find('.node-head')
    expect(head.exists()).toBe(true)
    expect(head.classes()).toContain('thinking')
    expect(head.text()).toContain('思考中')
    const body = w.find('.node-body')
    expect(body.exists()).toBe(true)
    expect((body.element as HTMLElement).style.display).toBe('none')
  })

  it('完成后：标题「思考」，仍保持折叠', () => {
    const w = mountNode(msg({ id: 'r1', type: 'reasoning', status: 'completed', content: '已完成的思考' }))
    const head = w.find('.node-head')
    expect(head.text()).toContain('思考')
    expect(head.classes()).not.toContain('thinking')
    expect((w.find('.node-body').element as HTMLElement).style.display).toBe('none')
  })
})

describe('MessageNode：工具调用（单行 + 三段式 + 就地重试）', () => {
  it('默认单行折叠：流式中显示「调用中…」标签，卡片体隐藏', () => {
    const w = mountNode(
      msg({ id: 'tc1', type: 'tool_call', status: 'streaming', name: 'shell', content: '{"cmd":"ls"}', children: [] }),
    )
    expect(w.find('.node-head').exists()).toBe(true)
    expect(w.text()).toContain('调用中')
    expect((w.find('.node-body').element as HTMLElement).style.display).toBe('none')
  })

  it('例外：内含待审批 user_prompt → 自动展开（审批入口可见）', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'tool_call',
        status: 'streaming',
        name: 'shell',
        content: '{}',
        children: [
          msg({
            id: 'up1',
            type: 'user_prompt',
            status: 'waiting_user_action',
            meta: { prompt: { kind: 'confirm', tool_name: 'shell', risk_level: 'high', description: '删除文件' } },
            parent_id: 'tc1',
          }),
        ],
      }),
    )
    expect((w.find('.node-body').element as HTMLElement).style.display).not.toBe('none')
    expect(w.text()).toContain('批准执行')
  })

  it('三段式：请求 / 过程（子会话 Turn）/ 结果', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'tool_call',
        status: 'completed',
        name: 'agent',
        content: '{}',
        children: [
          msg({
            id: 'sub1',
            type: 'turn',
            role: 'tool',
            status: 'completed',
            name: 'sub-agent',
            parent_id: 'tc1',
            children: [msg({ id: 'st1', type: 'text', status: 'completed', content: '子流正文', parent_id: 'sub1' })],
          }),
          msg({ id: 'res1', type: 'text', role: 'tool', status: 'completed', content: '"ok"', parent_id: 'tc1' }),
        ],
      }),
    )
    const labels = w.findAll('.ts-label').map((l) => l.text())
    expect(labels).toEqual(['请求', '过程', '结果'])
    // 子会话 Turn 以折叠节点形态嵌在「过程」段中
    expect(w.text()).toContain('sub-agent')
  })

  it('无子会话的工具：「过程」段整段隐藏（仅 请求 + 结果）', () => {
    const w = mountNode(
      msg({
        id: 'tc2',
        type: 'tool_call',
        status: 'completed',
        content: '{}',
        children: [msg({ id: 'res2', type: 'text', role: 'tool', status: 'completed', content: '"ok"', parent_id: 'tc2' })],
      }),
    )
    expect(w.findAll('.tool-section').length).toBe(2)
    expect(w.text()).not.toContain('过程')
  })

  it('工具失败 → 就地重试（发射 ToolCall id，供单工具 retry），叶子级不出重试按钮', async () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'tool_call',
        status: 'failed',
        error: '工具执行失败',
        name: 'shell',
        content: '{}',
        children: [msg({ id: 'res1', type: 'text', role: 'tool', status: 'failed', content: '', error: 'boom', parent_id: 'tc1' })],
      }),
    )
    const btn = w.find('.error-box button.retry')
    expect(btn.exists()).toBe(true)
    await btn.trigger('click')
    expect(w.emitted('retry')?.[0]).toEqual(['tc1'])
  })

  it('可补充参数的失败（failure_kind=error）→ 自动展开出补充参数表单', () => {
    const w = mountNode(
      msg({
        id: 'tc1',
        type: 'tool_call',
        status: 'failed',
        error: '参数缺失',
        meta: { failure_kind: 'error' },
        content: '{}',
        children: [],
      }),
    )
    expect(w.find('.supply').exists()).toBe(true)
  })
})

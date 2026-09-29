/**
 * 对话线分界 — 纯函数单测（node 环境）
 *
 * ## 为什么这份测试必须与 Rust 侧逐条对应
 *
 * `conversation_line.ts` 是 `symbio/src/plugins/session/context/conversation_view.rs`
 * 的**镜像**，而 `protocol-mirror-audit` 只守常量 / 枚举 / 字段名，**不守谓词**。
 * 于是两边各写一份穷举测试是这份镜像的唯一防线：
 *
 * | 本文件 | Rust 侧 |
 * |---|---|
 * | `projection_keeps_only_the_conversation_line` | 同名 |
 * | `turn 的子正文在对话线上` | `turn_child_answer_text_is_on_the_conversation_line` |
 * | `逐类排除` | `tool_and_turn_and_reasoning_are_excluded` |
 * | `工具结果正文不泄漏` | `tool_result_payload_never_leaks_into_the_projection` |
 * | `workRootsOf 只过滤根级` | 前端独有（工作面板的数据源） |
 *
 * 改规则时**必须同时改两处测试**——这是镜像的代价，写在文件头而不是留给下一个人发现。
 */

import { describe, expect, it } from 'vitest'
import {
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_SYSTEM,
  CHAT_ROLE_TOOL,
  CHAT_ROLE_USER,
  MESSAGE_TYPE_COMPRESSION,
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TOOL_CALL,
  MESSAGE_TYPE_TURN,
  MESSAGE_TYPE_USER_PROMPT,
  type ChatMessage,
  type ChatMessageType,
  type ChatRole,
} from '../chat_message'
import { conversationNodesOf, isConversationNode, workRootsOf } from '../conversation_line'

/** 造一条最小节点：只填分界判据要读的三个字段 */
function msg(
  id: string,
  role: ChatRole | undefined,
  type: ChatMessageType | undefined,
  parentId?: string,
  text = '',
): ChatMessage {
  return {
    id,
    parent_id: parentId,
    role,
    type,
    content: text,
    status: 'completed',
  }
}

/**
 * 一份真实形状的转写：用户 → Turn（含推理 / 正文 / 工具调用 → 工具结果）→ 根级首响。
 * 与 Rust 侧 `conversation_view.test.rs::transcript()` **同形**。
 */
function transcript(): ChatMessage[] {
  return [
    msg('u1', CHAT_ROLE_USER, undefined, undefined, '读一下 README'),
    msg('turn1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_TURN),
    msg('r1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_REASONING, 'turn1', '先看文件'),
    msg('t1', CHAT_ROLE_ASSISTANT, 'text', 'turn1', '文件已读完'),
    msg('tc1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_TOOL_CALL, 'turn1', '{"path":"README.md"}'),
    msg('tr1', CHAT_ROLE_TOOL, 'text', 'tc1', '# Symbio\n很长的文件正文……'),
    msg('f1', CHAT_ROLE_ASSISTANT, 'text', undefined, '好的，我去看一下。'),
    msg('c1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_COMPRESSION, undefined, '正在压缩上下文…'),
    msg('p1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_USER_PROMPT, undefined, '允许执行吗？'),
    msg('s1', CHAT_ROLE_SYSTEM, 'text', undefined, '系统注入的框架文本'),
  ]
}

describe('isConversationNode：判据只有两条', () => {
  it('用户文本 / 助手文本判真', () => {
    expect(isConversationNode(msg('a', CHAT_ROLE_USER, 'text'))).toBe(true)
    expect(isConversationNode(msg('b', CHAT_ROLE_ASSISTANT, 'text'))).toBe(true)
  })

  it('type 缺省视为文本（后端流式帧可能不带 type）', () => {
    expect(isConversationNode(msg('a', CHAT_ROLE_USER, undefined))).toBe(true)
    expect(isConversationNode(msg('b', CHAT_ROLE_ASSISTANT, undefined))).toBe(true)
  })

  it('role 缺省不算对话线（Rust 侧 None 落空）', () => {
    expect(isConversationNode(msg('a', undefined, 'text'))).toBe(false)
  })

  it('五类非文本节点一律判假', () => {
    for (const t of [
      MESSAGE_TYPE_REASONING,
      MESSAGE_TYPE_TOOL_CALL,
      MESSAGE_TYPE_TURN,
      MESSAGE_TYPE_USER_PROMPT,
      MESSAGE_TYPE_COMPRESSION,
    ] as const) {
      expect(isConversationNode(msg('x', CHAT_ROLE_ASSISTANT, t))).toBe(false)
    }
  })

  it('role = tool / system 判假 —— 即便类型是文本', () => {
    expect(isConversationNode(msg('a', CHAT_ROLE_TOOL, 'text'))).toBe(false)
    expect(isConversationNode(msg('b', CHAT_ROLE_SYSTEM, 'text'))).toBe(false)
  })
})

describe('conversationNodesOf：投影（扁平、保序）', () => {
  it('只留下用户消息与助手文本，顺序即入参顺序', () => {
    expect(conversationNodesOf(transcript()).map((m) => m.id)).toEqual(['u1', 't1', 'f1'])
  })

  it('turn 的子正文在对话线上 —— parent_id 不参与判定', () => {
    const answer = conversationNodesOf(transcript()).find((m) => m.id === 't1')
    expect(answer).toBeDefined()
    expect(answer?.parent_id).toBe('turn1')
  })

  it('工具结果正文一个字节都不该出现', () => {
    const joined = conversationNodesOf(transcript())
      .map((m) => (typeof m.content === 'string' ? m.content : ''))
      .join('')
    expect(joined).not.toContain('很长的文件正文')
  })

  it('空输入 ⇒ 空投影（不 panic、不补占位）', () => {
    expect(conversationNodesOf([])).toEqual([])
  })

  it('纯工作转写 ⇒ 空投影（对话线可以为空，这不是错误）', () => {
    const onlyWork = [
      msg('turn1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_TURN),
      msg('tc1', CHAT_ROLE_ASSISTANT, MESSAGE_TYPE_TOOL_CALL, 'turn1', '{}'),
    ]
    expect(conversationNodesOf(onlyWork)).toEqual([])
  })

  it('产出是新数组，不改动入参', () => {
    const source = transcript()
    const before = source.length
    const out = conversationNodesOf(source)
    expect(out).not.toBe(source)
    expect(source.length).toBe(before)
  })
})

describe('workRootsOf：工作面板的数据源', () => {
  /** 调用方给的是 `messageTree`（根级），故这里也先取根级 */
  const rootsOf = (all: ChatMessage[]) => all.filter((m) => m.parent_id === undefined)

  it('根级里不属于对话线的那些', () => {
    expect(workRootsOf(rootsOf(transcript())).map((m) => m.id)).toEqual([
      'turn1',
      'c1',
      'p1',
      's1',
    ])
  })

  it('它不是树遍历器：入参给什么就过滤什么，子节点照样留下', () => {
    // 有意如此 —— 剪子节点要重建整棵树并维护父子反向引用，为展示重复付这个代价不值得。
    // 这条同时防"顺手把它改成递归"：那会让工作面板丢掉 turn 里的工具调用。
    expect(workRootsOf(transcript()).map((m) => m.id)).toEqual([
      'turn1',
      'r1',
      'tc1',
      'tr1',
      'c1',
      'p1',
      's1',
    ])
  })

  it('与对话线互为补集（同一份根级上）', () => {
    const roots = rootsOf(transcript())
    const conv = new Set(conversationNodesOf(roots).map((m) => m.id))
    const work = new Set(workRootsOf(roots).map((m) => m.id))
    for (const r of roots) expect(conv.has(r.id) !== work.has(r.id)).toBe(true)
    expect(conv.size + work.size).toBe(roots.length)
  })

  it('空输入 ⇒ 空（不 panic）', () => {
    expect(workRootsOf([])).toEqual([])
  })
})

/**
 * 会话转写规则 —— 纯函数单测（node 环境）
 *
 * 这几条规则原先只能通过「造一个 Pinia store + 喂一串 patch」间接验证，
 * 而它们恰恰是**出错代价最高**的部分：增量落点判错 → 内容重复拼接或倒退；
 * 水合判错 → 在途节点消失（lost update）或幽灵节点复活；
 * 截断判错 → 列表尾部残留后端已不存在的节点，且没有任何机制会纠正它。
 */

import { describe, expect, it } from 'vitest'
import {
  PREVIEW_MAX,
  appendContent,
  hydrateTranscript,
  isInProgressMessage,
  isRootTurn,
  previewOf,
  sortTranscript,
  truncateIdsFrom,
} from '../sessionTranscript'
import type { ChatMessage } from '@/schemas/chat_message'

function msg(p: Partial<ChatMessage> & { id: string }): ChatMessage {
  return { role: 'assistant', type: 'text', status: 'completed', ...p }
}

describe('sortTranscript：seq 是唯一权威顺序锚点', () => {
  it('按 seq 升序；不修改入参', () => {
    const input = [msg({ id: 'c', seq: 3 }), msg({ id: 'a', seq: 1 }), msg({ id: 'b', seq: 2 })]
    const out = sortTranscript(input)
    expect(out.map((m) => m.id)).toEqual(['a', 'b', 'c'])
    expect(input.map((m) => m.id)).toEqual(['c', 'a', 'b'])
  })

  it('缺失 seq 时回退 timestamp（旧数据），缺失两者按 0', () => {
    const out = sortTranscript([
      msg({ id: 'x', timestamp: 20 }),
      msg({ id: 'y' }),
      msg({ id: 'z', seq: 1 }),
    ])
    expect(out.map((m) => m.id)).toEqual(['y', 'z', 'x'])
  })
})

describe('previewOf：缩略卡预览', () => {
  it('只取 assistant 的文本；用户消息 / 工具返回不参与', () => {
    expect(previewOf({ role: 'assistant', content: '正文' })).toBe('正文')
    expect(previewOf({ role: 'user', content: '用户说的' })).toBeNull()
    expect(previewOf({ role: 'tool', content: '结果' })).toBeNull()
  })

  it('多模态内容只拼 text 分片（图片段不产生预览）', () => {
    expect(
      previewOf({
        role: 'assistant',
        content: [
          { type: 'text', text: '看这张' },
          { type: 'image_url', image_url: { url: 'data:x' } },
        ],
      }),
    ).toBe('看这张')
    // 只有图片、没有文本 → 无预览（不能把空串当成"预览为空"写进去）
    expect(
      previewOf({ role: 'assistant', content: [{ type: 'image_url', image_url: { url: 'x' } }] }),
    ).toBeNull()
  })

  it('超长截断并加省略号；空内容返回 null', () => {
    const long = 'x'.repeat(PREVIEW_MAX + 20)
    const out = previewOf({ role: 'assistant', content: long })
    expect(out).not.toBeNull()
    expect(out!.length).toBe(PREVIEW_MAX + 1)
    expect(out!.endsWith('…')).toBe(true)
    expect(previewOf({ role: 'assistant', content: '' })).toBeNull()
    expect(previewOf({ role: 'assistant' })).toBeNull()
  })
})

describe('appendContent：帧携带 `delta` 时的唯一内容落点', () => {
  it('把增量拼到正文尾部', () => {
    const out = appendContent(msg({ id: 'x', content: '你好' }), '世界')
    expect(out.content).toBe('你好世界')
  })

  it('工具**参数**帧同样追加（参数是窄增量，不是全量重发）', () => {
    const out = appendContent(msg({ id: 'tc', type: 'tool_call', content: '{"a' }), '":1}')
    expect(out.content).toBe('{"a":1}')
  })

  it('role=tool 的**工具响应**同样追加（响应被覆盖成空的回归）', () => {
    // 回归：`mergeMessagePatch` 曾把 `role=tool` 判为"流式帧全量重发"因而整条替换。
    // 后端透传子会话的 `delta`（逐片响应），前端于是只剩最后一片——
    // 表现为"工具卡片有请求、响应是空的"。语义只由帧字段给出，不看角色。
    const out = appendContent(msg({ id: 'r', role: 'tool', content: '共有 ' }), '10 个文件')
    expect(out.content).toBe('共有 10 个文件')
  })

  it('多模态正文经契约层取值（不静默丢成空串）', () => {
    const out = appendContent(msg({ id: 'x', content: [{ type: 'text', text: '看这张' }] }), '图')
    expect(out.content).toBe('看这张图')
  })

  it('不改动入参（返回新对象）', () => {
    const before = msg({ id: 'x', content: 'a' })
    appendContent(before, 'b')
    expect(before.content).toBe('a')
  })
})

describe('hydrateTranscript：快照即权威，整表装载', () => {
  it('快照里的节点整条替换（含状态与 seq）', () => {
    const { map } = hydrateTranscript(
      [msg({ id: 'a', content: '历史', seq: 5, status: 'completed' })],
      { a: msg({ id: 'a', content: '旧', status: 'streaming' }) },
    )
    expect(map.a.content).toBe('历史')
    expect(map.a.status).toBe('completed')
    expect(map.a.seq).toBe(5)
  })

  it('本地副本不参与装载（权威在存储；在途节点由增量通道继续收敛）', () => {
    const { map } = hydrateTranscript(
      [msg({ id: 'a', seq: 100 })],
      { b: msg({ id: 'b', status: 'streaming', content: '在途' }) },
    )
    expect(map.b).toBeUndefined()
    expect(map.a.seq).toBe(100)
  })

  it('本地终态且不在快照里的节点被丢弃（幽灵节点不复活）', () => {
    const { map } = hydrateTranscript([], {
      ghost: msg({ id: 'ghost', status: 'completed', content: '已被删除' }),
      pending: msg({ id: 'pending', status: 'streaming', content: '在途' }),
    })
    expect(map.ghost).toBeUndefined()
    expect(map.pending).toBeUndefined()
  })

  it('缺 seq 的旧数据按数组顺序排在已有 seq 之后', () => {
    const { map } = hydrateTranscript(
      [msg({ id: 'a', seq: 7 }), msg({ id: 'b' }), msg({ id: 'c' })],
      {},
    )
    expect(map.b.seq).toBe(8)
    expect(map.c.seq).toBe(9)
  })

  it('还原「等待审批」角标（缩略卡在重开会话后仍正确）', () => {
    expect(hydrateTranscript([msg({ id: 'a', status: 'waiting_user_action' })], {}).waitingApproval).toBe(
      true,
    )
    expect(hydrateTranscript([msg({ id: 'a', status: 'completed' })], {}).waitingApproval).toBe(false)
  })

  it('忽略没有 id 的节点（不造无名条目）', () => {
    const { map } = hydrateTranscript([msg({ id: '' }), msg({ id: 'ok' })], {})
    expect(Object.keys(map)).toEqual(['ok'])
  })

  it('空快照 + 空本地 → 空结果且游标为 0', () => {
    const { map, lastSeq, waitingApproval } = hydrateTranscript([], {})
    expect(map).toEqual({})
    expect(lastSeq).toBe(0)
    expect(waitingApproval).toBe(false)
  })
})

describe('truncateIdsFrom：「目标 + 其后全部」', () => {
  const ordered = [msg({ id: 'a', seq: 1 }), msg({ id: 'b', seq: 2 }), msg({ id: 'c', seq: 3 })]

  it('从锚点切到末尾（与后端 drain(i..) 同源）', () => {
    expect(truncateIdsFrom(ordered, 'b')).toEqual(['b', 'c'])
    expect(truncateIdsFrom(ordered, 'a')).toEqual(['a', 'b', 'c'])
    expect(truncateIdsFrom(ordered, 'c')).toEqual(['c'])
  })

  it('锚点不在列表里 → 空数组（绝不拿不存在的锚点截断整个列表）', () => {
    expect(truncateIdsFrom(ordered, 'zzz')).toEqual([])
    expect(truncateIdsFrom(ordered, '')).toEqual([])
    expect(truncateIdsFrom([], 'a')).toEqual([])
  })
})

describe('在途节点判据', () => {
  it('根级 Turn = type=turn 且无父节点（唯一允许挂 error 的节点）', () => {
    expect(isRootTurn({ type: 'turn' })).toBe(true)
    expect(isRootTurn({ type: 'turn', parent_id: 'p' })).toBe(false)
    expect(isRootTurn({ type: 'text' })).toBe(false)
  })

  it('进行中 = streaming / waiting_user_action（其余已是终态）', () => {
    expect(isInProgressMessage({ status: 'streaming' })).toBe(true)
    expect(isInProgressMessage({ status: 'waiting_user_action' })).toBe(true)
    expect(isInProgressMessage({ status: 'completed' })).toBe(false)
    expect(isInProgressMessage({ status: 'failed' })).toBe(false)
    expect(isInProgressMessage({ status: 'aborted' })).toBe(false)
    expect(isInProgressMessage({})).toBe(false)
  })
})

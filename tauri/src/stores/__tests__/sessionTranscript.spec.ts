/**
 * 会话转写规则 —— 纯函数单测（node 环境）
 *
 * 这几条规则原先只能通过「造一个 Pinia store + 喂一串 patch」间接验证，
 * 而它们恰恰是**出错代价最高**的部分：合并判错 → 内容重复拼接或倒退；
 * 水合判错 → 在途节点消失（lost update）或幽灵节点复活；
 * 截断判错 → 列表尾部残留后端已不存在的节点，且没有任何机制会纠正它。
 */

import { describe, expect, it } from 'vitest'
import {
  PREVIEW_MAX,
  hydrateTranscript,
  isInProgressMessage,
  isRootTurn,
  mergeMessagePatch,
  previewOf,
  sortTranscript,
  stuckFailurePlanOf,
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

describe('mergeMessagePatch：流式合并语义', () => {
  it('正文 / 思考的字符串内容**追加**（增量 token）', () => {
    const merged = mergeMessagePatch(msg({ id: 'x', content: '你好' }), { id: 'x', content: '世界' })
    expect(merged.content).toBe('你好世界')
  })

  it('tool_call 与 role=tool 的内容**整条替换**（流式帧是全量重发）', () => {
    const tc = mergeMessagePatch(
      msg({ id: 'tc', type: 'tool_call', content: '{"a"' }),
      { id: 'tc', content: '{"a":1}' },
    )
    expect(tc.content).toBe('{"a":1}')

    const tool = mergeMessagePatch(
      msg({ id: 'r', role: 'tool', content: '{"partial"' }),
      { id: 'r', content: '{"full":true}' },
    )
    expect(tool.content).toBe('{"full":true}')
  })

  it('补丁不带 content 键时**不清空**已有内容（终态状态补发场景）', () => {
    const merged = mergeMessagePatch(msg({ id: 'x', content: '参数' }), {
      id: 'x',
      status: 'completed',
    })
    expect(merged.content).toBe('参数')
    expect(merged.status).toBe('completed')
  })

  it('显式传 `content: undefined` 会清空——这是对象展开的语义，调用方别这么写', () => {
    // `{ ...existing, ...patch }` 对**存在但为 undefined** 的键会覆盖成 undefined，
    // 随后的 `patch.content != null` 判定救不回来。这不是本函数的额外规则，
    // 而是 JS 展开语义；把边界钉在这里，免得下次有人以为"传 undefined 等于不传"。
    // 正确做法：构造补丁时**省略**该键（如上一条用例）。
    const cleared = mergeMessagePatch(msg({ id: 'x', content: '参数' }), {
      id: 'x',
      content: undefined,
    })
    expect(cleared.content).toBeUndefined()
  })

  it('meta 浅合并：只覆盖补丁带来的键，保留其余', () => {
    const merged = mergeMessagePatch(
      msg({ id: 'x', meta: { a: 1, b: 2 } }),
      { id: 'x', meta: { b: 9, c: 3 } },
    )
    expect(merged.meta).toEqual({ a: 1, b: 9, c: 3 })
  })
})

describe('hydrateTranscript：快照与本地在途节点的合并', () => {
  it('快照里的节点整条替换（含状态与 seq）', () => {
    const { map } = hydrateTranscript(
      [msg({ id: 'a', content: '历史', seq: 5, status: 'completed' })],
      { a: msg({ id: 'a', content: '旧', status: 'streaming' }) },
    )
    expect(map.a.content).toBe('历史')
    expect(map.a.status).toBe('completed')
    expect(map.a.seq).toBe(5)
  })

  it('本地在途节点保留并**重新分配 seq**（本地游标可能小于快照最大 seq）', () => {
    const { map, lastSeq } = hydrateTranscript(
      [msg({ id: 'a', seq: 100 })],
      { b: msg({ id: 'b', status: 'streaming', content: '在途' }) },
    )
    expect(map.b).toBeDefined()
    expect(map.b.content).toBe('在途')
    // 关键：不能沿用本地的小游标（否则在途节点会排到历史之前）
    expect(map.b.seq).toBeGreaterThan(100)
    expect(lastSeq).toBe(map.b.seq)
  })

  it('本地终态且不在快照里的节点被丢弃（幽灵节点不复活）', () => {
    const { map } = hydrateTranscript([], {
      ghost: msg({ id: 'ghost', status: 'completed', content: '已被删除' }),
      pending: msg({ id: 'pending', status: 'streaming', content: '在途' }),
    })
    expect(map.ghost).toBeUndefined()
    expect(map.pending).toBeDefined()
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

describe('看门狗判据', () => {
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

describe('看门狗定稿计划（stuckFailurePlanOf）', () => {
  it('错误只挂在根级 Turn 上；半截子节点定稿为 completed 且不挂 error', () => {
    const plan = stuckFailurePlanOf(
      [
        msg({ id: 'root', type: 'turn', status: 'streaming' }),
        msg({ id: 'text', status: 'streaming' }),
        msg({ id: 'tool', type: 'tool_call', status: 'streaming' }),
      ],
      '连接中断',
    )

    expect(plan.failed.map((m) => m.id)).toEqual(['root'])
    expect(plan.failed[0].error).toBe('连接中断')
    expect(plan.completed.map((m) => m.id)).toEqual(['text', 'tool'])
    // 每条子节点都不挂 error —— 否则同一条错误会刷到每条半截消息上（429 刷屏的根因）
    expect(plan.completed.every((m) => !m.error)).toBe(true)
    expect(plan.rootTurnFailed).toBe(true)
  })

  it('子节点里的 Turn（有父节点）不算根，错误不会落到它身上', () => {
    const plan = stuckFailurePlanOf(
      [msg({ id: 'sub', type: 'turn', parent_id: 'root', status: 'streaming' })],
      '连接中断',
    )
    expect(plan.failed).toHaveLength(0)
    expect(plan.completed.map((m) => m.id)).toEqual(['sub'])
    // 没有根级 Turn ⇒ 调用方降级为「会话级错误」
    expect(plan.rootTurnFailed).toBe(false)
  })

  it('已终态的消息不进计划（只收尾仍在进行中的）', () => {
    const plan = stuckFailurePlanOf(
      [msg({ id: 'done', status: 'completed' }), msg({ id: 'old', status: 'failed' })],
      '连接中断',
    )
    expect(plan.empty).toBe(true)
    expect(plan.failed).toHaveLength(0)
    expect(plan.completed).toHaveLength(0)
  })
})

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
  dropUnselectedBranches,
  hydrateTranscript,
  isInProgressMessage,
  previewOf,
  sortTranscript,
  truncateIdsFrom,
} from '../sessionTranscript'
import {
  NODE_KIND_BODY,
  NODE_KIND_SKELETON,
  type ChatNode,
} from '@/schemas/chat_node'

const AS_PERSON = { kind: 'person', id: 'person/local' } as const
const AS_AGENT = { kind: 'agent', id: 'session/main' } as const
const AS_TOOL = { kind: 'tool', id: 'vdfs/write' } as const

function msg(p: Partial<ChatNode> & { id: string }): ChatNode {
  return { principal: AS_AGENT, type: NODE_KIND_BODY, status: 'completed', ...p }
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
    expect(previewOf({ principal: AS_AGENT, content: '正文' })).toBe('正文')
    expect(previewOf({ principal: AS_PERSON, content: '用户说的' })).toBeNull()
    expect(previewOf({ principal: AS_TOOL, content: '结果' })).toBeNull()
  })

  it('多模态内容只拼 text 分片（图片段不产生预览）', () => {
    expect(
      previewOf({
        principal: AS_AGENT,
        content: [
          { type: 'text', text: '看这张' },
          { type: 'image_url', image_url: { url: 'data:x' } },
        ],
      }),
    ).toBe('看这张')
    // 只有图片、没有文本 → 无预览（不能把空串当成"预览为空"写进去）
    expect(
      previewOf({ principal: AS_AGENT, content: [{ type: 'image_url', image_url: { url: 'x' } }] }),
    ).toBeNull()
  })

  it('超长截断并加省略号；空内容返回 null', () => {
    const long = 'x'.repeat(PREVIEW_MAX + 20)
    const out = previewOf({ principal: AS_AGENT, content: long })
    expect(out).not.toBeNull()
    expect(out!.length).toBe(PREVIEW_MAX + 1)
    expect(out!.endsWith('…')).toBe(true)
    expect(previewOf({ principal: AS_AGENT, content: '' })).toBeNull()
    expect(previewOf({ principal: AS_AGENT })).toBeNull()
  })
})

describe('appendContent：帧携带 `delta` 时的唯一内容落点', () => {
  it('把增量拼到正文尾部', () => {
    const out = appendContent(msg({ id: 'x', content: '你好' }), '世界')
    expect(out.content).toBe('你好世界')
  })

  it('工具**参数**帧同样追加（参数是窄增量，不是全量重发）', () => {
    const out = appendContent(msg({ id: 'tc', type: NODE_KIND_SKELETON, content: '{"a' }), '":1}')
    expect(out.content).toBe('{"a":1}')
  })

  it('role=tool 的**工具响应**同样追加（响应被覆盖成空的回归）', () => {
    // 回归：`mergeMessagePatch` 曾把 `role=tool` 判为"流式帧全量重发"因而整条替换。
    // 后端透传子会话的 `delta`（逐片响应），前端于是只剩最后一片——
    // 表现为"工具卡片有请求、响应是空的"。语义只由帧字段给出，不看角色。
    const out = appendContent(msg({ id: 'r', principal: AS_TOOL, content: '共有 ' }), '10 个文件')
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
  it('进行中 = streaming / waiting_user_action（其余已是终态）', () => {
    expect(isInProgressMessage({ status: 'streaming' })).toBe(true)
    expect(isInProgressMessage({ status: 'waiting_user_action' })).toBe(true)
    expect(isInProgressMessage({ status: 'completed' })).toBe(false)
    expect(isInProgressMessage({ status: 'failed' })).toBe(false)
    expect(isInProgressMessage({ status: 'aborted' })).toBe(false)
    expect(isInProgressMessage({})).toBe(false)
  })
})

describe('dropUnselectedBranches：未选中支连同整棵子树一起让位（[D-173]）', () => {
  /** 一条成员根：`selected` 决定它藏不藏。候选由夹具手写（读侧只认长度与下标）。 */
  const branch = (id: string, candidates: string[], index: number, selected: boolean) =>
    msg({
      id,
      meta: { branch: { group: candidates[0], candidates, index, total: candidates.length, selected } },
    })
  const child = (id: string, parent?: string, turnRoot?: string) =>
    msg({ id, parent_id: parent, meta: turnRoot ? { turn_root: turnRoot } : undefined })
  const ids = (list: ChatNode[]) => list.map((m) => m.id)

  it('没有分支标注 ⇒ 原样返回**同一个**数组引用（每一次可见消息计算都跑它，不能白复制）', () => {
    const input = [child('u1'), child('t1', 'u1'), child('r1', 't1')]
    expect(dropUnselectedBranches(input)).toBe(input)
  })

  it('藏掉未选中支连同它的子孙；共同祖先留着', () => {
    const list = [
      child('u1'),
      branch('t1', ['t1', 't2'], 0, false),
      child('r1', 't1'),
      branch('t2', ['t1', 't2'], 1, true),
      child('r2', 't2'),
    ]
    expect(ids(dropUnselectedBranches(list))).toEqual(['u1', 't2', 'r2'])
  })

  it('`flat` 档升根的子孙（`parent_id` 为空、靠 `meta.turn_root` 认回）一起藏', () => {
    // 这一支的工具结果在呈现树里被升成了根：只按 `parent_id` 收子树会把它漏在外面，
    // 于是模型与界面各读一套——界面少了这一条，模型多读一条矛盾的答复。
    const list = [
      child('u1'),
      branch('t1', ['t1', 't2'], 0, false),
      child('w1', undefined, 't1'),
      child('w2', 'w1'),
      branch('t2', ['t1', 't2'], 1, true),
      child('v1', undefined, 't2'),
    ]
    expect(ids(dropUnselectedBranches(list))).toEqual(['u1', 't2', 'v1'])
  })

  it('选中态换了 ⇒ 藏的是另一支：判据只来自标注，不在这里重算默认', () => {
    const list = [
      branch('t1', ['t1', 't2'], 0, true),
      branch('t2', ['t1', 't2'], 1, false),
    ]
    expect(ids(dropUnselectedBranches(list))).toEqual(['t1'])
  })

  it('三支时只留被读的那一支', () => {
    const list = [
      branch('t1', ['t1', 't2', 't3'], 0, false),
      branch('t2', ['t1', 't2', 't3'], 1, true),
      branch('t3', ['t1', 't2', 't3'], 2, false),
      child('r1', 't1'),
      child('r2', 't2'),
      child('r3', 't3'),
    ]
    expect(ids(dropUnselectedBranches(list))).toEqual(['t2', 'r2'])
  })

  it('形状不齐的标注读成「不在组里」⇒ 一条都不藏（宁显示两支，也不静默删掉一条历史）', () => {
    const list = [
      msg({ id: 't1', meta: { branch: { group: 't1', candidates: ['t1'], index: 0, total: 2 } } }),
      msg({ id: 't2', meta: { branch: 'not-an-object' } }),
    ]
    expect(ids(dropUnselectedBranches(list))).toEqual(['t1', 't2'])
  })

  it('子边成环时转得完（脏数据不把渲染卡死）', () => {
    // 一条自己指认自己是自己子孙的根（parent_id 成环的最小形状）。
    const list = [{ ...branch('t1', ['t1', 't2'], 0, false), parent_id: 't1' }, branch('t2', ['t1', 't2'], 1, true)]
    expect(ids(dropUnselectedBranches(list))).toEqual(['t2'])
  })
})

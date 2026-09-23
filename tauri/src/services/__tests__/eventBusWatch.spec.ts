/**
 * subscribeVdfsChanged —— 「订阅即登记」单测（node 环境）
 *
 * 锁住一个静默失效：只订阅前端总线而不向后端 `vdfs/watch` 登记路径，
 * 后端就不会往这条频道投递任何变更——前端模式（本地发布）照常工作、
 * 单测照常绿，接上真实后端后一条变更都收不到。
 *
 * 因此这里验证三件事：
 * 1. 订阅会按作用域前缀登记 watch，取消订阅会摘除；
 * 2. 同一前缀的多个订阅者**共享**一次登记（引用计数），最后一个撤走才 unwatch；
 * 3. 订虚拟根不登记（根不在任何 provider 身上，登记只会换来后端报错）。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn(), connectPlugin: vi.fn() }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() }
}))

import { callPlugin } from '@/services/plugin'
import { VDFS_UNWATCH, VDFS_WATCH, type VdfsChange } from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

// 合成根：与根名无关（见 schemas/__tests__/vdfs.spec.ts 的说明）
setVdfsRoot('@vfs')
import { publishVdfsChangedLocal, subscribeVdfsChanged, vdfsChangeInScope } from '../eventBus'

/** 已发生的 watch / unwatch 登记（按调用顺序） */
function registrations(): Array<{ op: string; path: string }> {
  return vi
    .mocked(callPlugin)
    .mock.calls.filter(([op]) => op === VDFS_WATCH || op === VDFS_UNWATCH)
    .map(([op, payload]) => ({
      op: op as string,
      path: (payload as { path: string }).path
    }))
}

/**
 * 等登记链跑完：链上是 `prev.then(async () => await callPlugin(...))`，
 * 深度随订阅数增长，逐个 await 微任务数不过来——排空微任务队列的宏任务一步到位。
 */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

beforeEach(() => {
  vi.mocked(callPlugin).mockReset()
  vi.mocked(callPlugin).mockResolvedValue(undefined as never)
})

describe('subscribeVdfsChanged 的后端登记', () => {
  it('订阅即登记作用域前缀，取消订阅即摘除', async () => {
    const off = subscribeVdfsChanged({ prefix: '@vfs/session' }, () => {})
    await settle()

    expect(registrations()).toEqual([{ op: VDFS_WATCH, path: '@vfs/session' }])

    off()
    await settle()

    expect(registrations()).toEqual([
      { op: VDFS_WATCH, path: '@vfs/session' },
      { op: VDFS_UNWATCH, path: '@vfs/session' }
    ])
  })

  it('同一前缀的多个订阅者共享一次登记，最后一个撤走才 unwatch', async () => {
    const offA = subscribeVdfsChanged({ prefix: '@vfs/session' }, () => {})
    const offB = subscribeVdfsChanged({ prefix: '@vfs/session' }, () => {})
    await settle()

    expect(registrations().filter((r) => r.op === VDFS_WATCH)).toHaveLength(1)

    offA()
    await settle()
    // 还有一个订阅者在，不该摘
    expect(registrations().some((r) => r.op === VDFS_UNWATCH)).toBe(false)

    offB()
    await settle()
    expect(registrations().filter((r) => r.op === VDFS_UNWATCH)).toHaveLength(1)
  })

  it('不同前缀各自登记，互不影响', async () => {
    const offList = subscribeVdfsChanged({ prefix: '@vfs/session', directChildren: true }, () => {})
    const offOne = subscribeVdfsChanged({ prefix: '@vfs/session/s1' }, () => {})
    await settle()

    expect(registrations().map((r) => r.path)).toEqual(['@vfs/session', '@vfs/session/s1'])

    offOne()
    await settle()
    expect(registrations()).toEqual([
      { op: VDFS_WATCH, path: '@vfs/session' },
      { op: VDFS_WATCH, path: '@vfs/session/s1' },
      { op: VDFS_UNWATCH, path: '@vfs/session/s1' }
    ])

    offList()
    await vi.waitFor(() => {
      expect(registrations().filter((r) => r.op === VDFS_UNWATCH)).toHaveLength(2)
    })
  })

  it('订虚拟根不登记（根无实时能力）', async () => {
    const off = subscribeVdfsChanged({ prefix: '@vfs' }, () => {})
    await settle()
    expect(registrations()).toEqual([])

    off()
    await settle()
    expect(registrations()).toEqual([])
  })

  it('取消订阅可重复调用，登记计数不会被多扣', async () => {
    const off = subscribeVdfsChanged({ prefix: '@vfs/session' }, () => {})
    await settle()

    off()
    off()
    off()
    await settle()

    expect(registrations().filter((r) => r.op === VDFS_UNWATCH)).toHaveLength(1)
  })
})

/**
 * `delta` 是**传输形态**的正文增量（后端 `VdfsChange` 的可选字段），不是一条
 * 独立的变更类型。
 *
 * 锁两件事，两件都是静默失效：
 *
 * 1. **带 `delta` 的变更不得被形状判定丢掉**——`dispatch` 会先检查 `path` 是不是
 *    字符串；若哪天有人给「载荷变更」加一条独立分支，这条会红。
 * 2. **`delta` 必须原样到达消费者**——它决定「就地追加还是回读」，
 *    被转发层裁掉会退化成静默的「少一段字」。
 */
describe('带 delta 的变更照常派发', () => {
  it('delta 不被当作形状异常丢弃，且原样到达消费者', () => {
    const seen: VdfsChange[] = []
    const off = subscribeVdfsChanged({ prefix: '@vfs/session/s1/消息' }, (c) => seen.push(c))

    publishVdfsChangedLocal({
      path: '@vfs/session/s1/消息',
      data: { id: 'm1', delta: '片段' }
    })

    expect(seen).toHaveLength(1)
    expect((seen[0].data as { delta?: string }).delta).toBe('片段')

    off()
  })

  it('作用域判定只看 path，与是否带 delta 无关', () => {
    const scope = { prefix: '@vfs/session/s1/消息' }
    expect(vdfsChangeInScope(scope, '@vfs/session/s1/消息/m1')).toBe(true)
    expect(vdfsChangeInScope(scope, '@vfs/session/s1')).toBe(false)
    // 带 delta 的变更走的是同一个判定入口（path 决定一切）
    expect(vdfsChangeInScope(scope, '@vfs/session/s1/消息/m2')).toBe(true)
  })
})

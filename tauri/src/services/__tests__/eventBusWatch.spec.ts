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
 * 3. 订 `.vdfs` 根不登记（根不在任何 provider 身上，登记只会换来后端报错）。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn(), connectPlugin: vi.fn() }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() }
}))

import { callPlugin } from '@/services/plugin'
import { VDFS_ROOT, VDFS_UNWATCH, VDFS_WATCH } from '@/schemas/vdfs'
import { subscribeVdfsChanged } from '../eventBus'

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
    const off = subscribeVdfsChanged({ prefix: '.vdfs/session' }, () => {})
    await settle()

    expect(registrations()).toEqual([{ op: VDFS_WATCH, path: '.vdfs/session' }])

    off()
    await settle()

    expect(registrations()).toEqual([
      { op: VDFS_WATCH, path: '.vdfs/session' },
      { op: VDFS_UNWATCH, path: '.vdfs/session' }
    ])
  })

  it('同一前缀的多个订阅者共享一次登记，最后一个撤走才 unwatch', async () => {
    const offA = subscribeVdfsChanged({ prefix: '.vdfs/session' }, () => {})
    const offB = subscribeVdfsChanged({ prefix: '.vdfs/session' }, () => {})
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
    const offList = subscribeVdfsChanged({ prefix: '.vdfs/session', directChildren: true }, () => {})
    const offOne = subscribeVdfsChanged({ prefix: '.vdfs/session/s1' }, () => {})
    await settle()

    expect(registrations().map((r) => r.path)).toEqual(['.vdfs/session', '.vdfs/session/s1'])

    offOne()
    await settle()
    expect(registrations()).toEqual([
      { op: VDFS_WATCH, path: '.vdfs/session' },
      { op: VDFS_WATCH, path: '.vdfs/session/s1' },
      { op: VDFS_UNWATCH, path: '.vdfs/session/s1' }
    ])

    offList()
    await vi.waitFor(() => {
      expect(registrations().filter((r) => r.op === VDFS_UNWATCH)).toHaveLength(2)
    })
  })

  it('订 `.vdfs` 根不登记（根无实时能力）', async () => {
    const off = subscribeVdfsChanged({ prefix: VDFS_ROOT }, () => {})
    await settle()
    expect(registrations()).toEqual([])

    off()
    await settle()
    expect(registrations()).toEqual([])
  })

  it('取消订阅可重复调用，登记计数不会被多扣', async () => {
    const off = subscribeVdfsChanged({ prefix: '.vdfs/session' }, () => {})
    await settle()

    off()
    off()
    off()
    await settle()

    expect(registrations().filter((r) => r.op === VDFS_UNWATCH)).toHaveLength(1)
  })
})

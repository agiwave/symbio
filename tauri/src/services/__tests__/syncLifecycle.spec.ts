/**
 * syncLifecycle —— 「全局唯一订阅」生命周期的护栏（node 环境）
 *
 * 本模块承担的是**模块级订阅的固有陷阱**（HMR 重置模块级变量 ⇒ 订出第二条），
 * 因此它最需要被锁住的恰恰是「HMR 之后还拦不拦得住」——那个场景在业务测试里
 * 永远复现不出来（测试不会热更新模块），只能在这里用一个**新实例**模拟。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}))

import { logger } from '@/utils/logger'
import { createSyncLifecycle } from '../syncLifecycle'

/** 每个用例用独立标记名：否则用例之间会通过 globalThis 互相污染 */
let flag = ''

beforeEach(() => {
  flag = `__symTestLifecycle_${Math.random().toString(36).slice(2)}`
  vi.clearAllMocks()
})

describe('createSyncLifecycle：全局唯一订阅的生命周期', () => {
  it('启动 → 再启动被拦住（幂等），并留痕', () => {
    const lc = createSyncLifecycle('[t]', flag)
    expect(lc.begin()).toBe(true)
    lc.attach(() => {})

    expect(lc.begin()).toBe(false)
    expect(logger.warn).toHaveBeenCalledWith('[t]', 'already started')
  })

  it('HMR 守卫：模块级句柄丢失后，globalThis 上的标记仍拦住第二次订阅', () => {
    const before = createSyncLifecycle('[t]', flag)
    before.begin()
    before.attach(() => {})

    // 模拟模块热更新：模块重新执行 ⇒ 新实例、模块级 unsub 为空，但标记还在
    const after = createSyncLifecycle('[t]', flag)
    expect(after.begin()).toBe(false)
  })

  it('end：调用取消句柄、复位标记，之后可以重启', () => {
    const lc = createSyncLifecycle('[t]', flag)
    const unsub = vi.fn()
    lc.begin()
    lc.attach(unsub)

    lc.end()
    expect(unsub).toHaveBeenCalledTimes(1)

    // 标记已复位 ⇒ 新实例（乃至本实例）都能重新启动
    expect(createSyncLifecycle('[t]', flag).begin()).toBe(true)
  })

  it('启动前解析失败（未 attach 就 end）是安全的，且**不留下标记**', () => {
    const lc = createSyncLifecycle('[t]', flag)
    lc.begin()
    lc.end() // 调用方解析失败时的善后

    expect(logger.info).not.toHaveBeenCalled() // 没订上就无所谓"stopped"
    // 关键：失败不会把同步器永久锁死
    expect(createSyncLifecycle('[t]', flag).begin()).toBe(true)
  })

  it('end 幂等：重复调用只取消一次', () => {
    const lc = createSyncLifecycle('[t]', flag)
    const unsub = vi.fn()
    lc.begin()
    lc.attach(unsub)

    lc.end()
    lc.end()
    expect(unsub).toHaveBeenCalledTimes(1)
  })

  it('边界（有意接受）：begin 不是占位，未 attach 前重复 begin 都返回 true', () => {
    const lc = createSyncLifecycle('[t]', flag)
    expect(lc.begin()).toBe(true)
    expect(lc.begin()).toBe(true) // 标记在 attach 时才置位，见模块文档
  })
})

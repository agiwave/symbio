/**
 * withFallback —— 服务层「吞错返兜底」口径的护栏（node 环境）
 *
 * 本原语是 9 处读/列入口的共同失败口径，它自己必须是被锁住的那一个：
 * 一旦它的行为漂移（比如忘了记日志、或把兜底求值提前到成功路径），
 * 全部调用点的失败行为会一起静默改变——而失败行为恰恰最难在业务测试里被发现。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
}))

import { logger } from '@/utils/logger'
import { withFallback } from '../fallback'

const LOG = { tag: 'some-service', what: 'doThing failed' }

/** 必然抛错的 op，用于走失败分支 */
const boom = (err: unknown = new Error('boom')) => async (): Promise<never> => {
  throw err
}

describe('withFallback：读/列类服务的统一失败口径', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('成功：原样返回，不记日志、不取兜底', async () => {
    const fallback = vi.fn(() => 'fallback')
    const r = await withFallback(async () => 'ok', fallback, LOG)

    expect(r).toBe('ok')
    expect(fallback).not.toHaveBeenCalled()
    expect(logger.error).not.toHaveBeenCalled()
  })

  it('失败：返回兜底值，并以 error 级留痕（tag + what + 原始错误）', async () => {
    const err = new Error('boom')
    const r = await withFallback(boom(err), () => 'fallback', LOG)

    expect(r).toBe('fallback')
    expect(logger.error).toHaveBeenCalledWith('some-service', 'doThing failed:', err)
  })

  it('level 可降噪：预期内的失败走 debug，不污染 error 级日志', async () => {
    await withFallback(boom(), () => null, { ...LOG, level: 'debug' })

    expect(logger.debug).toHaveBeenCalledWith('some-service', 'doThing failed:', expect.anything())
    expect(logger.error).not.toHaveBeenCalled()
  })

  it('兜底是惰性的：成功路径上不为它做任何计算', async () => {
    const fallback = vi.fn(() => {
      throw new Error('兜底不该在成功路径被求值')
    })

    await expect(withFallback(async () => 1, fallback, LOG)).resolves.toBe(1)
    expect(fallback).not.toHaveBeenCalled()
  })

  it('void 场景（订阅/取消订阅类）：失败被吞掉，调用方不必自己包 try', async () => {
    await expect(
      withFallback(boom(), () => {}, { ...LOG, level: 'debug' })
    ).resolves.toBeUndefined()
  })
})

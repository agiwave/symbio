/**
 * 路由信封的**来源字段**（`metadata.origin`）单测
 *
 * 这一条守的是「诊断事实能不能到达日志」：`origin` 的整条价值链是
 * 「调用方给理由 → `buildMetadata` 写进 `metadata.origin` → Tauri 的
 * `Start routing` 留痕打出」。中间任何一环断掉都**不会报错**——日志里只是少了
 * 那个字段，于是又回到「从时间戳反推谁发的」。
 *
 * 因此这里不测「函数被调过」，而是直接断言**送上 IPC 的那个信封**里有什么：
 * 有理由就必须有 `origin`；没给理由就**一个键都不多**（否则「有没有来源」
 * 本身不可判，且所有路由的信封形状都会悄悄变一遍）。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))

import { invoke } from '@tauri-apps/api/core'
import { callPlugin, HEAD_ORIGIN, HEAD_PATH } from '../plugin'

/** 取第 n 次 `invoke('route_v2', …)` 送上 IPC 的信封 */
function envelope(n = 0): { metadata: Record<string, string>; payload: unknown } {
  const call = vi.mocked(invoke).mock.calls[n]
  expect(call[0], '只走 route_v2 这一条原生通道').toBe('route_v2')
  return (call[1] as { request: { metadata: Record<string, string> } }).request as never
}

/** 让 `route_v2` 立即回一个同步 Data 帧（`timeoutMs = 0` 时不挂超时竞速） */
function stubImmediateData(): void {
  vi.mocked(invoke).mockResolvedValue({
    metadata: {},
    payload: { type: 'Data', data: null },
  } as never)
}

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  stubImmediateData()
})

describe('buildMetadata：来源随请求走', () => {
  it('给了 origin ⇒ 信封里带上它（日志据此回答「谁、为什么」）', async () => {
    await callPlugin('vdfs/stat', { path: '/p' }, 0, { origin: 'missing-baseline' })

    const env = envelope()
    expect(env.metadata[HEAD_ORIGIN]).toBe('missing-baseline')
    expect(env.metadata[HEAD_PATH]).toBe('vdfs/stat')
  })

  it('没给 origin ⇒ **不写这个键**（信封形状与从前逐字节一致）', async () => {
    await callPlugin('vdfs/stat', { path: '/p' }, 0)

    const env = envelope()
    expect(env.metadata).not.toHaveProperty(HEAD_ORIGIN)
  })

  it('来源与路由正交：换路由不改来源，换来源不改路由', async () => {
    await callPlugin('vdfs/read', { path: '/p' }, 0, { origin: 'transcript-load' })
    await callPlugin('vdfs/list', { path: '/p' }, 0, { origin: 'bootstrap' })

    expect(envelope(0).metadata).toMatchObject({
      [HEAD_PATH]: 'vdfs/read',
      [HEAD_ORIGIN]: 'transcript-load',
    })
    expect(envelope(1).metadata).toMatchObject({
      [HEAD_PATH]: 'vdfs/list',
      [HEAD_ORIGIN]: 'bootstrap',
    })
  })
})

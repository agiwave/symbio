/**
 * 全局连接状态 —— 「断连可见性」的判据（`DE-020` 的「还差什么」表第 3 行）
 *
 * 锁两件事：
 * 1. 状态**从真实连接事件来**：接上 ⇒ `connected`，断开 / 失败 ⇒ `reconnecting`；
 * 2. 写口唯一：界面读的是 `connectionStatus()` 的只读视图，事件总线是唯一写方——
 *    「界面说断开、总线还在收帧」这种分裂在类型上就写不出来。
 *
 * 顺带锁住「首连那一瞬」：初始态是 `connecting`，不是 `connected`。把它当已连接会让
 * 启动窗内的失败静默（用户看到的是「什么都没发生」而不是「正在连接」）。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/services/plugin', () => ({ callPlugin: vi.fn(), connectPlugin: vi.fn() }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() }
}))

import { connectPlugin, type ConnectEvent, type Connection } from '@/services/plugin'
import { setVdfsRoot } from '@/schemas/vdfsRoot'

setVdfsRoot('@vfs')
import { _resetEventBusForTest, connectEventBus } from '../eventBus'
import { connectionStatus } from '../connectionStatus'

/** 让 `connectPlugin` 返回一个**未连接**的桩（再次 `connectEventBus` 才会真重连）。 */
function stubConnect(): void {
  vi.mocked(connectPlugin).mockImplementation(
    async () =>
      ({
        connectionId: 'c-test',
        isConnected: false,
        close: vi.fn(async () => {})
      }) as unknown as Connection
  )
}

/** 连接体交出的那个 `onEvent`：断开事件只能从它进来。 */
function handlerOfCall(index: number): (e: ConnectEvent) => void {
  const args = vi.mocked(connectPlugin).mock.calls[index]
  const handler = args?.[2]
  expect(typeof handler).toBe('function')
  return handler as (e: ConnectEvent) => void
}

beforeEach(() => {
  _resetEventBusForTest()
  vi.mocked(connectPlugin).mockReset()
  stubConnect()
})

describe('连接状态', () => {
  it('初始态是 connecting，不是 connected', () => {
    expect(connectionStatus().value).toBe('connecting')
  })

  it('接上 ⇒ connected；断开事件 ⇒ reconnecting；再接上 ⇒ connected', async () => {
    await connectEventBus()
    expect(connectionStatus().value).toBe('connected')

    handlerOfCall(0)({ type: 'disconnected', data: { reason: 'test' } })
    expect(connectionStatus().value).toBe('reconnecting')

    await connectEventBus()
    expect(connectionStatus().value).toBe('connected')
  })

  it('连接失败也算 reconnecting（不是停在 connecting）', async () => {
    vi.mocked(connectPlugin).mockRejectedValueOnce(new Error('boom'))
    await connectEventBus().catch(() => {})
    expect(connectionStatus().value).toBe('reconnecting')
  })

  it('只读视图不可写：写口只有总线', () => {
    const status = connectionStatus()
    // `readonly(ref)` 的 setter 写入时告警并保持原值（Vue 的实现），这里只断言
    // **值没被写进去**——告警文案不是契约。
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    ;(status as unknown as { value: string }).value = 'connected'
    expect(status.value).toBe('connecting')
    warn.mockRestore()
  })
})

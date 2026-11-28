/**
 * plugin.ts 传输层回归（HTTP/WS 唯一通道）
 *
 * 三端交付物（桌面启动器 / CLI serve / web）共用这层：它锁住的是几条会静默出错的契约：
 * 1. `callPlugin` 的 invoke 语义走 POST /api/v1/invoke，取回最后一帧；
 * 2. Error 帧优先于 EOF 的失败出口；
 * 3. 超时关连接、请求 settle 后清定时器——不能让定时器把活跃连接关掉；
 * 4. 持久会话走 WS，token 只走子协议不进 URL；远端不可达只报错、不回退。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/** vi.mock 工厂在 import 前执行，共享状态必须走 vi.hoisted。 */
const S = vi.hoisted(() => ({
  location: { kind: 'local' } as { kind: 'local' | 'remote'; remoteUrl?: string; remoteKey?: string },
  fetchCalls: [] as Array<{ url: string; init?: RequestInit }>,
}))

vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), debug: vi.fn(), info: vi.fn() },
}))
vi.mock('../systemLocation', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../systemLocation')>()
  return {
    ...actual,
    loadSystemLocation: vi.fn(() => S.location),
    setCurrentLocation: vi.fn(),
    setLocationError: vi.fn(),
  }
})

import { logger } from '@/utils/logger'
import { setLocationError } from '../systemLocation'
import {
  callPlugin,
  connectPlugin,
  getConnectionModel,
  getOutboundConfig,
  reloadGatewayTransport,
} from '../plugin'

class FakeWebSocket {
  static readonly OPEN = 1
  static instances: FakeWebSocket[] = []
  readyState = 0
  onopen: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  sent: string[] = []
  closed = false

  constructor(public readonly url: string, public readonly protocols: string[]) {
    FakeWebSocket.instances.push(this)
    queueMicrotask(() => {
      this.readyState = FakeWebSocket.OPEN
      this.onopen?.()
    })
  }

  send(data: string): void { this.sent.push(data) }
  close(): void { this.closed = true; this.readyState = 3; this.onclose?.() }
}

/** 让 /api/v1/invoke 回一个 Data 帧 */
function stubInvokeData(data: unknown): void {
  vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
    if (String(url).includes('/health')) return { ok: true }
    S.fetchCalls.push({ url: String(url), init })
    return { ok: true, text: async () => JSON.stringify({ type: 'Data', data }) }
  }))
}

function fireWsFrame(ws: FakeWebSocket, frame: unknown): void {
  ws.onmessage?.({ data: JSON.stringify(frame) })
}

beforeEach(async () => {
  vi.stubGlobal('window', {})
  vi.stubGlobal('location', { origin: 'http://localhost:1420' })
  stubInvokeData(null)
  vi.stubGlobal('WebSocket', FakeWebSocket)
  vi.mocked(logger.error).mockClear()
  vi.mocked(setLocationError).mockClear()
  S.fetchCalls.length = 0
  S.location = { kind: 'local' }
  FakeWebSocket.instances = []
  await reloadGatewayTransport()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('callPlugin：invoke 语义', () => {
  it('Data 响应 ⇒ 直接 resolve', async () => {
    stubInvokeData({ ok: 1 })
    await expect(callPlugin('vdfs/stat')).resolves.toEqual({ ok: 1 })
    expect(S.fetchCalls.map((c) => c.url)).toEqual(['http://localhost:1420/api/v1/invoke'])
  })

  it('非 2xx ⇒ 带 error 正文抛错', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (String(url).includes('/health')) return { ok: true }
      return { ok: false, status: 500, text: async () => JSON.stringify({ error: 'gateway exploded' }) }
    }))
    await expect(callPlugin('vdfs/stat')).rejects.toThrow('gateway exploded')
  })

  it('超时 ⇒ reject，且定时器在 settle 后清理', async () => {
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      if (String(url).includes('/health')) return { ok: true }
      return new Promise(() => {}) // 永不返回
    }))
    const pending = callPlugin('vdfs/stat', undefined, 20)
    const error = pending.catch((e) => e) // 先挂 handler，避免迟到挂载的 unhandled rejection
    await expect(error).resolves.toMatchObject({ message: expect.stringMatching(/timed out/) })
  })
})

describe('WS 持久会话', () => {
  it('URL 无 token，Bearer 通过子协议传递，http 映射为 ws', async () => {
    S.location = { kind: 'remote', remoteUrl: 'http://remote.test?token=url-key', remoteKey: '' }
    await reloadGatewayTransport()

    await connectPlugin('event/bus', { channel: 'x' })
    const ws = FakeWebSocket.instances[0]
    expect(ws.url).toBe('ws://remote.test/api/v1/ws')
    expect(ws.url).not.toContain('token')
    expect(ws.protocols).toEqual(['symbio.v1', 'bearer.url-key'])
    expect(ws.sent).toHaveLength(1)
  })

  it('无 token 时只发送 symbio.v1 子协议', async () => {
    await connectPlugin('event/bus')
    const ws = FakeWebSocket.instances[0]
    expect(ws.url).toBe('ws://localhost:1420/api/v1/ws')
    expect(ws.protocols).toEqual(['symbio.v1'])
  })

  it('EOF(onclose) ⇒ disconnected(done)', async () => {
    const onEvent = vi.fn()
    await connectPlugin('event/bus', undefined, onEvent)
    FakeWebSocket.instances[0].close()
    expect(onEvent).toHaveBeenCalledWith({ type: 'disconnected', data: { reason: 'done' } })
  })

  it('Error 帧 ⇒ 两条事件且标记断开', async () => {
    const onEvent = vi.fn()
    const conn = await connectPlugin('event/bus', undefined, onEvent)
    fireWsFrame(FakeWebSocket.instances[0], { Error: ['stream failed', null] })

    expect(onEvent).toHaveBeenCalledWith({ type: 'error', data: 'stream failed' })
    expect(onEvent).toHaveBeenCalledWith({ type: 'disconnected', data: { reason: 'error' } })
    expect(conn.isConnected).toBe(false)
  })
})

describe('传输模型与错误口径', () => {
  it('local 默认同源 Server；forceNative 兼容提示不改变出站', async () => {
    expect(getConnectionModel()).toEqual({ environment: 'browser', target: 'server' })
    await expect(callPlugin('vdfs/stat')).resolves.toBeNull()
    await expect(callPlugin('home/reload', {}, 0, { forceNative: true })).resolves.toBeNull()
    expect(getOutboundConfig()).toEqual({ protocol: 'http', endpoint: 'http://localhost:1420', token: '' })
    expect(S.fetchCalls.map((c) => c.url)).toEqual([
      'http://localhost:1420/api/v1/invoke',
      'http://localhost:1420/api/v1/invoke',
    ])
  })

  it('远端不可达只显示 locationError，保留 HTTP 连接', async () => {
    S.location = { kind: 'remote', remoteUrl: 'http://remote.test', remoteKey: 'k' }
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline')))
    await reloadGatewayTransport()

    await expect(callPlugin('vdfs/stat', {}, 0, { forceNative: true })).rejects.toThrow('offline')
    expect(setLocationError).toHaveBeenCalledWith(expect.stringContaining('保留 Server HTTP 连接'))
    expect(getConnectionModel()).toEqual({ environment: 'browser', target: 'server' })
    expect(getOutboundConfig()).toEqual({ protocol: 'http', endpoint: 'http://remote.test', token: 'k' })
  })
})

/**
 * plugin.ts 传输层回归：握手、双传输、超时/EOF 竞态
 *
 * 这一层是 3 个 Tauri 命令与 HTTP 网关之间的唯一边界，也是全仓测试最薄的地方。
 * 锁住的不是「函数被调用过」，而是几条会静默出错的契约：
 * 1. `conn_id` 恒等于预注册的 `clientId`——不等时当场红，不能静默丢帧；
 * 2. EOF 是会话式调用的收束口，Error 帧优先于 EOF 的失败出口；
 * 3. 超时关连接、请求 settle 后清定时器——不能让 30s 定时器把活跃连接关掉；
 * 4. HTTP 两条传输（invoke / ws）的错误路径不会产生「全绿但没数据」。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/** vi.mock 工厂在 import 前执行，共享状态必须走 vi.hoisted。 */
const S = vi.hoisted(() => ({
  frameListeners: new Map<string, (e: { payload: unknown }) => void>(),
  eofListeners: new Map<string, () => void>(),
  unlistened: [] as string[],
  location: { kind: 'local' } as { kind: 'local' | 'remote'; remoteUrl?: string; remoteKey?: string },
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    if (name.endsWith('/eof')) S.eofListeners.set(name, cb as () => void)
    else S.frameListeners.set(name, cb)
    return () => { S.unlistened.push(name) }
  }),
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

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { logger } from '@/utils/logger'
import { setLocationError } from '../systemLocation'
import {
  callPlugin,
  connectPlugin,
  getConnectionModel,
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

  constructor(public readonly url: string, public readonly protocols: string[]) {
    FakeWebSocket.instances.push(this)
    queueMicrotask(() => {
      this.readyState = FakeWebSocket.OPEN
      this.onopen?.()
    })
  }

  send(data: string): void { this.sent.push(data) }
  close(): void { this.readyState = 3; this.onclose?.() }
}

/** 让 `route_v2` 回一个 Connection 帧，且 `data` 取它收到的 clientId（满足契约）。 */
function stubConnectionResponse(): void {
  vi.mocked(invoke).mockImplementation(async (_cmd, args: unknown) => {
    const clientId = (args as { clientId: string }).clientId
    return {
      metadata: {},
      payload: { type: 'Connection', data: clientId },
    } as never
  })
}

/** 取会话 id：callPlugin 内部先 await 传输初始化才发 invoke，故须等到 route_v2 出现。 */
async function lastSessionId(): Promise<string> {
  await vi.waitFor(() => {
    expect(vi.mocked(invoke).mock.calls.some((c) => c[0] === 'route_v2')).toBe(true)
  })
  const call = vi.mocked(invoke).mock.calls.find((c) => c[0] === 'route_v2')!
  return (call[1] as { clientId: string }).clientId
}

function fireFrame(sessionId: string, frame: unknown): void {
  S.frameListeners.get(`route/${sessionId}`)?.({ payload: frame })
}

function fireEof(sessionId: string): void {
  S.eofListeners.get(`route/${sessionId}/eof`)?.()
}

beforeEach(async () => {
  vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: vi.fn() } })
  vi.stubGlobal('location', { origin: 'http://localhost:1420' })
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockResolvedValue({
    metadata: {},
    payload: { type: 'Data', data: null },
  } as never)
  vi.mocked(listen).mockClear()
  vi.mocked(logger.error).mockClear()
  vi.mocked(setLocationError).mockClear()
  S.frameListeners.clear()
  S.eofListeners.clear()
  S.unlistened.length = 0
  S.location = { kind: 'local' }
  FakeWebSocket.instances = []
  await reloadGatewayTransport()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('nativeTransport：握手契约', () => {
  it('conn_id !== clientId ⇒ 当场抛错，不静默丢帧', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      metadata: {},
      payload: { type: 'Connection', data: 'another-id' },
    } as never)

    await expect(callPlugin('vdfs/stat', undefined, 0)).rejects.toThrow(/conn_id.*不一致/)
    // catch 统一清理：同一对监听器只 unlisten 一次（不重复调）
    expect(S.unlistened.length).toBe(2)
  })

  it('invoke 抛错 ⇒ 监听器全部清理，错误原样上抛', async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error('ipc down'))

    await expect(callPlugin('vdfs/stat', undefined, 0)).rejects.toThrow('ipc down')
    expect(S.unlistened.length).toBe(2)
  })

  it('同步 Data 响应 ⇒ 立即 resolve，不挂会话', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      metadata: {},
      payload: { type: 'Data', data: { ok: 1 } },
    } as never)

    await expect(callPlugin('vdfs/stat')).resolves.toEqual({ ok: 1 })
  })
})

describe('callPlugin：EOF / Error / 超时竞态', () => {
  it('EOF 收束会话式调用，取最后一帧 Data', async () => {
    stubConnectionResponse()
    const pending = callPlugin<{ n: number }>('chat/stream', undefined, 0)
    const sid = await lastSessionId()

    fireFrame(sid, { Data: { type: 'chunk', data: { n: 1 } } })
    fireFrame(sid, { Data: { type: 'chunk', data: { n: 42 } } })
    fireEof(sid)

    await expect(pending).resolves.toEqual({ n: 42 })
  })

  it('Error 帧优先于 EOF ⇒ reject 错误正文', async () => {
    stubConnectionResponse()
    const pending = callPlugin('chat/stream', undefined, 0)
    const sid = await lastSessionId()

    fireFrame(sid, { Error: ['boom', null] })
    fireEof(sid)

    await expect(pending).rejects.toThrow('boom')
  })

  it('超时 ⇒ 关连接并 reject；定时器在 settle 后清理', async () => {
    stubConnectionResponse()
    const pending = callPlugin('chat/stream', undefined, 20)
    // handler 必须在 reject 前挂上——否则 Node 把迟到挂载判成 unhandled rejection
    const error = pending.catch((e) => e)
    const sid = await lastSessionId()

    await expect(error).resolves.toMatchObject({ message: expect.stringMatching(/timed out/) })
    expect(vi.mocked(invoke).mock.calls.some((c) => c[0] === 'route_v2_close')).toBe(true)
    expect(S.eofListeners.has(`route/${sid}/eof`)).toBe(true)
  })
})

describe('connectPlugin：会话建立与错误事件', () => {
  it('建立连接后 Error 帧 ⇒ error + disconnected 两条事件', async () => {
    stubConnectionResponse()
    const onEvent = vi.fn()
    const conn = await connectPlugin('event/bus', undefined, onEvent)
    expect(conn.isConnected).toBe(true)
    const sid = await lastSessionId()

    fireFrame(sid, { Error: ['stream failed', null] })

    expect(onEvent).toHaveBeenCalledWith({ type: 'error', data: 'stream failed' })
    expect(onEvent).toHaveBeenCalledWith({ type: 'disconnected', data: { reason: 'error' } })
  })

  it('EOF ⇒ disconnected(done)', async () => {
    stubConnectionResponse()
    const onEvent = vi.fn()
    await connectPlugin('event/bus', undefined, onEvent)
    fireEof(await lastSessionId())

    expect(onEvent).toHaveBeenCalledWith({ type: 'disconnected', data: { reason: 'done' } })
  })

  it('响应里没有 Connection ⇒ 当场拒绝', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      metadata: {},
      payload: { type: 'Data', data: null },
    } as never)

    await expect(connectPlugin('event/bus')).rejects.toThrow(/session failed to establish/)
  })
})

describe('协议违规：非法帧不击穿边界', () => {
  it('非对象帧 / 多键帧 ⇒ 记 error 日志，不 resolve', async () => {
    stubConnectionResponse()
    let settled = false
    const pending = callPlugin('vdfs/stat', undefined, 0).finally(() => { settled = true })
    const sid = await lastSessionId()

    fireFrame(sid, 'not-an-object')
    fireFrame(sid, { Data: 1, Error: ['x', null] })

    expect(vi.mocked(logger.error).mock.calls.length).toBeGreaterThanOrEqual(2)
    expect(settled).toBe(false)
    fireEof(sid)
    await pending.catch(() => {})
  })
})

describe('http 传输：远端不可达回退 + invoke 错误', () => {
  it('embedded 健康探测失败 ⇒ 回退 native（不卡死）', async () => {
    S.location = { kind: 'remote', remoteUrl: 'http://remote.invalid', remoteKey: 'k' }
    const fetchMock = vi.fn().mockRejectedValue(new Error('offline'))
    vi.stubGlobal('fetch', fetchMock)

    await reloadGatewayTransport()
    await callPlugin('vdfs/stat')

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(vi.mocked(invoke)).toHaveBeenCalled()
  })

  it('invoke 非 2xx ⇒ 带 error 正文抛错', async () => {
    S.location = { kind: 'remote', remoteUrl: 'http://remote.test', remoteKey: 'k' }
    const fetchMock = vi.fn(async (url: string) => {
      if (String(url).includes('/health')) return { ok: true }
      return {
        ok: false,
        status: 500,
        text: async () => JSON.stringify({ error: 'gateway exploded' }),
      }
    })
    vi.stubGlobal('fetch', fetchMock)

    await reloadGatewayTransport()
    await expect(callPlugin('vdfs/stat')).rejects.toThrow('gateway exploded')
  })
})

describe('browser 传输模型', () => {
  it('local 默认同源 Server；普通路由、forceNative 与 gateway 控制面均不 invoke', async () => {
    vi.stubGlobal('window', {})
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => ({
      ok: true,
      text: async () => JSON.stringify({ type: 'Data', data: { ok: true } }),
    }))
    vi.stubGlobal('fetch', fetchMock)
    await reloadGatewayTransport()

    expect(getConnectionModel()).toEqual({ environment: 'browser', target: 'server' })
    await expect(callPlugin('vdfs/stat')).resolves.toEqual({ ok: true })
    await expect(callPlugin('home/reload', {}, 0, { forceNative: true })).resolves.toEqual({ ok: true })
    await expect(callPlugin('gateway/config', {}, 0, { forceNative: true })).resolves.toEqual({ ok: true })

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      'http://localhost:1420/api/v1/invoke',
      'http://localhost:1420/api/v1/invoke',
      'http://localhost:1420/api/v1/invoke',
    ])
    expect(invoke).not.toHaveBeenCalled()
    expect(listen).not.toHaveBeenCalled()
  })

  it('远端不可达只显示 locationError，保留 HTTP 且绝不回退 native', async () => {
    vi.stubGlobal('window', {})
    S.location = { kind: 'remote', remoteUrl: 'http://remote.test', remoteKey: 'k' }
    const fetchMock = vi.fn().mockRejectedValue(new Error('offline'))
    vi.stubGlobal('fetch', fetchMock)
    await reloadGatewayTransport()

    await expect(callPlugin('vdfs/stat', {}, 0, { forceNative: true })).rejects.toThrow('offline')
    expect(setLocationError).toHaveBeenCalledWith(expect.stringContaining('浏览器不会回退本机连接'))
    expect(getConnectionModel()).toEqual({ environment: 'browser', target: 'server' })
    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      'http://remote.test/api/v1/health',
      'http://remote.test/api/v1/invoke',
    ])
    expect(invoke).not.toHaveBeenCalled()
    expect(listen).not.toHaveBeenCalled()
  })
})

describe('WebSocket 传输', () => {
  it('URL 无 token，Bearer 通过子协议传递，http 映射为 ws', async () => {
    S.location = { kind: 'remote', remoteUrl: 'http://remote.test?token=url-key', remoteKey: '' }
    vi.stubGlobal('fetch', vi.fn(async (_url: string, _init?: RequestInit) => ({ ok: true })))
    vi.stubGlobal('WebSocket', FakeWebSocket)
    await reloadGatewayTransport()

    await connectPlugin('event/bus', { channel: 'x' })
    const ws = FakeWebSocket.instances[0]
    expect(ws.url).toBe('ws://remote.test/api/v1/ws')
    expect(ws.url).not.toContain('token')
    expect(ws.protocols).toEqual(['symbio.v1', 'bearer.url-key'])
    expect(ws.sent).toHaveLength(1)
  })

  it('无 token 时只发送 symbio.v1 子协议', async () => {
    vi.stubGlobal('window', {})
    vi.stubGlobal('fetch', vi.fn(async (_url: string, _init?: RequestInit) => ({
      ok: true,
      text: async () => JSON.stringify({ type: 'Data', data: null }),
    })))
    vi.stubGlobal('WebSocket', FakeWebSocket)
    await reloadGatewayTransport()

    await connectPlugin('event/bus')
    const ws = FakeWebSocket.instances[0]
    expect(ws.url).toBe('ws://localhost:1420/api/v1/ws')
    expect(ws.protocols).toEqual(['symbio.v1'])
    expect(invoke).not.toHaveBeenCalled()
  })
})

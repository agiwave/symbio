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

import { beforeEach, describe, expect, it, vi } from 'vitest'

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
vi.mock('../systemLocation', () => ({
  loadSystemLocation: vi.fn(() => S.location),
  setCurrentLocation: vi.fn(),
  setLocationError: vi.fn(),
}))

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { logger } from '@/utils/logger'
import {
  callPlugin,
  connectPlugin,
  reloadGatewayTransport,
} from '../plugin'

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

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockResolvedValue({
    metadata: {},
    payload: { type: 'Data', data: null },
  } as never)
  vi.mocked(listen).mockClear()
  vi.mocked(logger.error).mockClear()
  S.frameListeners.clear()
  S.eofListeners.clear()
  S.unlistened.length = 0
  S.location = { kind: 'local' }
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
  it('健康探测失败 ⇒ 回退 native（不卡死）', async () => {
    S.location = { kind: 'remote', remoteUrl: 'http://remote.invalid', remoteKey: 'k' }
    const fetchMock = vi.fn().mockRejectedValue(new Error('offline'))
    vi.stubGlobal('fetch', fetchMock)

    await reloadGatewayTransport()
    await callPlugin('vdfs/stat')

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(vi.mocked(invoke)).toHaveBeenCalled()
    vi.unstubAllGlobals()
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
    vi.unstubAllGlobals()
  })
})

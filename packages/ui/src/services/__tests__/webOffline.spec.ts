// @vitest-environment happy-dom
/**
 * Web 交付物「无后端也能起来」的组合护栏。
 *
 * 服务层的失败口径由 `fallback.spec.ts` / `vdfs.spec.ts` / `home.spec.ts` 各自锁住，
 * 环境判定由 `systemLocation.spec.ts` 锁住，传输由 `pluginTransport.spec.ts` 锁住。
 * 这里补的是它们之间那条缝：`apps/web` 打出的产物在**没有后端、也没有 IPC** 时，
 * 按 main.ts 的启动顺序走一遍，是否真的能走到挂载。
 *
 * 钉的是三件会静默漂移的事：
 * 1. browser + local 选 HTTP 出站，绝不回退 Tauri IPC（否则 web 一启动就崩）；
 * 2. 读类在后端不可达时返回**同型**兜底，页面照常渲染；
 * 3. 写类直抛，不把失败吞成"成功"假象——启动链里由 main.ts 的 try/catch 兜住。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'

/** vi.mock 工厂在 import 前执行；跨 mock 共享状态必须走 vi.hoisted。 */
const S = vi.hoisted(() => ({
  location: { kind: 'local', localPath: '' } as {
    kind: 'local' | 'remote'
    localPath?: string
    remoteUrl?: string
    remoteKey?: string
  },
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
vi.mock('@/utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn(), debug: vi.fn() },
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
vi.mock('../plugin', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../plugin')>()
  return {
    ...actual,
    callPlugin: vi.fn(async () => {
      throw new Error('offline: no backend')
    }),
  }
})

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { isBrowserEnvironment } from '../systemLocation'
import { getConnectionModel, reloadGatewayTransport } from '../plugin'
import { getHomedirInfo, restoreHomedirAtStartup, switchHomedir } from '../home'
import { vdfsJoin } from '@/schemas/vdfs'
import { setVdfsRoot } from '@/schemas/vdfsRoot'
import { ensureVdfsRoot, listVdfs } from '../vdfs'
import { READBACK_REASON } from '../readback'

setVdfsRoot('@vfs')

beforeEach(() => {
  vi.clearAllMocks()
  S.location = { kind: 'local', localPath: '' }
  delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
})

describe('web 交付物：无后端启动链', () => {
  it('无 IPC + 无后端：判定为 browser，出站选 HTTP，启动前两步不中断', async () => {
    expect(isBrowserEnvironment()).toBe(true)

    // main.ts 启动链的前两步：定出站、引导 VDFS 根。后端全挂时都必须 resolve。
    await expect(reloadGatewayTransport()).resolves.toBeUndefined()
    await expect(ensureVdfsRoot()).resolves.toBeUndefined()

    expect(getConnectionModel()).toEqual({ environment: 'browser', target: 'server' })
    // 浏览器没有原生出口：整条链一次都不能碰 IPC。
    expect(invoke).not.toHaveBeenCalled()
    expect(listen).not.toHaveBeenCalled()
  })

  it('读类仍可交互：后端不可达时返回形状合法的空目录，而不是 null/undefined', async () => {
    await reloadGatewayTransport()
    const r = await listVdfs(READBACK_REASON.VDFS_BROWSER, vdfsJoin('@vfs', 'session'))

    expect(r).toEqual({
      path: vdfsJoin('@vfs', 'session'),
      node: expect.objectContaining({ name: vdfsJoin('@vfs', 'session') }),
      items: [],
    })
    expect(invoke).not.toHaveBeenCalled()
  })

  it('写类直抛：restoreHomedirAtStartup 的下发失败不被吞成成功', async () => {
    // 有上次会话目录时，启动回放会尝试下发；后端不可达必须让调用方看到。
    S.location = { kind: 'local', localPath: '/last/session' }
    await expect(restoreHomedirAtStartup()).rejects.toThrow('offline')
    await expect(switchHomedir('/last/session')).rejects.toThrow('offline')
    // 读类在同样条件下仍走兜底——两种口径不能串味。
    await expect(getHomedirInfo()).resolves.toEqual(expect.objectContaining({ homedir: '' }))
    expect(invoke).not.toHaveBeenCalled()
  })
})

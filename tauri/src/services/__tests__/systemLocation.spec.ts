import { afterEach, describe, expect, it, vi } from 'vitest'
import { formatLocation, isBrowserEnvironment } from '../systemLocation'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('运行环境模型', () => {
  it('无 window / 无 Tauri IPC 时判定为 browser', () => {
    vi.stubGlobal('window', undefined)
    expect(isBrowserEnvironment()).toBe(true)
    vi.stubGlobal('window', {})
    expect(isBrowserEnvironment()).toBe(true)
  })

  it('只有注入 invoke 函数的 Tauri WebView 才是 embedded', () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} })
    expect(isBrowserEnvironment()).toBe(true)
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: vi.fn() } })
    expect(isBrowserEnvironment()).toBe(false)
  })

  it('browser local 展示当前 Server，不把 homedir 说成浏览器本机', () => {
    vi.stubGlobal('window', {})
    expect(formatLocation({ kind: 'local', localPath: '/server/home/me' }))
      .toBe('当前 Server：/server/home/me')
    expect(formatLocation({ kind: 'local', localPath: '' })).toBe('当前 Server')
  })

  it('embedded local 仍展示本机 homedir', () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: vi.fn() } })
    expect(formatLocation({ kind: 'local', localPath: '/local/home/me' })).toBe('/local/home/me')
  })
})

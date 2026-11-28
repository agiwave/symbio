import { afterEach, describe, expect, it, vi } from 'vitest'
import { formatLocation, isBrowserEnvironment } from '../systemLocation'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('运行环境模型', () => {
  it('三端交付物统一判定为 browser（不再有 WebView/IPC 形态）', () => {
    vi.stubGlobal('window', undefined)
    expect(isBrowserEnvironment()).toBe(true)
    vi.stubGlobal('window', {})
    expect(isBrowserEnvironment()).toBe(true)
    // 即使注入了旧的 Tauri 桥痕迹也不再改变判定
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: vi.fn() } })
    expect(isBrowserEnvironment()).toBe(true)
  })

  it('local 展示当前 Server，不把 homedir 说成浏览器本机', () => {
    vi.stubGlobal('window', {})
    expect(formatLocation({ kind: 'local', localPath: '/server/home/me' }))
      .toBe('当前 Server：/server/home/me')
    expect(formatLocation({ kind: 'local', localPath: '' })).toBe('当前 Server')
  })
})

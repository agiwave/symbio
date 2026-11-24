import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const H = vi.hoisted(() => ({ open: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: H.open }))

import { pickPath } from '../nativePick'

beforeEach(() => {
  H.open.mockReset()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('统一路径取值', () => {
  it('browser 目录请求交给 DirectoryPicker，不打开原生或浏览器文件对话框', async () => {
    vi.stubGlobal('window', {})
    const onBrowserPick = vi.fn()

    await expect(pickPath('directory', { initialPath: '/srv', onBrowserPick })).resolves.toBeNull()
    expect(onBrowserPick).toHaveBeenCalledWith({ kind: 'directory', initialPath: '/srv' })
    expect(H.open).not.toHaveBeenCalled()
  })

  it('browser 未提供组件回调时明确报错，不静默降级', async () => {
    vi.stubGlobal('window', {})
    await expect(pickPath('directory')).rejects.toThrow('DirectoryPicker')
    expect(H.open).not.toHaveBeenCalled()
  })

  it('Server 目录 API 不提供文件选择时，browser 文件字段要求手动路径', async () => {
    vi.stubGlobal('window', {})
    const onBrowserPick = vi.fn()
    await expect(pickPath('file', { onBrowserPick })).rejects.toThrow('文件路径请手动填写')
    expect(onBrowserPick).not.toHaveBeenCalled()
    expect(H.open).not.toHaveBeenCalled()
  })

  it('embedded 动态调用 Tauri dialog，并保留取消语义', async () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke: vi.fn() } })
    H.open.mockResolvedValue('/picked/dir')
    await expect(pickPath('directory')).resolves.toBe('/picked/dir')
    expect(H.open).toHaveBeenCalledWith({ directory: true, multiple: false, title: '选择目录' })

    H.open.mockResolvedValue(null)
    await expect(pickPath('directory')).resolves.toBeNull()
  })
})

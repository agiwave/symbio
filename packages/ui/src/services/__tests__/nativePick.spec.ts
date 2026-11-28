import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { pickPath } from '../nativePick'

beforeEach(() => { vi.clearAllMocks() })
afterEach(() => { vi.unstubAllGlobals() })

describe('统一路径取值（三端统一走 Server 目录 API）', () => {
  it('目录请求交给 DirectoryPicker，不打开任何本机文件对话框', async () => {
    vi.stubGlobal('window', {})
    const onBrowserPick = vi.fn()

    await expect(pickPath('directory', { initialPath: '/srv', onBrowserPick })).resolves.toBeNull()
    expect(onBrowserPick).toHaveBeenCalledWith({ kind: 'directory', initialPath: '/srv' })
  })

  it('未提供组件回调时明确报错，不静默降级', async () => {
    vi.stubGlobal('window', {})
    await expect(pickPath('directory')).rejects.toThrow('DirectoryPicker')
  })

  it('文件路径字段要求手动填写（浏览器拿不到绝对路径）', async () => {
    vi.stubGlobal('window', {})
    const onBrowserPick = vi.fn()
    await expect(pickPath('file', { onBrowserPick })).rejects.toThrow('文件路径请手动填写')
    expect(onBrowserPick).not.toHaveBeenCalled()
  })

  it('不支持的 pick 种类返回 null，不抛错', async () => {
    await expect(pickPath(undefined)).resolves.toBeNull()
    await expect(pickPath('unknown' as never)).resolves.toBeNull()
  })
})

// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const H = vi.hoisted(() => ({
  getHostRuntime: vi.fn(),
  listHostDirectories: vi.fn(),
}))

vi.mock('@/services/hostDirectories', () => ({
  getHostRuntime: H.getHostRuntime,
  listHostDirectories: H.listHostDirectories,
}))

import DirectoryPicker from '../DirectoryPicker.vue'
import directoryPickerSource from '../DirectoryPicker.vue?raw'

function listing(path = '/srv/home/me') {
  return {
    path,
    parent: '/srv',
    roots: [{ name: '/', path: '/' }],
    entries: [{ name: 'projects', path: '/srv/home/me/projects', kind: 'directory' as const }],
  }
}

function mountPicker(initialPath = '', recentPaths: string[] = []) {
  return mount(DirectoryPicker, {
    props: { visible: true, initialPath, recentPaths },
  })
}

beforeEach(() => {
  H.getHostRuntime.mockReset()
  H.listHostDirectories.mockReset()
  H.getHostRuntime.mockResolvedValue({
    homedir: '/srv/home/me',
    bootstrap_path: '/srv/home/me/.symbio_bootstrap',
    deployment: 'server',
    capabilities: { directory_browser: true },
  })
  H.listHostDirectories.mockResolvedValue(listing())
})

describe('DirectoryPicker', () => {
  it('语义面板根规则显式跨组件，宽度与小视口高度约束才能命中 BaseModal', () => {
    const w = mountPicker('/srv/home/me')
    expect(directoryPickerSource).toContain(':global(.modal-panel.directory-picker) {')
    expect(w.get('.modal-panel.directory-picker').classes()).toContain('directory-picker')
  })

  it('无初始绝对路径时从 Server runtime homedir 开始，并显示根目录', async () => {
    const w = mountPicker('local-shell-path')
    await flushPromises()

    expect(H.getHostRuntime).toHaveBeenCalled()
    expect(H.listHostDirectories).toHaveBeenCalledWith('/srv/home/me')
    // 当前路径只在输入框显示一次，不再另起一行重复一份
    expect((w.get('.path-input').element as HTMLInputElement).value).toBe('/srv/home/me')
    expect(w.find('.root-btn').text()).toBe('/')
    expect(w.find('.directory-entry').text()).toContain('projects')
    // 条目只显示末段名称，完整路径留在 title 上供悬停核对
    expect(w.find('.directory-entry').text()).not.toContain('/srv/home/me/projects')
    expect(w.find('.directory-entry').attributes('title')).toBe('/srv/home/me/projects')
  })

  it('绝对初始路径直接请求 Server 目录；进入子目录与上一步都带 Server 路径', async () => {
    const w = mountPicker('/srv/projects')
    await flushPromises()
    expect(H.getHostRuntime).not.toHaveBeenCalled()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('/srv/projects')

    await w.find('.directory-entry').trigger('click')
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('/srv/home/me/projects')

    await w.find('.toolbar .btn').trigger('click')
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('/srv')
  })

  it('仅在选择当前已加载目录后 emit select', async () => {
    const w = mountPicker('/srv/home/me')
    await flushPromises()
    await w.find('.btn-primary').trigger('click')
    expect(w.emitted('select')).toEqual([['/srv/home/me']])
  })

  it('加载错误可见且禁止选择，可重试', async () => {
    H.listHostDirectories.mockRejectedValueOnce(new Error('Server 目录不可达'))
    const w = mountPicker('/srv/home/me')
    await flushPromises()

    expect(w.find('[role="alert"]').text()).toContain('Server 目录不可达')
    expect(w.find('.btn-primary').attributes('disabled')).toBeDefined()
    await w.find('.retry-btn').trigger('click')
    await flushPromises()
    expect((w.get('.path-input').element as HTMLInputElement).value).toBe('/srv/home/me')
    expect(w.find('.btn-primary').attributes('disabled')).toBeUndefined()
  })

  it('可直接粘贴绝对路径或 ~/ 路径前往，非法相对路径在原地说明', async () => {
    const w = mountPicker('/srv/home/me')
    await flushPromises()
    const input = w.get('.path-input')
    await input.setValue('projects')
    await w.find('.path-form').trigger('submit')
    expect(w.get('[role="alert"]').text()).toContain('完整路径')
    expect(H.listHostDirectories).toHaveBeenCalledTimes(1)

    await input.setValue('~/projects')
    await w.find('.path-form').trigger('submit')
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('~/projects')

    await input.setValue('C:\\work\\project')
    await w.find('.path-form').trigger('submit')
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('C:\\work\\project')
  })

  it('最近目录可一键前往，不必从根逐层浏览', async () => {
    const w = mountPicker('/srv/home/me', ['/srv/projects', '/opt/repo'])
    await flushPromises()
    expect(w.findAll('.recent .root-btn')).toHaveLength(2)
    await w.findAll('.recent .root-btn')[1].trigger('click')
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('/opt/repo')
  })

  it('取消只发 close，不提交路径', async () => {
    const w = mountPicker('/srv/home/me')
    await flushPromises()
    await w.findAll('.footer-buttons, .picker-footer .btn')[0].trigger('click')
    expect(w.emitted('close')).toHaveLength(1)
    expect(w.emitted('select')).toBeUndefined()
  })
})

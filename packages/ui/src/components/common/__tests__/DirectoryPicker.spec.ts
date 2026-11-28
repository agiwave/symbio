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
  // 首次调用（无 path）返回根目录列表；后续按路径返回子节点
  H.listHostDirectories.mockImplementation(async (path?: string) =>
    path ? listing(path) : { path: '', parent: null, roots: [{ name: '/', path: '/' }], entries: [] },
  )
})

describe('DirectoryPicker — 目录树', () => {
  it('语义面板根规则显式跨组件，宽度与小视口高度约束才能命中 BaseModal', () => {
    const w = mountPicker('/srv/home/me')
    expect(directoryPickerSource).toContain(':global(.modal-panel.directory-picker) {')
    expect(w.get('.modal-panel.directory-picker').classes()).toContain('directory-picker')
  })

  it('打开时从服务加载根节点，只显示文件夹名，完整路径在 title', async () => {
    const w = mountPicker('/srv/home/me')
    await flushPromises()

    expect(H.getHostRuntime).toHaveBeenCalled()
    expect(H.listHostDirectories).toHaveBeenCalledWith()
    expect(w.find('.tree-root').exists()).toBe(true)
    const node = w.find('.node-label')
    expect(node.find('.node-name').text()).toBe('/')
    expect(node.attributes('title')).toBe('/')
    // 初始路径作为选中态，完整路径只在底部选中条出现一次
    expect(w.find('.selection-path').text()).toBe('/srv/home/me')
  })

  it('点箭头懒加载子目录；重复折叠/展开不再请求', async () => {
    const w = mountPicker('/srv/home/me')
    await flushPromises()

    // 打开时只有「无参」那一次调用
    expect(H.listHostDirectories).toHaveBeenCalledTimes(1)

    await w.find('.twisty').trigger('click')
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenLastCalledWith('/')
    // 根节点 "/" 展开后载入 listing.entries 里的 projects
    expect(w.findAll('.node-name').map((n) => n.text())).toContain('projects')

    await w.find('.twisty').trigger('click') // 折叠
    await w.find('.twisty').trigger('click') // 再展开（走缓存）
    await flushPromises()
    expect(H.listHostDirectories).toHaveBeenCalledTimes(2) // 无参 + "/" 各一次
  })

  it('点节点选中；双击节点即确认，emit select 带该节点路径', async () => {
    const w = mountPicker('')
    await flushPromises()
    await w.find('.node-label').trigger('click')
    expect(w.find('.selection-path').text()).toBe('/')
    await w.find('.node-label').trigger('dblclick')
    expect(w.emitted('select')).toEqual([['/']])
  })

  it('子节点选中后，底部选择按钮 emit 该子路径', async () => {
    const w = mountPicker('')
    await flushPromises()
    await w.find('.twisty').trigger('click')
    await flushPromises()
    await w.findAll('.node-label')[1].trigger('click')
    expect(w.find('.selection-path').text()).toBe('/srv/home/me/projects')
    await w.find('.btn-primary').trigger('click')
    expect(w.emitted('select')).toEqual([['/srv/home/me/projects']])
  })

  it('加载根失败可见且禁止选择，可重试', async () => {
    H.listHostDirectories.mockRejectedValueOnce(new Error('Server 目录不可达'))
    const w = mountPicker('/srv/home/me')
    await flushPromises()

    expect(w.find('[role="alert"]').text()).toContain('Server 目录不可达')
    expect(w.find('.btn-primary').attributes('disabled')).toBeDefined()
    await w.find('.retry-btn').trigger('click')
    await flushPromises()
    expect(w.find('.tree-root').exists()).toBe(true)
    expect(w.find('.btn-primary').attributes('disabled')).toBeUndefined()
  })

  it('节点展开失败在节点旁给出错误（不炸整棵树）', async () => {
    H.listHostDirectories.mockImplementationOnce(async () => ({
      path: '', parent: null, roots: [{ name: '/', path: '/' }], entries: [],
    }))
    H.listHostDirectories.mockRejectedValueOnce(new Error('读不了'))
    const w = mountPicker('')
    await flushPromises()
    await w.find('.twisty').trigger('click')
    await flushPromises()
    expect(w.find('.node-error').text()).toContain('读不了')
    expect(w.find('.tree-root').exists()).toBe(true)
  })

  it('最近使用一键选中', async () => {
    const w = mountPicker('/srv/home/me', ['/srv/projects', '/opt/repo'])
    await flushPromises()
    expect(w.findAll('.recent .root-btn')).toHaveLength(2)
    await w.findAll('.recent .root-btn')[1].trigger('click')
    expect(w.find('.selection-path').text()).toBe('/opt/repo')
  })

  it('无初始路径时选择按钮禁用；取消只发 close', async () => {
    const w = mountPicker('')
    await flushPromises()
    expect(w.find('.btn-primary').attributes('disabled')).toBeDefined()
    await w.findAll('.picker-footer .btn')[0].trigger('click')
    expect(w.emitted('close')).toHaveLength(1)
    expect(w.emitted('select')).toBeUndefined()
  })
})

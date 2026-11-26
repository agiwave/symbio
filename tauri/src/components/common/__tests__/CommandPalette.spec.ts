// @vitest-environment happy-dom
/**
 * 命令面板组件单测（happy-dom）
 *
 * 分组与排序的判据在 `utils/__tests__/palette.spec.ts`，这里只钉组件这一层的四件事：
 * - **取的时机**：每次打开取一次（不缓存刚建好的那一项），关着时一次都不取；
 * - **键盘**：↑↓ 移动高亮并回环，Enter 前往高亮那一条并收面板；
 * - **地址口径**：推出去的是**浏览器地址**（`/vdfs/<rel>`），不是数据地址（`<根>/<rel>`）
 *   —— 直接把 `@vfs/session/abc` 塞进 URL 会开出一个谁也不认识的空页面；
 * - **截断要说出来**：后端说没取完，界面就不能把「没有匹配」演成「不存在」。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { VdfsItem } from '@/schemas/vdfs'

const hoisted = vi.hoisted(() => ({
  push: vi.fn(),
  treeVdfs: vi.fn(),
  requestNewResource: vi.fn(),
}))

vi.mock('vue-router', () => ({ useRouter: () => ({ push: hoisted.push }) }))
vi.mock('@/services/vdfs', () => ({ treeVdfs: hoisted.treeVdfs }))
vi.mock('@/utils/newResource', () => ({ requestNewResource: hoisted.requestNewResource }))

import CommandPalette from '../CommandPalette.vue'
import { setVdfsRoot } from '@/schemas/vdfsRoot'
import { READBACK_REASON } from '@/services/readback'
import type { PaletteEntry } from '@/utils/palette'

setVdfsRoot('@vfs')

function node(over: Partial<VdfsItem> & { path: string; name: string }): VdfsItem {
  return { title: over.name, kind: 'dir', status: 'active', access: 'lt', ...over } as VdfsItem
}

const CATEGORIES: VdfsItem[] = [
  node({
    path: '@vfs/session',
    name: 'session',
    title: '会话',
    new_type: { ext: 'session', title: '会话' },
  }),
  node({ path: '@vfs/model', name: 'model', title: '模型', new_type: { ext: 'model', title: '模型' } }),
  node({
    path: '@vfs/plugin_manager',
    name: 'plugin_manager',
    title: '插件管理',
    new_type: { ext: 'market', title: '添加插件' },
  }),
]

const APPEARANCE: PaletteEntry = {
  key: 'settings:appearance',
  title: '外观与提示音',
  hint: '主题、字体大小、会话结束提示音',
  keywords: ['外观', '主题', '字体', '提示音', '声音', '设置'],
  addr: '@vfs/plugin_manager',
  select: 'appearance',
}

const ITEMS: VdfsItem[] = [
  ...CATEGORIES,
  node({ path: '@vfs/session/abc', name: 'abc', title: 'abc 报表' }),
  node({ path: '@vfs/session/s1', name: 's1', title: '登录问题' }),
  node({ path: '@vfs/model/gpt', name: 'gpt', title: 'GPT-4o' }),
]

function arrange(
  resp: { nodes: VdfsItem[]; truncated?: boolean } = { nodes: ITEMS },
  props: { commands?: PaletteEntry[] } = {},
) {
  hoisted.treeVdfs.mockResolvedValue({ nodes: resp.nodes, truncated: resp.truncated ?? false })
  return mount(CommandPalette, { props: { visible: true, ...props } })
}

beforeEach(() => {
  hoisted.push.mockClear()
  hoisted.treeVdfs.mockReset()
  hoisted.requestNewResource.mockClear()
})

describe('取清单的时机', () => {
  it('★ 打开取一次：`vdfs/tree` 带面板这个理由与两层深度', async () => {
    arrange()
    await flushPromises()
    expect(hoisted.treeVdfs).toHaveBeenCalledTimes(1)
    expect(hoisted.treeVdfs).toHaveBeenCalledWith(
      READBACK_REASON.PALETTE_SEARCH,
      '@vfs',
      { depth: 2 },
      expect.any(Function),
    )
  })

  it('关着的时候一次都不取（面板不替用户做后台轮询）', async () => {
    hoisted.treeVdfs.mockResolvedValue({ nodes: [], truncated: false })
    mount(CommandPalette, { props: { visible: false } })
    await flushPromises()
    expect(hoisted.treeVdfs).not.toHaveBeenCalled()
  })

  it('重新打开 ⇒ 再取一次（刚建好的那一项必须能被看见）', async () => {
    const w = arrange()
    await flushPromises()
    await w.setProps({ visible: false })
    await w.setProps({ visible: true })
    await flushPromises()
    expect(hoisted.treeVdfs).toHaveBeenCalledTimes(2)
  })

  it('后端取不到 ⇒ 服务层降级成空清单，面板照常可关（不抛一张坏面板）', async () => {
    const w = arrange({ nodes: [] })
    await flushPromises()
    expect(w.find('.pa-note').text()).toContain('没有可导航的类别')
  })
})

describe('呈现与导航', () => {
  it('★ 空词同时给新建与类别导航；标题不泄漏内部 name', async () => {
    const w = arrange()
    await flushPromises()
    expect(w.findAll('.pa-title').map((r) => r.text())).toEqual([
      '会话',
      '模型',
      '添加插件',
      '会话',
      '模型',
      '插件管理',
    ])
    expect(w.findAll('.pa-hint').slice(0, 3).map((r) => r.text())).toEqual(['新建', '新建', '新建'])
    expect(w.text()).not.toContain('plugin_manager')
  })

  it('打字可命中类别导航；导航只 push，不触发新建请求', async () => {
    const w = arrange()
    await flushPromises()
    await w.find('input').setValue('插件')
    expect(w.findAll('.pa-title').map((r) => r.text())).toEqual(['添加插件', '插件管理'])

    await w.findAll('.pa-row')[1].trigger('click')
    await flushPromises()
    expect(hoisted.push).toHaveBeenCalledWith('/vdfs/plugin_manager')
    expect(hoisted.requestNewResource).not.toHaveBeenCalled()
  })

  it('新建命令用 new_type.title，到站后显式请求新建', async () => {
    const w = arrange()
    await flushPromises()
    const add = w.findAll('.pa-row').find((r) => r.find('.pa-title').text() === '添加插件')!
    await add.trigger('click')
    await flushPromises()
    expect(hoisted.push).toHaveBeenCalledWith('/vdfs/plugin_manager')
    expect(hoisted.requestNewResource).toHaveBeenCalledWith('@vfs/plugin_manager')
  })

  it('设置命令保留搜索别名与目录内选中项，也只导航', async () => {
    const w = arrange(undefined, { commands: [APPEARANCE] })
    await flushPromises()
    await w.find('input').setValue('主题')
    expect(w.findAll('.pa-title').map((r) => r.text())).toEqual(['外观与提示音'])
    await w.find('.pa-row').trigger('click')
    await flushPromises()
    expect(hoisted.push).toHaveBeenCalledWith({
      path: '/vdfs/plugin_manager',
      query: { select: 'appearance' },
    })
    expect(hoisted.requestNewResource).not.toHaveBeenCalled()
  })

  it('★ Enter 前往高亮那一条：推浏览器地址、发新建请求并收面板', async () => {
    const w = arrange()
    await flushPromises()

    await w.find('input').trigger('keydown.down') // 从「会话」新建让到「模型」新建
    await w.find('input').trigger('keydown.enter')
    await flushPromises()

    expect(hoisted.push, 'URL 里必须是 /vdfs/<rel>，把 @vfs/… 拼进去会开一张空白页').toHaveBeenCalledWith(
      '/vdfs/model',
    )
    expect(hoisted.requestNewResource).toHaveBeenCalledWith('@vfs/model')
    expect(w.emitted('close')).toBeTruthy()
  })

  it('↑↓ 回环：到顶再往上回到最后一格', async () => {
    const w = arrange()
    await flushPromises()
    const active = () => w.findAll('.pa-row').findIndex((r) => r.classes().includes('is-active'))

    expect(active()).toBe(0)
    await w.find('input').trigger('keydown.up')
    expect(active()).toBe(w.findAll('.pa-row').length - 1)
    await w.find('input').trigger('keydown.down')
    expect(active()).toBe(0)
  })

  it('资源结果只导航，打字后仍能直达浏览器地址', async () => {
    const w = arrange()
    await flushPromises()
    await w.find('input').setValue('abc')
    expect(w.findAll('.pa-title').map((r) => r.text())).toEqual(['abc 报表'])
    await w.find('.pa-row').trigger('click')
    await flushPromises()
    expect(hoisted.push).toHaveBeenCalledWith('/vdfs/session/abc')
    expect(hoisted.requestNewResource).not.toHaveBeenCalled()
  })

  it('★ 后端说没取完 ⇒ 面板把「结果可能不全」说出来', async () => {
    const w = arrange({ nodes: CATEGORIES, truncated: true })
    await flushPromises()
    expect(w.find('.pa-warn').text()).toContain('结果可能不全')
  })

  it('本地每页上限照样给出截断提示', async () => {
    const many = [
      ...CATEGORIES,
      ...Array.from({ length: 13 }, (_, i) =>
        node({ path: `@vfs/session/x${i}`, name: `x${i}`, title: `x${i}` }),
      ),
    ]
    const w = arrange({ nodes: many })
    await flushPromises()
    await w.find('input').setValue('x')
    expect(w.findAll('.pa-row')).toHaveLength(12)
    expect(w.find('.pa-warn').text()).toContain('仅显示前 12 条')
  })
})

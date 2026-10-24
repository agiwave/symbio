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
}))

vi.mock('vue-router', () => ({ useRouter: () => ({ push: hoisted.push }) }))
vi.mock('@/services/vdfs', () => ({ treeVdfs: hoisted.treeVdfs }))

import CommandPalette from '../CommandPalette.vue'
import { setVdfsRoot } from '@/schemas/vdfsRoot'
import { READBACK_REASON } from '@/services/readback'

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
]

const ITEMS: VdfsItem[] = [
  ...CATEGORIES,
  node({ path: '@vfs/session/abc', name: 'abc', title: 'abc 报表' }),
  node({ path: '@vfs/session/s1', name: 's1', title: '登录问题' }),
  node({ path: '@vfs/model/gpt', name: 'gpt', title: 'GPT-4o' }),
]

function arrange(resp: { nodes: VdfsItem[]; truncated?: boolean } = { nodes: ITEMS }) {
  hoisted.treeVdfs.mockResolvedValue({ nodes: resp.nodes, truncated: resp.truncated ?? false })
  return mount(CommandPalette, { props: { visible: true } })
}

beforeEach(() => {
  hoisted.push.mockClear()
  hoisted.treeVdfs.mockReset()
})

describe('取清单的时机', () => {
  it('★ 打开取一次：`vdfs/tree` 带面板这个理由与两层深度', async () => {
    arrange()
    await flushPromises()
    expect(hoisted.treeVdfs).toHaveBeenCalledTimes(1)
    expect(hoisted.treeVdfs).toHaveBeenCalledWith(READBACK_REASON.PALETTE_SEARCH, '@vfs', { depth: 2 })
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
    expect(w.find('.pa-note').text()).toContain('没有可新建的类别')
  })
})

describe('呈现与导航', () => {
  it('★ 空词给动作组（「新建会话」），打字后给命中的条目', async () => {
    const w = arrange()
    await flushPromises()
    expect(w.text()).toContain('新建会话')

    await w.find('input').setValue('abc')
    // 只比主标题：hint（地址相对段）的取值由 palette.spec 钉，这里钉的是「命中了哪几条」
    expect(w.findAll('.pa-title').map((r) => r.text())).toEqual(['abc 报表'])
  })

  it('★ Enter 前往高亮那一条：推**浏览器地址**而不是数据地址，并关掉面板', async () => {
    const w = arrange()
    await flushPromises()

    await w.find('input').trigger('keydown.down') // 从「新建会话」让到「新建模型」
    await w.find('input').trigger('keydown.enter')

    expect(hoisted.push, 'URL 里必须是 /vdfs/<rel>，把 @vfs/… 拼进去会开一张空白页').toHaveBeenCalledWith(
      '/vdfs/model',
    )
    expect(w.emitted('close')).toBeTruthy()
  })

  it('↑↓ 回环：到顶再往上回到最后一格', async () => {
    const w = arrange()
    await flushPromises()
    const active = () => w.findAll('.pa-row').findIndex((r) => r.classes().includes('is-active'))

    expect(active()).toBe(0)
    await w.find('input').trigger('keydown.up')
    expect(active()).toBe(1) // 最后一格
    await w.find('input').trigger('keydown.down')
    expect(active()).toBe(0)
  })

  it('点一行 = 前往那一行', async () => {
    const w = arrange()
    await flushPromises()
    await w.findAll('.pa-row')[1].trigger('click')
    expect(hoisted.push).toHaveBeenCalledWith('/vdfs/model')
  })

  it('★ 后端说没取完 ⇒ 面板把「结果可能不全」说出来', async () => {
    const w = arrange({ nodes: CATEGORIES, truncated: true })
    await flushPromises()
    expect(w.find('.pa-warn').text()).toContain('结果可能不全')
  })
})

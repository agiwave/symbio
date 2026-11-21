// @vitest-environment happy-dom
/**
 * MarketPanel — 插件市场的页面层判据
 *
 * 市场没有第二份插件清单：候选来自节点正文，安装写回同一节点。这两条链一断，
 * 「市场」就只剩一个空壳，因此测试直接钉住「正文 → 卡片」与「点击 → 写回」。
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import MarketPanel from '../MarketPanel.vue'
import type { VdfsItem } from '@/schemas/vdfs'

const mocks = vi.hoisted(() => ({ writeVdfs: vi.fn() }))
vi.mock('@/services/vdfs', () => ({ writeVdfs: mocks.writeVdfs }))

function marketItem(): VdfsItem {
  return {
    path: 'plugin_manager/market',
    name: 'market',
    title: '插件市场',
    kind: 'plugin_manager',
    status: '',
    access: 'rw',
    ext: 'market',
  }
}

function body(candidates: unknown[]): string {
  return JSON.stringify({ candidates })
}

const candidate = (name: string) => ({
  name,
  provider: name,
  source: 'builtin',
  label: `${name} 插件`,
})

describe('MarketPanel', () => {
  beforeEach(() => {
    mocks.writeVdfs.mockReset()
    mocks.writeVdfs.mockResolvedValue({ name: 'slo', created: true })
  })

  it('把候选正文渲染成卡片：名称、来源与安装按钮', () => {
    const w = mount(MarketPanel, {
      props: { node: marketItem(), data: body([candidate('slo'), candidate('setting')]) },
    })
    expect(w.findAll('.market-card')).toHaveLength(2)
    expect(w.text()).toContain('slo 插件')
    expect(w.text()).toContain('内置可选')
    expect(w.findAll('.install-btn')).toHaveLength(2)
  })

  it('点击安装写回市场节点，载荷只带插件工厂', async () => {
    const w = mount(MarketPanel, {
      props: { node: marketItem(), data: body([candidate('slo')]) },
    })
    await w.find('.install-btn').trigger('click')

    expect(mocks.writeVdfs).toHaveBeenCalledTimes(1)
    const [path, text] = mocks.writeVdfs.mock.calls[0]
    expect(path).toBe('plugin_manager/market')
    expect(JSON.parse(text)).toEqual({ plugin_provider: 'slo' })
    expect(w.text()).toContain('暂无可安装的可选插件')
  })

  it('安装失败时把原因留在页面上，不静默', async () => {
    mocks.writeVdfs.mockRejectedValueOnce(new Error('本构建未链接该工厂'))
    const w = mount(MarketPanel, {
      props: { node: marketItem(), data: body([candidate('slo')]) },
    })
    await w.find('.install-btn').trigger('click')
    await Promise.resolve()

    expect(w.find('.market-error').text()).toBe('本构建未链接该工厂')
    expect(w.findAll('.market-card')).toHaveLength(1)
  })

  it('空候选给出空态而不是空白页', () => {
    const w = mount(MarketPanel, {
      props: { node: marketItem(), data: body([]) },
    })
    expect(w.text()).toContain('暂无可安装的可选插件')
  })
})

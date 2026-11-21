// @vitest-environment happy-dom
/**
 * MarketPanel — 插件市场的页面层判据
 *
 * 市场没有第二份插件清单：候选随「添加插件」的 `new_type.schema` 作为 `data` 下发，
 * 安装只抛 `save` 载荷，写入由宿主按草稿规则完成。这两条链一断，市场就只剩空壳，
 * 因此测试直接钉住「schema → 卡片」与「点击 → save 载荷」。
 */

import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import MarketPanel from '../MarketPanel.vue'
import type { VdfsItem } from '@/schemas/vdfs'

/** 草稿节点：没有落盘路径——市场页据此证明自己不直接写 VDFS */
function marketDraft(): VdfsItem {
  return {
    path: '',
    name: '',
    title: '',
    kind: 'market',
    status: '',
    access: 'w',
    ext: 'market',
  }
}

const candidate = (name: string) => ({
  name,
  provider: name,
  source: 'builtin',
  label: `${name} 插件`,
})

// 真实链路里 provider_field 由后端唯一定义并随 schema 下发（VDFS_PLUGIN_PROVIDER_FIELD）。
const PROVIDER_FIELD = 'provider'
const data = (candidates: unknown[], providerField: string = PROVIDER_FIELD) => ({
  candidates,
  provider_field: providerField,
})

describe('MarketPanel', () => {
  it('把 schema 候选渲染成卡片：名称、来源与安装按钮', () => {
    const w = mount(MarketPanel, {
      props: { node: marketDraft(), data: data([candidate('demo'), candidate('setting')]) },
    })
    expect(w.findAll('.market-card')).toHaveLength(2)
    expect(w.text()).toContain('demo 插件')
    expect(w.text()).toContain('内置可选')
    expect(w.findAll('.install-btn')).toHaveLength(2)
  })

  it('点击安装只抛 save 载荷，不自己写 VDFS', async () => {
    const w = mount(MarketPanel, {
      props: { node: marketDraft(), data: data([candidate('demo')]) },
    })
    await w.find('.install-btn').trigger('click')

    expect(w.emitted('save')).toEqual([[{ provider: 'demo' }]])
  })

  it('安装载荷的键名来自后端下发的 provider_field，不硬编码', async () => {
    const w = mount(MarketPanel, {
      props: { node: marketDraft(), data: data([candidate('demo')], 'plugin_provider') },
    })
    await w.find('.install-btn').trigger('click')

    // 键名完全跟随后端定义——后端若改用别的键，前端无需改一行即可跟上。
    expect(w.emitted('save')).toEqual([[{ plugin_provider: 'demo' }]])
  })

  it('安装失败时把宿主的 error 留在页面上，不静默', () => {
    const w = mount(MarketPanel, {
      props: {
        node: marketDraft(),
        data: data([candidate('demo')]),
        error: '本构建未链接该工厂',
      },
    })
    expect(w.find('.market-error').text()).toBe('本构建未链接该工厂')
    expect(w.findAll('.market-card')).toHaveLength(1)
  })

  it('空候选给出空态而不是空白页', () => {
    const w = mount(MarketPanel, {
      props: { node: marketDraft(), data: data([]) },
    })
    expect(w.text()).toContain('暂无可安装的可选插件')
  })
})

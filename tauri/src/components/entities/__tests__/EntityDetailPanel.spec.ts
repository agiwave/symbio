// @vitest-environment happy-dom
/**
 * EntityDetailPanel — 组件测试（happy-dom）
 *
 * 覆盖：名称（ID）区块、extra 扩展区不重复展示保留字段、
 * 空态。协议资源路径（[provider]/[id].[kind]）属纯技术概念，
 * 产品化修正后不再展示（也不可复制）。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import EntityDetailPanel from '../EntityDetailPanel.vue'
import type { EntitySummary } from '../../../schemas/entities'

function makeItem(overrides: Partial<EntitySummary> = {}): EntitySummary {
  return {
    kind: 'model',
    name: 'OpenAI',
    id: 'openai',
    status: 'active',
    ...overrides,
  } as EntitySummary
}

describe('EntityDetailPanel', () => {
  it('渲染名称（ID）区块（provider 回退 kind 场景同样成立）', () => {
    const wrapper = mount(EntityDetailPanel, {
      props: { item: makeItem() },
    })
    expect(wrapper.text()).toContain('名称（ID）')
    expect(wrapper.text()).toContain('openai')
  })

  it('不展示协议资源路径（技术概念不入产品界面）', () => {
    const wrapper = mount(EntityDetailPanel, {
      props: { item: makeItem({ provider: 'worker' }) },
    })
    expect(wrapper.text()).not.toContain('实体路径')
    expect(wrapper.text()).not.toContain('worker/openai.model')
    expect(wrapper.text()).not.toContain('复制')
  })

  it('extra 扩展区不重复展示 provider / kind 等保留字段', () => {
    const wrapper = mount(EntityDetailPanel, {
      props: {
        item: makeItem({ provider: 'model', extra_field: 'x' } as Partial<EntitySummary>),
      },
    })
    const labels = wrapper.findAll('.detail-section label').map((l) => l.text())
    expect(labels).not.toContain('provider')
    expect(labels).not.toContain('kind')
    // 未保留的扩展字段正常展示
    expect(labels).toContain('extra_field')
  })

  it('item 为 null 时渲染空态提示', () => {
    const wrapper = mount(EntityDetailPanel, { props: { item: null } })
    expect(wrapper.text()).toContain('选择一个实体查看详情')
  })
})

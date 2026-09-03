// @vitest-environment happy-dom
/**
 * ResourceDetailPanel — 组件测试（happy-dom）
 *
 * 覆盖：资源路径区块渲染（provider 字段 / kind 兜底）、
 * extra 扩展区不重复展示 provider 字段。
 */
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import ResourceDetailPanel from '../ResourceDetailPanel.vue'
import type { ResourceSummary } from '../../../schemas/resources'

function makeItem(overrides: Partial<ResourceSummary> = {}): ResourceSummary {
  return {
    kind: 'model',
    name: 'OpenAI',
    id: 'openai',
    status: 'active',
    ...overrides,
  } as ResourceSummary
}

describe('ResourceDetailPanel 资源路径', () => {
  it('渲染名称（ID）与资源路径区块（provider 回退 kind）', () => {
    const wrapper = mount(ResourceDetailPanel, {
      props: { item: makeItem() },
    })
    expect(wrapper.text()).toContain('名称（ID）')
    expect(wrapper.text()).toContain('openai')
    expect(wrapper.text()).toContain('资源路径')
    expect(wrapper.text()).toContain('model/openai.model')
  })

  it('provider 字段存在时用于路径前缀', () => {
    const wrapper = mount(ResourceDetailPanel, {
      props: { item: makeItem({ provider: 'worker' }) },
    })
    expect(wrapper.text()).toContain('worker/openai.model')
  })

  it('extra 扩展区不重复展示 provider / kind 等保留字段', () => {
    const wrapper = mount(ResourceDetailPanel, {
      props: {
        item: makeItem({ provider: 'model', extra_field: 'x' } as Partial<ResourceSummary>),
      },
    })
    const labels = wrapper.findAll('.detail-section label').map((l) => l.text())
    // 路径区块只出现一次
    expect(labels.filter((l) => l === '资源路径')).toHaveLength(1)
    expect(labels).not.toContain('provider')
    expect(labels).not.toContain('kind')
    // 未保留的扩展字段正常展示
    expect(labels).toContain('extra_field')
  })

  it('item 为 null 时渲染空态提示', () => {
    const wrapper = mount(ResourceDetailPanel, { props: { item: null } })
    expect(wrapper.text()).toContain('选择一个资源查看详情')
  })
})

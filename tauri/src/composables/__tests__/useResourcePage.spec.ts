/**
 * useResourcePage / useResourceProviders 核心逻辑单测（node 环境）
 *
 * 覆盖（纯函数，无 Vue 实例）：
 * - resourcePage 混合平排展平 + name 排序
 * - resolveActiveTypes：all/单类型/逗号/未知过滤/supports_upload 过滤
 * - isManagerCreatable：supports_upload 与 mutable 解耦
 */
import { describe, expect, it } from 'vitest'
import { buildMixedItems, isManagerCreatable } from '../useResourcePage'
import { resolveActiveTypes } from '../useResourceProviders'
import type { ProviderInfo, ResourceSummary } from '@/schemas/resources'

const CAP = {
  zip_upload: false,
  independent_form: false,
  realtime_status: false,
  mutable: true,
  test_connection: false,
  read_only: false,
}

function provider(kind: string, order: number, supports_upload: boolean): ProviderInfo {
  return {
    kind,
    provider_name: kind,
    prefix: kind,
    capabilities: CAP,
    order,
    label: kind,
    supports_upload,
  }
}

function item(kind: string, id: string, name?: string): ResourceSummary {
  return { kind, id, name: name ?? id, status: 'active' } as ResourceSummary
}

// ============ buildMixedItems ============
describe('buildMixedItems 混合平排', () => {
  it('展平所有类型、按 name 自然排序（目录式交错）', () => {
    const active = [provider('model', 1, true), provider('mcp', 2, true)]
    const states = {
      model: { items: [item('model', 'b', 'Beta'), item('model', 'a', 'Alpha')], capabilities: CAP },
      mcp: { items: [item('mcp', 'm1', 'Gamma')], capabilities: CAP },
    }
    const out = buildMixedItems(active, states)
    expect(out.map((i) => i.id)).toEqual(['a', 'b', 'm1'])
    expect(out[0].kind).toBe('model')
    expect(out[2].kind).toBe('mcp')
  })

  it('空类型分组被忽略，name 缺失回退 id', () => {
    const active = [provider('model', 1, true), provider('skill', 2, true)]
    const states = {
      model: { items: [item('model', 'z')], capabilities: CAP },
      // skill 无数据
    }
    const out = buildMixedItems(active, states)
    expect(out).toHaveLength(1)
    expect(out[0].id).toBe('z')
  })

  it('单类型时只展平该类型（与分组页等价）', () => {
    const active = [provider('model', 1, true)]
    const states = {
      model: { items: [item('model', 'a'), item('model', 'b')], capabilities: CAP },
      mcp: { items: [item('mcp', 'x')], capabilities: CAP }, // 不在 active
    }
    const out = buildMixedItems(active, states)
    expect(out.map((i) => i.id)).toEqual(['a', 'b'])
  })
})

// ============ resolveActiveTypes ============
describe('resolveActiveTypes', () => {
  const all = [
    provider('model', 1, true),
    provider('mcp', 2, true),
    provider('skill', 3, true),
    provider('agent', 4, true),
    provider('session', 5, false), // supports_upload=false
  ]

  it('undefined / 空 / all → 所有 supports_upload 类型（session 排除）', () => {
    for (const p of [undefined, '', 'all']) {
      const out = resolveActiveTypes(all, p)
      expect(out.map((x) => x.kind)).toEqual(['model', 'mcp', 'skill', 'agent'])
    }
  })

  it('单类型显式可达（含 session）', () => {
    expect(resolveActiveTypes(all, 'session').map((x) => x.kind)).toEqual(['session'])
    expect(resolveActiveTypes(all, 'model').map((x) => x.kind)).toEqual(['model'])
  })

  it('逗号分隔按 order 排序 / 去重', () => {
    expect(resolveActiveTypes(all, 'agent,mcp,model').map((x) => x.kind)).toEqual(['model', 'mcp', 'agent'])
    expect(resolveActiveTypes(all, 'mcp,mcp').map((x) => x.kind)).toEqual(['mcp'])
  })

  it('未知类型过滤；过滤后为空回退 all', () => {
    expect(resolveActiveTypes(all, 'model,foo').map((x) => x.kind)).toEqual(['model'])
    expect(resolveActiveTypes(all, 'foo').map((x) => x.kind)).toEqual(['model', 'mcp', 'skill', 'agent'])
  })
})

// ============ isManagerCreatable ============
describe('isManagerCreatable', () => {
  it('supports_upload 与 mutable 解耦：缺一不可', () => {
    expect(isManagerCreatable({ supports_upload: true, capabilities: CAP })).toBe(true)
    expect(isManagerCreatable({ supports_upload: false, capabilities: CAP })).toBe(false) // session
    expect(
      isManagerCreatable({
        supports_upload: true,
        capabilities: { ...CAP, mutable: false, read_only: true },
      })
    ).toBe(false)
    expect(isManagerCreatable({})).toBe(false)
  })
})
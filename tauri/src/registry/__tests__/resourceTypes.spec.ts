/**
 * 资源类型注册表 — 纯逻辑单测（node 环境）
 *
 * 覆盖：注册表完备性、parseTypesParam 全语义、resourcePath 构造。
 */
import { describe, expect, it } from 'vitest'
import {
  DEFAULT_RESOURCE_TYPES,
  RESOURCE_TYPE_REGISTRY,
  parseTypesParam,
  resourcePath,
} from '../resourceTypes'
import type { ResourceType } from '../../schemas/resources'

describe('RESOURCE_TYPE_REGISTRY 完备性', () => {
  it('每类资源都有 label/provider/prefix/capabilities/order', () => {
    for (const [kind, d] of Object.entries(RESOURCE_TYPE_REGISTRY)) {
      expect(d.kind).toBe(kind)
      expect(d.label.length).toBeGreaterThan(0)
      expect(d.provider.length).toBeGreaterThan(0)
      expect(d.prefix.length).toBeGreaterThan(0)
      expect(d.capabilities).toBeDefined()
      expect(typeof d.order).toBe('number')
    }
  })

  it('五类资源（model/mcp/skill/agent/session）全部注册且 prefix 唯一', () => {
    const kinds = Object.keys(RESOURCE_TYPE_REGISTRY)
    for (const k of ['model', 'mcp', 'skill', 'agent', 'session'] as ResourceType[]) {
      expect(kinds).toContain(k)
    }
    const prefixes = Object.values(RESOURCE_TYPE_REGISTRY).map((d) => d.prefix)
    expect(new Set(prefixes).size).toBe(prefixes.length)
  })

  it('专属表单仅 model 注册（其余走通用兜底）', () => {
    for (const [kind, d] of Object.entries(RESOURCE_TYPE_REGISTRY)) {
      if (kind === 'model') expect(d.form).toBeDefined()
      else expect(d.form).toBeUndefined()
    }
  })

  it('session 的路径前缀为 worker/session', () => {
    expect(RESOURCE_TYPE_REGISTRY.session.prefix).toBe('worker/session')
    expect(RESOURCE_TYPE_REGISTRY.model.prefix).toBe('worker/model')
  })
})

describe('parseTypesParam', () => {
  it('undefined / 空串 / all → DEFAULT_RESOURCE_TYPES 按 order 排序', () => {
    for (const param of [undefined, '', 'all', '  all  ']) {
      const out = parseTypesParam(param)
      expect(out.map((d) => d.kind)).toEqual(DEFAULT_RESOURCE_TYPES)
      expect(out[0].order).toBeLessThan(out[out.length - 1].order)
    }
  })

  it('单类型 → 单元素列表', () => {
    expect(parseTypesParam('model').map((d) => d.kind)).toEqual(['model'])
    expect(parseTypesParam('session').map((d) => d.kind)).toEqual(['session'])
  })

  it('逗号分隔多类型 → 按 order 排序（与传入顺序无关）', () => {
    expect(parseTypesParam('mcp,model').map((d) => d.kind)).toEqual(['model', 'mcp'])
    expect(parseTypesParam('agent,skill,mcp').map((d) => d.kind)).toEqual(['mcp', 'skill', 'agent'])
  })

  it('重复类型去重', () => {
    expect(parseTypesParam('model,model').map((d) => d.kind)).toEqual(['model'])
  })

  it('未知类型过滤；过滤后为空回退 all', () => {
    expect(parseTypesParam('model,foo').map((d) => d.kind)).toEqual(['model'])
    expect(parseTypesParam('foo').map((d) => d.kind)).toEqual(DEFAULT_RESOURCE_TYPES)
    expect(parseTypesParam('foo,bar').map((d) => d.kind)).toEqual(DEFAULT_RESOURCE_TYPES)
  })

  it('空白容忍（trim）', () => {
    expect(parseTypesParam(' model , mcp ').map((d) => d.kind)).toEqual(['model', 'mcp'])
  })
})

describe('resourcePath', () => {
  it('构造 [provider]/[id].[kind]', () => {
    expect(resourcePath('model', 'openai', 'model')).toBe('model/openai.model')
    expect(resourcePath('skill', 'pdf', 'skill')).toBe('skill/pdf.skill')
    expect(resourcePath('mcp', 'filesystem', 'mcp')).toBe('mcp/filesystem.mcp')
  })

  it('provider 与 kind 可以不同（未来插件显示名分叉场景）', () => {
    expect(resourcePath('worker', 'openai', 'model')).toBe('worker/openai.model')
  })
})

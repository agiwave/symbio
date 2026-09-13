/**
 * 实体类型注册表 — 前端展示层纯逻辑单测（node 环境）
 *
 * 注册表不再维护硬编码类型清单/前缀/能力，只做：
 * - editor 组件按 kind 注册
 * - icon 按 kind 注册
 * 资源的存在/能力/寻址来自后端（VDFS 挂载点与节点；实体页已于 S5 下线）。
 */
import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import {
  getEntityEditor,
  getEntityEditorFor,
  getEntityIcon,
  getEntityIconFor,
  registerEntityEditor,
  registerEntityIcon,
} from '../entityTypes'

const Dummy = defineComponent({ template: '<div />' })

describe('registerEntityEditor / getEntityEditor', () => {
  it('model / mcp / skill / agent:bundle 改由后端 detail 定义驱动，前端不再注册', () => {
    // definition-driven detail：注册 editor 缺席时由 DetailForm 渲染
    expect(getEntityEditor('model')).toBeUndefined()
    expect(getEntityEditor('mcp')).toBeUndefined()
    expect(getEntityEditor('skill')).toBeUndefined()
    expect(getEntityEditorFor({ kind: 'agent', config_type: 'bundle' })).toBeUndefined()
  })

  it('setting 已迁移到 VDFS：实体侧不再注册任何分区 editor', () => {
    for (const ext of ['appearance', 'session', 'local', 'web', 'gateway', 'about']) {
      expect(getEntityEditorFor({ kind: 'setting', config_type: ext })).toBeUndefined()
    }
  })

  it('未注册的 kind 返回 undefined（走通用兜底）', () => {
    expect(getEntityEditor('unknown-type')).toBeUndefined()
  })

  it('可动态注册新类型 editor', () => {
    registerEntityEditor('demo-kind', Dummy)
    expect(getEntityEditor('demo-kind')).toBe(Dummy)
  })
})

describe('getEntityEditorFor（项级"扩展名"分发）', () => {
  it('kind:ext 命中项级 editor', () => {
    registerEntityEditor('demo:alpha', Dummy)
    expect(getEntityEditorFor({ kind: 'demo', config_type: 'alpha' })).toBe(Dummy)
  })

  it('config_type 未注册时回退 kind 级 editor', () => {
    registerEntityEditor('demo', Dummy)
    expect(getEntityEditorFor({ kind: 'demo', config_type: 'unknown-ext' })).toBe(Dummy)
  })

  it('无 config_type 的实体走 kind 级查找', () => {
    expect(getEntityEditorFor({ kind: 'session' })).toBeTruthy()
    expect(getEntityEditorFor({ kind: 'mcp' })).toBeUndefined()
  })

  it('非 string 的 config_type 被忽略（后端 extra 兼容）', () => {
    expect(getEntityEditorFor({ kind: 'demo', config_type: 42 })).toBe(
      getEntityEditorFor({ kind: 'demo' })
    )
  })
})

describe('registerEntityIcon / getEntityIcon', () => {
  it('未注册 icon 返回 undefined（走默认图标）', () => {
    expect(getEntityIcon('unknown-type')).toBeUndefined()
  })

  it('内置六大 kind 均注册了独立图标', () => {
    for (const kind of ['model', 'mcp', 'agent', 'skill', 'session', 'setting']) {
      expect(getEntityIcon(kind)).toBeTruthy()
    }
  })

  it('可动态注册 icon', () => {
    registerEntityIcon('mcp', Dummy)
    expect(getEntityIcon('mcp')).toBe(Dummy)
  })
})

describe('getEntityIconFor（项级图标分发）', () => {
  it('setting 各分区有专属图标（VDFS 列表项复用同一套项级图标）', () => {
    for (const ext of ['appearance', 'session', 'local', 'web', 'gateway', 'about']) {
      expect(getEntityIconFor({ kind: 'setting', config_type: ext })).toBeTruthy()
    }
  })

  it('未注册图标的类型返回 undefined', () => {
    expect(getEntityIconFor({ kind: 'unknown-type' })).toBeUndefined()
  })
})
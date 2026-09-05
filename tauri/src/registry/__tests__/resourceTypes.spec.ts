/**
 * 资源类型注册表 — 前端展示层纯逻辑单测（node 环境）
 *
 * 注册表不再维护硬编码类型清单/前缀/能力，只做：
 * - editor 组件按 kind 注册
 * - icon 按 kind 注册
 * 类型的存在/能力/前缀来自后端 ProviderInfo（见 useResourceProviders）。
 */
import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import {
  getResourceEditor,
  getResourceEditorFor,
  getResourceIcon,
  getResourceIconFor,
  registerResourceEditor,
  registerResourceIcon,
} from '../resourceTypes'

const Dummy = defineComponent({ template: '<div />' })

describe('registerResourceEditor / getResourceEditor', () => {
  it('model / 设置三分区改由后端 detail 定义驱动，前端不再注册', () => {
    // definition-driven detail：注册 editor 缺席时由 DetailForm 渲染
    expect(getResourceEditor('model')).toBeUndefined()
    expect(getResourceEditorFor({ kind: 'setting', config_type: 'session' })).toBeUndefined()
    expect(getResourceEditorFor({ kind: 'setting', config_type: 'local' })).toBeUndefined()
    expect(getResourceEditorFor({ kind: 'setting', config_type: 'web' })).toBeUndefined()
  })

  it('未注册的 kind 返回 undefined（走通用兜底）', () => {
    expect(getResourceEditor('unknown-type')).toBeUndefined()
    expect(getResourceEditor('mcp')).toBeUndefined()
  })

  it('可动态注册新类型 editor', () => {
    registerResourceEditor('setting', Dummy)
    expect(getResourceEditor('setting')).toBe(Dummy)
  })
})

describe('getResourceEditorFor（项级"扩展名"分发）', () => {
  it('setting 各分区按 config_type 进入不同 editor（定义驱动的三分区除外）', () => {
    expect(getResourceEditorFor({ kind: 'setting', config_type: 'appearance' })).toBeTruthy()
    expect(getResourceEditorFor({ kind: 'setting', config_type: 'about' })).toBeTruthy()
  })

  it('config_type 未注册时回退 kind 级 editor', () => {
    registerResourceEditor('setting', Dummy)
    expect(getResourceEditorFor({ kind: 'setting', config_type: 'unknown-ext' })).toBe(Dummy)
  })

  it('无 config_type 的资源走 kind 级查找', () => {
    expect(getResourceEditorFor({ kind: 'session' })).toBeTruthy()
    expect(getResourceEditorFor({ kind: 'mcp' })).toBeUndefined()
  })

  it('非 string 的 config_type 被忽略（后端 extra 兼容）', () => {
    expect(getResourceEditorFor({ kind: 'setting', config_type: 42 })).toBe(
      getResourceEditorFor({ kind: 'setting' })
    )
  })
})

describe('registerResourceIcon / getResourceIcon', () => {
  it('未注册 icon 返回 undefined（走默认图标）', () => {
    expect(getResourceIcon('unknown-type')).toBeUndefined()
  })

  it('内置六大 kind 均注册了独立图标', () => {
    for (const kind of ['model', 'mcp', 'agent', 'skill', 'session', 'setting']) {
      expect(getResourceIcon(kind)).toBeTruthy()
    }
  })

  it('可动态注册 icon', () => {
    registerResourceIcon('mcp', Dummy)
    expect(getResourceIcon('mcp')).toBe(Dummy)
  })
})

describe('getResourceIconFor（项级图标分发）', () => {
  it('setting 各分区有专属图标', () => {
    for (const ext of ['appearance', 'session', 'local', 'web', 'about']) {
      expect(getResourceIconFor({ kind: 'setting', config_type: ext })).toBeTruthy()
    }
  })

  it('未注册图标的类型返回 undefined', () => {
    expect(getResourceIconFor({ kind: 'unknown-type' })).toBeUndefined()
  })
})
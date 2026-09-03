/**
 * 资源类型注册表 — 前端展示层纯逻辑单测（node 环境）
 *
 * 注册表不再维护硬编码类型清单/前缀/能力，只做：
 * - editor 组件按 kind 注册
 * - icon 按 kind 注册
 * - resourcePath 构造
 * 类型的存在/能力/前缀来自后端 ProviderInfo（见 useResourceProviders）。
 */
import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import {
  getResourceEditor,
  getResourceIcon,
  registerResourceEditor,
  registerResourceIcon,
  resourcePath,
} from '../resourceTypes'

const Dummy = defineComponent({ template: '<div />' })

describe('registerResourceEditor / getResourceEditor', () => {
  it('model 预注册专属编辑表单', () => {
    expect(getResourceEditor('model')).toBeTruthy()
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

describe('registerResourceIcon / getResourceIcon', () => {
  it('未注册 icon 返回 undefined（走默认图标）', () => {
    expect(getResourceIcon('mcp')).toBeUndefined()
  })

  it('可动态注册 icon', () => {
    registerResourceIcon('mcp', Dummy)
    expect(getResourceIcon('mcp')).toBe(Dummy)
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
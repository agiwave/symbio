/**
 * VDFS 渲染器注册表 — 前端纯 UI 映射单测（node 环境）
 *
 * 覆盖：ext → 渲染器解析（前端选择详情页面的唯一入口）、目录优先、
 * 未命中回退 fallback、组件登记与查取。
 *
 * 注意：本文件只验证「映射」这一纯逻辑；具体组件装配在
 * `registry/vdfsRenderers.ts`（副作用导入），不在此断言组件内部行为。
 */

import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import {
  getVdfsRenderer,
  registerVdfsRenderer,
  resolveVdfsRenderer,
} from '../vdfsTypes'
import type { VdfsNode } from '@/schemas/vdfs'

function node(partial: Partial<VdfsNode> & { name: string }): VdfsNode {
  return {
    path: `/${partial.name}`,
    title: partial.name,
    kind: 'file',
    status: 'active',
    access: 'r',
    ...partial,
  }
}

describe('resolveVdfsRenderer', () => {
  it('目录优先于扩展名（访问位含 l）', () => {
    expect(resolveVdfsRenderer(node({ name: 'prompts.md', access: 'lt' }))).toBe('dir')
  })

  it('按扩展名分发到机制级渲染器', () => {
    expect(resolveVdfsRenderer(node({ name: 'local', ext: 'form' }))).toBe('form')
    expect(resolveVdfsRenderer(node({ name: 'abc', ext: 'session' }))).toBe('session')
    expect(resolveVdfsRenderer(node({ name: 'a.md' }))).toBe('markdown')
    expect(resolveVdfsRenderer(node({ name: 'a.json' }))).toBe('json')
    expect(resolveVdfsRenderer(node({ name: 'a.log' }))).toBe('text')
  })

  it('消息是列表项：ext = message → 专用只读渲染器（不是通用文本编辑器）', () => {
    // 转写列表项的结构在 attributes 里（role / type / status / error），
    // 通用文本渲染器看不见它们，因此必须有自己的渲染器。
    // 同时它是**文本缓冲**（正文即内容），追加型变更可就地拼接。
    expect(resolveVdfsRenderer(node({ name: 'm1', ext: 'message' }))).toBe('message')
    // 未声明 ext 时按文件名推导——消息节点恒显式声明 ext，故这里回退 fallback
    expect(resolveVdfsRenderer(node({ name: 'm1' }))).toBe('fallback')
  })

  it('前端状态自持的专属面板按语义 ext 分发（设置分区）', () => {
    expect(resolveVdfsRenderer(node({ name: 'appearance', ext: 'appearance' }))).toBe('appearance')
    expect(resolveVdfsRenderer(node({ name: 'about', ext: 'about' }))).toBe('about')
  })

  it('未知扩展名 / 无扩展名 → fallback（页面永不空白）', () => {
    expect(resolveVdfsRenderer(node({ name: 'a.bin' }))).toBe('fallback')
    expect(resolveVdfsRenderer(node({ name: 'noext' }))).toBe('fallback')
    expect(resolveVdfsRenderer(null)).toBe('fallback')
  })
})

describe('registerVdfsRenderer / getVdfsRenderer', () => {
  it('登记后可按标识取回组件；未登记返回 undefined', () => {
    const Dummy = defineComponent({ template: '<div />' })
    expect(getVdfsRenderer('json')).toBeUndefined()
    registerVdfsRenderer('json', Dummy)
    expect(getVdfsRenderer('json')).toBe(Dummy)
  })
})

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
  isTextualRenderer,
  registerVdfsRenderer,
  rendererReadsNodeText,
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

/**
 * 两个谓词的**边界**必须逐词锁死：它们此前被写了三遍，且其中一遍其实是另一个集合
 * （`rendererReadsNodeText` ⊃ `isTextualRenderer`）。这类"形状相同但不是同一件事"
 * 的错误只有把每个渲染器逐条断言才拦得住——泛泛地测两个 `true` 是测不出来的。
 */
describe('isTextualRenderer / rendererReadsNodeText', () => {
  it('isTextualRenderer：正文即文本缓冲的四种（追加可安全拼接）', () => {
    for (const r of ['text', 'markdown', 'json', 'message'] as const) {
      expect(isTextualRenderer(r), r).toBe(true)
    }
  })

  it('isTextualRenderer：其余一律不是（表单与二进制不可追加）', () => {
    for (const r of ['dir', 'form', 'session', 'appearance', 'about', 'fallback'] as const) {
      expect(isTextualRenderer(r), r).toBe(false)
    }
  })

  it('rendererReadsNodeText：文本缓冲 ∪ form（表单字段值也取自正文）', () => {
    for (const r of ['text', 'markdown', 'json', 'message', 'form'] as const) {
      expect(rendererReadsNodeText(r), r).toBe(true)
    }
  })

  it('rendererReadsNodeText：其余各有自己的数据来源，替它们读正文是白读', () => {
    for (const r of ['dir', 'session', 'appearance', 'about', 'fallback'] as const) {
      expect(rendererReadsNodeText(r), r).toBe(false)
    }
  })

  it('两者的差集**恰好**是 form —— 这正是它们不能合并的原因', () => {
    const all = [
      'dir',
      'form',
      'session',
      'message',
      'markdown',
      'json',
      'text',
      'appearance',
      'about',
      'fallback',
    ] as const
    const onlyReads = all.filter((r) => rendererReadsNodeText(r) && !isTextualRenderer(r))
    expect(onlyReads).toEqual(['form'])
  })
})

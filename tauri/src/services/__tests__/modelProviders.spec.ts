// @vitest-environment happy-dom
import { describe, it, expect } from 'vitest'
import { generateUniqueProviderId } from '../modelProviders'

describe('generateUniqueProviderId', () => {
  it('由名称生成可读 slug', () => {
    expect(generateUniqueProviderId('OpenAI GPT-4o', [])).toBe('openai-gpt-4o')
  })

  it('名称为空时回退为 provider', () => {
    expect(generateUniqueProviderId('   ', [])).toBe('provider')
  })

  it('清除非 ASCII 与特殊字符', () => {
    expect(generateUniqueProviderId('豆包/Doubao!', [])).toBe('doubao')
  })

  it('与现有 ID 冲突时递增后缀', () => {
    expect(generateUniqueProviderId('deepseek', ['deepseek'])).toBe('deepseek-2')
    expect(generateUniqueProviderId('deepseek', ['deepseek', 'deepseek-2'])).toBe('deepseek-3')
  })

  it('自动生成的 ID 不与任何现有 ID 冲突', () => {
    const existing = ['a', 'a-2', 'a-3']
    const id = generateUniqueProviderId('a', existing)
    expect(existing).not.toContain(id)
  })
})
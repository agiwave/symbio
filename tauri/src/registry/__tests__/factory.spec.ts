/**
 * 渲染器注册表工厂 —— 机制层的纯逻辑单测（node 环境）
 *
 * 「标识 → 组件 + 兜底」这条机制此前在 `vdfsTypes.ts` 与 `messageTypes.ts`
 * 各实现了一遍，现在只有一份。故直接对它下断言——两域各自的登记/解析行为
 * 由它们自己的 spec 覆盖（`vdfsTypes.spec.ts` / `messageRenderers.spec.ts`）。
 *
 * 锁定两件事：
 * 1. 三个动作的语义（register / get / resolve）；
 * 2. **两张表互相隔离**——这是「工厂而非共享表」的全部理由：两个域的标识空间
 *    同名（都有 `text` / `fallback`），共用一张表会静默串台。
 */

import { describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'
import { createRendererRegistry } from '../factory'

const A = defineComponent({ template: '<div />' })
const B = defineComponent({ template: '<span />' })
const FB = defineComponent({ template: '<p />' })

describe('createRendererRegistry', () => {
  it('register / get：登记即可取回；未登记返回 undefined', () => {
    const r = createRendererRegistry<'a' | 'b' | 'fallback'>('fallback')
    r.register('a', A)
    expect(r.get('a')).toBe(A)
    expect(r.get('b')).toBeUndefined()
  })

  it('resolve：未登记回落到兜底键', () => {
    const r = createRendererRegistry<'a' | 'b' | 'fallback'>('fallback')
    r.register('fallback', FB)
    r.register('a', A)
    expect(r.resolve('a')).toBe(A)
    expect(r.resolve('b')).toBe(FB)
  })

  it('resolve：兜底键自身也没登记 → undefined（调用方自行兜底）', () => {
    const r = createRendererRegistry<'a' | 'b' | 'fallback'>('fallback')
    r.register('a', A)
    expect(r.resolve('b')).toBeUndefined()
  })

  it('同键后登记者胜（装配点启动时各调一次，覆盖不会真的发生）', () => {
    const r = createRendererRegistry<'a' | 'b' | 'fallback'>('fallback')
    r.register('a', A)
    r.register('a', B)
    expect(r.get('a')).toBe(B)
  })

  it('每次创建都是独立的一张表（两域同名标识互不覆盖，一张表的兜底不影响另一张）', () => {
    const vdfs = createRendererRegistry<'text' | 'fallback'>('fallback')
    const msg = createRendererRegistry<'text' | 'fallback'>('fallback')
    vdfs.register('text', A)
    msg.register('text', B)
    expect(vdfs.resolve('text')).toBe(A)
    expect(msg.resolve('text')).toBe(B)
    // 只给 vdfs 登记兜底：msg 的解析结果不受影响
    vdfs.register('fallback', FB)
    expect(msg.resolve('text')).toBe(B)
  })

  it('三个动作可解构出去单独调用（实现不依赖 this）', () => {
    const r = createRendererRegistry<'a' | 'b' | 'fallback'>('fallback')
    const { register, get, resolve } = r
    register('a', A)
    register('fallback', FB)
    expect(get('a')).toBe(A)
    expect(resolve('a')).toBe(A)
  })
})

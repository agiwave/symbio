/**
 * 代次守卫 — 单测（node 环境）
 *
 * 守的是**机制本身**：`useVdfs`（详情读取 / 追加代际）与 `ChatMainPanel`
 * （会话切换）三处手写守卫都收敛到这个原语上，因此它的语义一旦漂移，
 * 三处会一起错——这正需要一份把它钉死的测试（原先那三处各写一遍，谁也没测）。
 *
 * 三个断言面：
 * 1. **单槽**：`advance` 单调递增；`revision()` 与取号相等 = 仍是最新；
 *    一次 `advance` 作废此前所有在途响应（这是「后来者作废先来者」的全部语义）。
 * 2. **键控**：每键各计数；**换键视为 0**（旧键的计数不再有意义）。
 * 3. **实例隔离**：不同关注点各起一个实例，互不作废。
 */

import { describe, expect, it } from 'vitest'
import { useGenerationGuard } from '../useGenerationGuard'

describe('useGenerationGuard — 单槽（后来者作废先来者）', () => {
  it('advance 从 1 起单调递增，revision 跟随', () => {
    const g = useGenerationGuard()
    expect(g.revision()).toBe(0) // 还没取过号
    expect(g.advance()).toBe(1)
    expect(g.revision()).toBe(1)
    expect(g.advance()).toBe(2)
    expect(g.revision()).toBe(2)
  })

  it('取号后未被 advance 时，该号仍是最新', () => {
    const g = useGenerationGuard()
    const rev = g.advance()
    expect(g.revision()).toBe(rev)
  })

  it('一次 advance 作废此前在途响应（模拟两次并发读取）', async () => {
    const g = useGenerationGuard()
    const applied: string[] = []
    // 第一次读取：慢（先发起、后落地）
    const slow = (async () => {
      const rev = g.advance()
      await new Promise((r) => setTimeout(r, 10))
      if (g.revision() === rev) applied.push('slow')
    })()
    // 第二次读取：快（后发起、先落地）
    const fast = (async () => {
      const rev = g.advance()
      if (g.revision() === rev) applied.push('fast')
    })()
    await Promise.all([slow, fast])
    // 慢响应必须被丢弃：否则它会覆盖新选中项的数据
    expect(applied).toEqual(['fast'])
  })
})

describe('useGenerationGuard — 键控（快照必须仍是当前的）', () => {
  it('同一个键上每次 advance 各计一代（换键前的计数随取随用）', () => {
    const g = useGenerationGuard()
    expect(g.advance('a')).toBe(1)
    expect(g.advance('a')).toBe(2)
    expect(g.revision('a')).toBe(2)
    // 从没 advance 过的键 = 0
    expect(g.revision('未用过')).toBe(0)
  })

  it('键控槽**只保留最近一个键**（内存有界，且正合语义）', () => {
    const g = useGenerationGuard()
    g.advance('a')
    g.advance('a')
    // 切到 b：a 的计数被遗忘——只有「当前那个节点」谈得上已应用几次追加
    expect(g.advance('b')).toBe(1)
    expect(g.revision('b')).toBe(1)
    expect(g.revision('a')).toBe(0)
  })

  it('读取期间的增量落地 ⇒ 响应过期（追加代际的真实场景）', async () => {
    const g = useGenerationGuard()
    const path = '@vfs/session/s1/消息'
    let text = 'AB'
    // 发起读取：记下快照
    const read = (async () => {
      const snap = g.revision(path)
      await new Promise((r) => setTimeout(r, 10))
      if (g.revision(path) !== snap) return '丢弃' // 期间有增量 → 丢弃旧快照
      return `应用:${text}`
    })()
    // 读取在途时，追加就地拼接并前进一代
    text += 'C'
    g.advance(path)
    expect(await read).toBe('丢弃')

    // 反例：没有增量落地 ⇒ 响应正常应用
    const snap = g.revision(path)
    const read2 = (async () => {
      await new Promise((r) => setTimeout(r, 5))
      return g.revision(path) === snap ? `应用:${text}` : '丢弃'
    })()
    expect(await read2).toBe('应用:ABC')
  })
})

describe('useGenerationGuard — 实例隔离', () => {
  it('一个实例的代次不影响另一个（详情读取 vs 追加代际）', () => {
    const detail = useGenerationGuard()
    const append = useGenerationGuard()
    const rev = detail.advance()
    append.advance('p')
    append.advance('p')
    // 追加的 advance 不该作废详情读取的号——两者作废时机不同，共用一个会互相误伤
    expect(detail.revision()).toBe(rev)
    expect(append.revision('p')).toBe(2)
  })
})

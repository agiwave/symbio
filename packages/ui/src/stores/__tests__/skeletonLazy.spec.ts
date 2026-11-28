// @vitest-environment happy-dom
/**
 * 骨架惰性重取（[] 前端半）——纯原语 + localStorage 账本
 *
 * 覆盖 exit 判据里两块**无需浏览器即可证**的部分：
 * 1. **往返一致**：`releaseSkeleton` 摘掉子树（保留 root），`mergeReleased` 把重取帧
 *    装回后与原字典逐键相等——证明「释放」与「重取」互逆。
 * 2. **跨刷新保留**：`markReleased` / `markExpanded` 落到 localStorage，新实例从账本
 *    恢复被释放的骨架集合。
 *
 * `subtreeOf` 与后端 `subtree_nodes` 的 BFS 语义逐字对齐，这里也单独钉 depth=1 / 'all'
 * 与「root 缺席 ⇒ 空」三种形状。
 */
import { describe, it, expect, beforeEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import type { ChatNode } from '@/schemas/chat_node'
import {
  subtreeOf,
  releaseSkeleton,
  mergeReleased,
  statsOf,
  useReleasedSkeletonsStore,
} from '../skeletonLazy'

const STORAGE_KEY = 'symbio.releasedSkeletons'

/** 造一个扁平节点：只需 id / parent_id，其余字段对本层无意义。 */
function node(id: string, parentId?: string): ChatNode {
  return parentId === undefined ? { id } : { id, parent_id: parentId }
}

/** 字典化：id → 节点。 */
function dictOf(...nodes: ChatNode[]): Record<string, ChatNode> {
  const out: Record<string, ChatNode> = {}
  for (const n of nodes) out[n.id] = n
  return out
}

/** 一棵 root ─ a / b(streaming) ─ b1 的样本树：statsOf 与账本用例共用。 */
const tree: ChatNode[] = [
  { id: 'root', type: 'skeleton', status: 'completed' },
  { id: 'a', parent_id: 'root', type: 'body', status: 'completed' },
  { id: 'b', parent_id: 'root', type: 'skeleton', status: 'streaming' },
  { id: 'b1', parent_id: 'b', type: 'notice', status: 'completed' },
]

describe('subtreeOf（前端子树算子，镜像后端 subtree_nodes）', () => {
  // root ─ a ─ a1
  //    └─ b
  const nodes = [node('root'), node('a', 'root'), node('b', 'root'), node('a1', 'a')]

  it("'all' 取整棵后代（不含 root）", () => {
    expect(subtreeOf(nodes, 'root', 'all').map((n) => n.id).sort()).toEqual(['a', 'a1', 'b'])
  })

  it('depth=1 只取直接子节点', () => {
    expect(subtreeOf(nodes, 'root', 1).map((n) => n.id).sort()).toEqual(['a', 'b'])
  })

  it('root 不在表里 ⇒ 空', () => {
    expect(subtreeOf(nodes, 'ghost', 'all')).toEqual([])
  })

  it('叶子节点的后代为空', () => {
    expect(subtreeOf(nodes, 'a1', 'all')).toEqual([])
  })
})

describe('释放 → 重取 往返', () => {
  const original = dictOf(
    node('root'),
    node('a', 'root'),
    node('b', 'root'),
    node('a1', 'a'),
    node('keep', undefined),
  )

  it('releaseSkeleton 摘掉整棵子树、保留 root 与无关节点', () => {
    const { kept, released } = releaseSkeleton(original, 'root')
    expect(Object.keys(kept).sort()).toEqual(['keep', 'root'])
    expect(released.map((n) => n.id).sort()).toEqual(['a', 'a1', 'b'])
  })

  it('重取帧装回后与原字典逐键相等（往返一致）', () => {
    const { kept, released } = releaseSkeleton(original, 'root')
    // 模拟「按需重取」返回的窄帧就是当初摘掉的那批（后端 subtree_nodes 同语义）
    const restored = mergeReleased(kept, released)
    expect(Object.keys(restored).sort()).toEqual(Object.keys(original).sort())
    for (const id of Object.keys(original)) {
      expect(restored[id]).toEqual(original[id])
    }
  })
})

describe('statsOf（折叠 chip 的数据源：多少条 / 什么类型 / 还在跑几个）', () => {
  it('按类型计数、running 只数流式中', () => {
    expect(statsOf(tree.slice(1))).toEqual({
      count: 3,
      byType: { body: 1, skeleton: 1, notice: 1 },
      running: 1,
    })
  })

  it('空子树 ⇒ 全零（不是 undefined）', () => {
    expect(statsOf([])).toEqual({ count: 0, byType: {}, running: 0 })
  })

  it('缺 type 的节点归 unknown（不猜形态）', () => {
    expect(statsOf([{ id: 'x' }])).toEqual({
      count: 1,
      byType: { unknown: 1 },
      running: 0,
    })
  })
})

describe('useReleasedSkeletonsStore（localStorage 账本）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
  })

  it('默认无释放记录', () => {
    const store = useReleasedSkeletonsStore()
    expect(store.isReleased('s1', 'root')).toBe(false)
    expect(store.summaryOf('s1', 'root')).toBeUndefined()
  })

  it('markReleased 记入账本并持久化（摘要带 stats）', () => {
    const store = useReleasedSkeletonsStore()
    store.markReleased('s1', 'root', statsOf(tree.slice(1)))
    expect(store.isReleased('s1', 'root')).toBe(true)
    expect(store.summaryOf('s1', 'root')).toEqual({
      count: 3,
      byType: { body: 1, skeleton: 1, notice: 1 },
      running: 1,
    })
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({
      s1: { root: { count: 3, byType: { body: 1, skeleton: 1, notice: 1 }, running: 1 } },
    })
  })

  it('markExpanded 从账本摘除并持久化', () => {
    const store = useReleasedSkeletonsStore()
    store.markReleased('s1', 'root', statsOf(tree.slice(1)))
    store.markExpanded('s1', 'root')
    expect(store.isReleased('s1', 'root')).toBe(false)
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({ s1: {} })
  })

  it('新实例从 localStorage 恢复被释放的骨架（跨刷新保留）', () => {
    const store = useReleasedSkeletonsStore()
    store.markReleased('s1', 'root', statsOf(tree.slice(1)))
    store.markReleased('s1', 'other', statsOf(tree.slice(1, 2)))
    store.markExpanded('s1', 'other')

    // 模拟刷新：换新 Pinia 实例，setup 重跑 → loadLedger 读回 localStorage
    setActivePinia(createPinia())
    const reloaded = useReleasedSkeletonsStore()
    expect(reloaded.isReleased('s1', 'root')).toBe(true)
    expect(reloaded.summaryOf('s1', 'root')).toEqual({
      count: 3,
      byType: { body: 1, skeleton: 1, notice: 1 },
      running: 1,
    })
    expect(reloaded.isReleased('s1', 'other')).toBe(false)
  })

  it('forgetSession 清偿整段（会话删除 / 骨架已不在）', () => {
    const store = useReleasedSkeletonsStore()
    store.markReleased('s1', 'root', statsOf(tree.slice(1)))
    store.markReleased('s2', 'root', statsOf(tree.slice(1)))
    store.forgetSession('s1')
    expect(store.isReleased('s1', 'root')).toBe(false)
    expect(store.isReleased('s2', 'root')).toBe(true)
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({
      s2: { root: { count: 3, byType: { body: 1, skeleton: 1, notice: 1 }, running: 1 } },
    })
  })

  it('账本里损坏的 JSON 退化为空，不抛', () => {
    localStorage.setItem(STORAGE_KEY, '{ not json')
    setActivePinia(createPinia())
    const store = useReleasedSkeletonsStore()
    expect(store.isReleased('s1', 'root')).toBe(false)
  })
})

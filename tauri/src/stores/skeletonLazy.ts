/**
 * 骨架惰性重取（[T-150] 前端半）——折叠骨架本地释放子节点 + 按需重取的**纯原语**与
 * 「已释放骨架」的 localStorage 账本。
 *
 * 分两层，各自可独立测：
 * - **纯函数**（`subtreeOf` / `releaseSkeleton` / `mergeReleased`）：对扁平节点表做
 *   「摘子树 / 装回子树」，是组件与 composable 调用的数据基座；镜像后端
 *   `plugin/nodes.rs::subtree_nodes` 的 BFS 语义（前端按 `parent_id` 组装、后端按同
 *   一根算子，两侧逐字对齐才能保证「重取 = 释放前」）。
 * - **`useReleasedSkeletonsStore`**：一张 `会话 → 骨架 → 摘要` 的账本，持久化到
 *   localStorage（`symbio.releasedSkeletons`），跨刷新保留。`unloaded` 因此**不进
 *   `ChatNode` 线格式**——它是纯前端状态，写进 wire 会被 `protocol-mirror-audit` D 组
 *   判红（前后端字段名一比对就多一个）。
 *
 * 摘要（释放时有多少子节点）也存这张表：折叠态要显示「N 项」却已不持有那些节点，
 * 本地结算的计数是唯一来源（后端没有 `NodeStats` producer，见 T-150 文档 150-ii）。
 */

import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { ChatNode } from '@/schemas/chat_node'

const STORAGE_KEY = 'symbio.releasedSkeletons'

/** 一棵子树释放时留下的摘要——够渲染折叠 chip，不含正文。 */
export interface ReleasedSummary {
  /** 释放时该骨架有多少后代节点 */
  count: number
}

/** 账本形态：`会话 id → 骨架 id → 摘要`。 */
type PersistedLedger = Record<string, Record<string, ReleasedSummary>>

/**
 * 取 `root` 的后代节点（**不含 root 本身**）。`depth=1` 只直接子节点；`'all'` 整棵。
 *
 * 按 `parent_id` BFS，遍历序确定（先入表者在先）。root 不在 `nodes` 里 ⇒ 空。
 * 与后端 `subtree_nodes` 同语义——两侧必须一致，否则「重取回来的形状」≠「释放掉的形状」。
 */
export function subtreeOf(
  nodes: ChatNode[],
  root: string,
  depth: 1 | 'all' = 'all',
): ChatNode[] {
  const children = new Map<string, ChatNode[]>()
  let hasRoot = false
  for (const n of nodes) {
    if (n.id === root) hasRoot = true
    const p = n.parent_id
    if (p != null && p !== '') {
      const arr = children.get(p)
      if (arr) arr.push(n)
      else children.set(p, [n])
    }
  }
  if (!hasRoot) return []

  const out: ChatNode[] = []
  let frontier = children.get(root) ?? []
  let guard = nodes.length + 1 // 环防御：合法转写是树，不会触顶
  while (frontier.length > 0 && guard-- > 0) {
    const next: ChatNode[] = []
    for (const n of frontier) {
      out.push(n)
      if (depth !== 1) {
        const kids = children.get(n.id)
        if (kids) next.push(...kids)
      }
    }
    if (depth === 1) break
    frontier = next
  }
  return out
}

/**
 * 从节点字典里**摘掉** `root` 的子树（保留 root 自身）——本地释放。
 * 返回被摘掉的节点，供账本计数、也供「重取=装回」的形状对照。
 */
export function releaseSkeleton(
  dict: Record<string, ChatNode>,
  root: string,
): { kept: Record<string, ChatNode>; released: ChatNode[] } {
  const all = Object.values(dict)
  const released = subtreeOf(all, root, 'all')
  const kept: Record<string, ChatNode> = {}
  const drop = new Set(released.map((n) => n.id))
  for (const [id, n] of Object.entries(dict)) {
    if (!drop.has(id)) kept[id] = n
  }
  return { kept, released }
}

/**
 * 把重取回来的窄帧**装回**节点字典（按 id 覆盖）。这是
 * `applyTranscriptMessages` 的极简同形版——「释放 → 重取 → 树一致」往返里用它证明
 * 摘掉与装回互为逆。生产路径的字段合并仍走 store 的落地口。
 */
export function mergeReleased(
  dict: Record<string, ChatNode>,
  frames: ChatNode[],
): Record<string, ChatNode> {
  const next: Record<string, ChatNode> = { ...dict }
  for (const f of frames) next[f.id] = f
  return next
}

function loadLedger(): PersistedLedger {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? (JSON.parse(raw) as PersistedLedger) : {}
  } catch {
    return {}
  }
}

export const useReleasedSkeletonsStore = defineStore('releasedSkeletons', () => {
  // 刷新即从 localStorage 恢复：被释放的骨架集合跨刷新保留（exit 判据）。
  const ledger = ref<PersistedLedger>(loadLedger())

  function persist(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(ledger.value))
    } catch {
      // 隐私模式 / 配额满 ⇒ 退化：账本仍活在内存里，只是不跨刷新。
    }
  }

  /** 该骨架当前是否处于「已释放」态。 */
  function isReleased(sessionId: string, mid: string): boolean {
    return ledger.value[sessionId]?.[mid] != null
  }

  /** 折叠摘要（释放时有多少后代）；未释放 ⇒ undefined。 */
  function summaryOf(sessionId: string, mid: string): ReleasedSummary | undefined {
    return ledger.value[sessionId]?.[mid]
  }

  /** 记一次释放（携带本地结算的后代数）。 */
  function markReleased(sessionId: string, mid: string, count: number): void {
    const bySid = ledger.value[sessionId] ?? {}
    bySid[mid] = { count }
    ledger.value = { ...ledger.value, [sessionId]: bySid }
    persist()
  }

  /** 记一次展开重取（从账本摘除该骨架）。 */
  function markExpanded(sessionId: string, mid: string): void {
    const bySid = ledger.value[sessionId]
    if (!bySid || bySid[mid] == null) return
    const rest = { ...bySid }
    delete rest[mid]
    ledger.value = { ...ledger.value, [sessionId]: rest }
    persist()
  }

  return { ledger, isReleased, summaryOf, markReleased, markExpanded }
})

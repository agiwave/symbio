/**
 * useWorkbench —— 「侧边栏 + 列表 + 详情」三栏工作台核心状态机（唯一实现）
 *
 * 整个 App 是一台三栏工作台：侧边栏（类别导航）+ 列表 + 详情。本项目所有
 * 资源型页面共用本核心，差异（数据源、删除语义）通过 ops 注入：
 *
 * - 顶层资源页（useWorkbenchView entity 模式）：categories = 后端 providers 注册表，
 *   listItems = resources/list（按 kind），deleteItem = resources/delete；
 * - 容器资源页（useWorkbenchView container 模式）：categories =
 *   ProviderInfo.container_kinds，listAll = resources/list（payload.container 一次
 *   取回全类别），deleteItem = resources/delete（带 container）。
 *
 * 协议完全统一：两者走同一套 `${prefix}/resources/*`，容器仅多 container 字段；
 * 状态机（选中复合键 `${kind}:${id}`、creating 模式、加载/删除状态、删除后刷新）
 * 在此唯一实现，页面组合式只是薄适配层。
 */

import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import type { ResourceCapabilities, ResourceSummary } from '@/schemas/resources'

/** 工作台类别（侧边栏一项）：kind + 展示标签 + 能力（决定可建/可删/可测） */
export interface WorkbenchCategory {
  kind: string
  label: string
  capabilities: ResourceCapabilities
}

/** 每类别的清单状态（capabilities 后端下发为真相源） */
export interface WorkbenchKindState {
  items: ResourceSummary[]
  capabilities: ResourceCapabilities
}

/** 清单拉取结果（capabilities 缺省回退类别声明） */
export interface WorkbenchListResult {
  items: ResourceSummary[]
  capabilities?: ResourceCapabilities
}

/** 差异注入点：页面组合式以 ops 描述「我的数据从哪来、怎么删」 */
export interface WorkbenchOps {
  /** 当前工作台的类别集合（响应式 getter） */
  categories: () => WorkbenchCategory[]
  /** 按类别拉取清单（与 listAll 二选一，优先 listAll） */
  listItems?: (kind: string) => Promise<WorkbenchListResult>
  /** 一次拉取全部类别清单（跨类别返回，按 item.kind 分箱） */
  listAll?: () => Promise<WorkbenchListResult>
  /** 删除条目（缺省 = 该工作台不可删） */
  deleteItem?: (kind: string, id: string) => Promise<void>
  /** 加载后无选中时自动选中首项（顶层资源页 true；容器页 false 留空引导） */
  autoSelect?: boolean
  logTag?: string
}

export function useWorkbench(ops: WorkbenchOps) {
  const toast = useToast()
  const logTag = ops.logTag ?? 'Workbench'

  // === 通用状态 ===
  const loading = ref(false)
  const saving = ref(false)
  const testing = ref(false)
  const creating = ref(false)
  /** 选中键（复合 `${kind}:${id}`），null = 未选中 */
  const selectedId = ref<string | null>(null)
  const deletingId = ref<string | null>(null)

  // === 类别与清单 ===
  const categories: ComputedRef<WorkbenchCategory[]> = computed(() => ops.categories())
  const kindStates = ref<Record<string, WorkbenchKindState>>({})

  /** 选中项解析（复合键 → 类别内查找） */
  const selected = computed<{ kind: string; item: ResourceSummary } | null>(() => {
    const key = selectedId.value
    if (!key) return null
    const idx = key.indexOf(':')
    if (idx < 0) return null
    const kind = key.slice(0, idx)
    const id = key.slice(idx + 1)
    const item = kindStates.value[kind]?.items.find((i) => i.id === id)
    return item ? { kind, item } : null
  })

  /** 平排列表：所有类别所有项按类别顺序展平（顶层资源页用） */
  const itemsAll = computed<ResourceSummary[]>(() => {
    const flat: ResourceSummary[] = []
    for (const c of categories.value) {
      for (const it of kindStates.value[c.kind]?.items ?? []) flat.push(it)
    }
    return flat
  })

  /** 某类别清单（容器页用；非响应式包装，视图中建议 itemsOf(kind) 计算属性） */
  function itemsOf(kind: string): ResourceSummary[] {
    return kindStates.value[kind]?.items ?? []
  }

  // === 选中 ===
  /** 选中（复合键）；传 null 清除选中。选中即退出新建态。 */
  function selectKey(key: string | null) {
    creating.value = false
    selectedId.value = key || null
  }

  /** 是否选中该条目（复合键比较） */
  function isSelected(kind: string, id: string): boolean {
    return selectedId.value === `${kind}:${id}`
  }

  /** 首 项键（按 itemsAll 顺序；空 → null） */
  function firstKey(): string | null {
    for (const it of itemsAll.value) return `${it.kind}:${it.id}`
    return null
  }

  // === 新建模式 ===
  function enterCreate() {
    creating.value = true
    selectedId.value = null
  }

  /** 取消新建：恢复到首项（autoSelect 模式）或清空 */
  function cancelCreate() {
    creating.value = false
    if (ops.autoSelect) {
      const first = firstKey()
      if (first) selectKey(first)
    }
  }

  // === 加载 ===
  /** 加载全部类别清单；listAll 优先（单请求分箱），否则逐类别并行 */
  async function loadAll() {
    loading.value = true
    try {
      if (ops.listAll) {
        const resp = await ops.listAll()
        kindStates.value = partitionByKind(resp, categories.value)
      } else if (ops.listItems) {
        const entries = await Promise.all(
          categories.value.map(async (c) => [c.kind, await ops.listItems!(c.kind)] as const)
        )
        const states: Record<string, WorkbenchKindState> = {}
        for (const [kind, resp] of entries) {
          states[kind] = toKindState(resp, kind, categories.value)
        }
        kindStates.value = states
      }
      if (ops.autoSelect && !selectedId.value && !creating.value) {
        const first = firstKey()
        if (first) selectKey(first)
      }
    } finally {
      loading.value = false
    }
  }

  /** 刷新某类别（listAll 模式下整表重拉后仅更新该类别箱） */
  async function refreshKind(kind: string) {
    const caps = categories.value.find((c) => c.kind === kind)?.capabilities
    if (ops.listAll) {
      const resp = await ops.listAll()
      const next = partitionByKind(resp, categories.value)
      kindStates.value = { ...kindStates.value, [kind]: next[kind] ?? { items: [], capabilities: caps ?? fallbackCaps() } }
      return
    }
    if (!ops.listItems) return
    const resp = await ops.listItems(kind)
    kindStates.value = {
      ...kindStates.value,
      [kind]: toKindState(resp, kind, categories.value),
    }
  }

  // === 删除（唯一实现：确认 → deletingId → ops.deleteItem → 刷新 → 清选中） ===
  async function remove(kind: string, id: string, label?: string): Promise<boolean> {
    if (!ops.deleteItem) {
      logger.warn(logTag, `类别 ${kind} 未注入 deleteItem，忽略删除`)
      return false
    }
    if (deletingId.value === id) return false
    if (!window.confirm(`确认删除${label ? ` ${label}` : '资源'}「${id}」？`)) return false
    deletingId.value = id
    try {
      await ops.deleteItem(kind, id)
      toast.showToast('success', `已删除${label ? ` ${label}` : '资源'}「${id}」`)
      await refreshKind(kind)
      if (selectedId.value === `${kind}:${id}`) selectKey(null)
      return true
    } catch (err) {
      logger.error(logTag, 'delete failed:', err)
      toast.showToast('error', `删除失败: ${err}`)
      return false
    } finally {
      deletingId.value = null
    }
  }

  return {
    // 状态
    loading,
    saving,
    testing,
    creating,
    selectedId,
    deletingId,
    // 类别与清单
    categories,
    kindStates,
    itemsAll,
    itemsOf,
    selected,
    // 行为
    selectKey,
    isSelected,
    firstKey,
    enterCreate,
    cancelCreate,
    loadAll,
    refreshKind,
    remove,
    showToast: toast.showToast,
  }
}

/** listAll 结果 → 按 item.kind 分箱（仅保留已声明类别，未知类别丢弃） */
function partitionByKind(
  resp: WorkbenchListResult,
  categories: readonly WorkbenchCategory[]
): Record<string, WorkbenchKindState> {
  const states: Record<string, WorkbenchKindState> = {}
  for (const c of categories) {
    states[c.kind] = { items: [], capabilities: c.capabilities }
  }
  for (const it of resp.items ?? []) {
    const known = states[it.kind]
    if (known) known.items.push(it)
  }
  if (resp.capabilities) {
    for (const c of categories) states[c.kind].capabilities = resp.capabilities
  }
  return states
}

function toKindState(
  resp: WorkbenchListResult,
  kind: string,
  categories: readonly WorkbenchCategory[]
): WorkbenchKindState {
  const caps =
    categories.find((c) => c.kind === kind)?.capabilities ??
    fallbackCaps()
  return {
    items: resp.items ?? [],
    capabilities: resp.capabilities ?? caps,
  }
}

function fallbackCaps(): ResourceCapabilities {
  return {
    zip_upload: false,
    independent_form: false,
    realtime_status: false,
    mutable: false,
    test_connection: false,
    read_only: true,
  }
}

/** 供适配层做 props 类型标注 */
export type { Ref }

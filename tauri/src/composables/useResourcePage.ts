/**
 * useResourcePage —— 统一资源页核心逻辑（可单测的组合式函数）
 *
 * 承载：活动类型解析、多类型数据加载、混合平排列表、复合选中、创建/删除能力判定。
 * ResourceManagerView 只保留模板、事件绑定与新建/删除/测试等 UI 副作用。
 *
 * 混合列表 = 所有活动类型的所有资源**平排**为一张列表（不分类型分组），
 * 按 name 自然排序（目录式交错，类似一个目录中多种后缀文件混排）。
 * 选择用 `${kind}:${id}` 复合键跨类型唯一。
 */

import { computed, ref, type Component, type ComputedRef, type Ref } from 'vue'
import { listResources } from '@/services/resources'
import { useResourceManager } from '@/composables/useResourceManager'
import { resolveActiveTypes, useResourceProviders } from '@/composables/useResourceProviders'
import { getResourceEditor } from '@/registry/resourceTypes'
import type { ProviderInfo, ResourceCapabilities, ResourceSummary } from '@/schemas/resources'

export interface TypeState {
  items: ResourceSummary[]
  capabilities: ResourceCapabilities
}

export interface SelectedResource {
  kind: string
  item: ResourceSummary
}

export function useResourcePage(typesParam: Ref<string | undefined>) {
  const { providers, getProvider, labelOf, capabilitiesOf } = useResourceProviders()

  // === 活动类型（注册表派生的运行时类型集合） ===
  const activeTypes: ComputedRef<ProviderInfo[]> = computed(() =>
    resolveActiveTypes(providers.value, typesParam.value)
  )
  const isMulti = computed(() => activeTypes.value.length > 1)

  // === 每类型数据（capabilities 后端下发为真相源） ===
  const typeStates = ref<Record<string, TypeState>>({})

  // === 选中（复合键 `${kind}:${id}`）——复用 useResourceManager 的 selectedId/creating ===
  const {
    selectedId,
    creating,
    saving,
    loading,
    deletingId,
    select,
    enterCreateMode,
    showToast,
  } = useResourceManager({ logTag: 'ResourcePage' })

  /** 混合平排列表：所有活动类型所有项展平，按 name 排序 */
  const items = computed<ResourceSummary[]>(() =>
    buildMixedItems(activeTypes.value, typeStates.value)
  )

  const selected = computed<SelectedResource | null>(() => {
    const key = selectedId.value
    if (!key) return null
    const idx = key.indexOf(':')
    if (idx < 0) return null
    const kind = key.slice(0, idx)
    const id = key.slice(idx + 1)
    const item = typeStates.value[kind]?.items.find((i) => i.id === id)
    return item ? { kind, item } : null
  })

  const totalCount = computed(() =>
    Object.values(typeStates.value).reduce((n, s) => n + s.items.length, 0)
  )
  const enabledCount = computed(() =>
    Object.values(typeStates.value).reduce(
      (n, s) => n + s.items.filter((i) => i.status === 'active' || i.status === 'working').length,
      0
    )
  )

  const allItemsEmpty = computed(() =>
    activeTypes.value.every((d) => (typeStates.value[d.kind]?.items.length ?? 0) === 0)
  )

  /** 当前活动类型中可创建的类型（supports_upload && 可写 && 非只读） */
  const creatableInActive = computed(() => {
    const activeKinds = new Set(activeTypes.value.map((p) => p.kind))
    return activeTypes.value
      .filter((p) => p.kind && activeKinds.has(p.kind))
      .filter(isManagerCreatable)
  })
  const canCreate = computed(() => creatableInActive.value.length > 0)

  /** 选中项的删除能力（supports_upload && 可写 && 非只读） */
  const canDeleteSelected = computed(() => {
    const sel = selected.value
    if (!sel) return false
    return isManagerCreatable(getProvider(sel.kind) ?? makeReadonly(sel.kind))
  })

  /** 某 kind 的展示标签 */
  function kindLabel(kind: string): string {
    return labelOf(kind, kind)
  }

  /** 某 kind 的能力（未加载/未知 → 只读空态） */
  function capsOf(kind: string): ResourceCapabilities {
    return capabilitiesOf(kind)
  }

  /** 某 kind 的专属编辑表单（未注册 null → 通用兜底） */
  function editorOf(kind: string) {
    const d = getProvider(kind)
    if (d) return getResourceEditor(d.kind)
    return getResourceEditor(kind)
  }

  const selectedEditor = computed(() =>
    selected.value ? getResourceEditor(selected.value.kind) ?? null : null
  )
  /** 某 kind 的专属编辑表单（新建时用，未注册 null → 通用兜底） */
  function createEditor(kind: string | null): Component | null {
    return kind ? getResourceEditor(kind) ?? null : null
  }

  // === 加载 ===
  async function loadAll() {
    loading.value = true
    try {
      const entries = await Promise.all(
        activeTypes.value.map(async (d) => [d.kind, await listResources(d.kind)] as const)
      )
      const states: Record<string, TypeState> = {}
      for (const [kind, resp] of entries) {
        states[kind] = {
          items: resp.items || [],
          capabilities: resp.capabilities || capabilitiesOf(kind),
        }
      }
      typeStates.value = states
      if (!selectedId.value && !creating.value) {
        const first = firstItemKey()
        if (first) select(first)
      }
    } finally {
      loading.value = false
    }
  }

  async function refreshKind(kind: string) {
    const list = await listResources(kind)
    typeStates.value = {
      ...typeStates.value,
      [kind]: {
        items: list.items || [],
        capabilities: list.capabilities || capabilitiesOf(kind),
      },
    }
  }

  function firstItemKey(): string | null {
    for (const it of items.value) {
      return `${it.kind}:${it.id}`
    }
    return null
  }

  // === 新建 ===
  const createKind = ref<string | null>(null)
  const activeFormKind = computed<string | null>(() =>
    creating.value ? createKind.value : (selected.value?.kind ?? null)
  )

  /** 绑定某类型并进入创建态（单类型新建 / 类型选择面板点选后） */
  function beginCreate(kind: string) {
    createKind.value = kind
    if (!creating.value) enterCreateMode()
  }

  /** 进入创建态但不绑定类型 → 渲染"类型选择面板"（多类型新建） */
  function startTypeChoice() {
    createKind.value = null
    if (!creating.value) enterCreateMode()
  }

  function cancelCreate() {
    creating.value = false
    createKind.value = null
    const first = firstItemKey()
    if (first) select(first)
  }

  // === 删除/测试后刷新选中 ===
  async function afterMutate(kind: string) {
    await refreshKind(kind)
  }

  return {
    activeTypes,
    isMulti,
    typeStates,
    items,
    selected,
    selectedId,
    creating,
    saving,
    loading,
    deletingId,
    totalCount,
    enabledCount,
    allItemsEmpty,
    creatableInActive,
    canCreate,
    canDeleteSelected,
    kindLabel,
    capsOf,
    editorOf,
    selectedEditor,
    createEditor,
    activeFormKind,
    createKind,
    select,
    loadAll,
    refreshKind,
    firstItemKey,
    beginCreate,
    startTypeChoice,
    cancelCreate,
    afterMutate,
    showToast,
  }
}

/** 是否可在资源管理器内创建/删除（supports_upload 为"协议实现"维度，与 mutable 解耦） */
export function isManagerCreatable(p: { supports_upload?: boolean; capabilities?: ResourceCapabilities }): boolean {
  return Boolean(p.supports_upload && p.capabilities?.mutable && !p.capabilities.read_only)
}

/**
 * 混合平排列表纯函数：把所有活动类型的所有资源展平为一张列表（不分类型分组），
 * 按 name（小写）自然排序——目录式交错（一个目录混排多种后缀文件）。
 */
export function buildMixedItems(
  activeTypes: readonly ProviderInfo[],
  typeStates: Record<string, TypeState>
): ResourceSummary[] {
  const flat: Array<{ item: ResourceSummary; name: string }> = []
  for (const d of activeTypes) {
    for (const it of typeStates[d.kind]?.items ?? []) {
      flat.push({ item: it, name: (it.name || it.id).toLowerCase() })
    }
  }
  return flat.sort((a, b) => a.name.localeCompare(b.name)).map((x) => x.item)
}

/** 兜底只读 provider（未知类型） */
function makeReadonly(kind: string): ProviderInfo {
  return {
    kind,
    provider_name: kind,
    prefix: kind,
    capabilities: { zip_upload: false, independent_form: false, realtime_status: false, mutable: false, test_connection: false, read_only: true },
    order: 999,
    label: kind,
    supports_upload: false,
  }
}
/**
 * 运行时资源 provider store（模块级单例）
 *
 * 前端在一处拉取后端 `resources/providers` 注册表，供多处共享：MainLayout
 * 据此动态生成左侧"资源"导航，ResourceManagerView 据此得到统一资源页的类型集合。
 * 与 useToast 一样是模块级单例，无需 Pinia。
 *
 * 分层原则：
 * - 类型的**存在/能力/前缀/顺序/标签**来自后端 ProviderInfo（权威）
 * - 前端只补充 UI 映射（editor 组件、icon）——见 registry/resourceTypes.ts
 */

import { computed, shallowRef } from 'vue'
import { fetchProviders } from '@/services/resources'
import type { ProviderInfo, ResourceCapabilities } from '@/schemas/resources'
import { RESOURCE_LABELS, NAV_RESOURCES, NAV_SETTINGS } from '@/schemas/resources'
import { logger } from '@/utils/logger'

const providers = shallowRef<ProviderInfo[]>([])
let state: 'idle' | 'loading' | 'loaded' = 'idle'
let loadingPromise: Promise<void> | null = null

/**
 * 幂等加载 provider 注册表：并发调用共享同一 promise，成功后跳过；
 * 失败不置 loaded，下次调用可重试。
 */
export async function loadProviders(): Promise<void> {
  if (state === 'loaded') return
  if (!loadingPromise) {
    state = 'loading'
    loadingPromise = (async () => {
      try {
        const list = await fetchProviders()
        providers.value = list
        state = 'loaded'
      } finally {
        loadingPromise = null
        if (state !== 'loaded') state = 'idle'
      }
    })()
  }
  await loadingPromise
}

/** 全部已注册 provider（按 order 排序） */
export function useResourceProviders() {
  const sortedProviders = computed(() =>
    [...providers.value].sort((a, b) => a.order - b.order)
  )

  /** 某 kind 的 provider；未知类型返回 null */
  function getProvider(kind: string): ProviderInfo | null {
    return providers.value.find((p) => p.kind === kind) ?? null
  }

  /** kind 展示标签（后端 label 优先，兜底 RESOURCE_LABELS，再兜底 kind） */
  function labelOf(kind: string, fallback?: string): string {
    const p = getProvider(kind)
    if (p?.label) return p.label
    return RESOURCE_LABELS[kind] ?? fallback ?? kind
  }

  /** kind 能力（provider 不存在 / 未加载时返回只读空态） */
  function capabilitiesOf(kind: string) {
    return (
      getProvider(kind)?.capabilities ?? {
        zip_upload: false,
        independent_form: false,
        realtime_status: false,
        mutable: false,
        test_connection: false,
        read_only: true,
      }
    )
  }

  /** 可在资源管理器内创建/删除的类型（supports_upload && 可写 && 非只读） */
  const creatableProviders = computed(() =>
    sortedProviders.value.filter(
      (p) => p.supports_upload && p.capabilities.mutable && !p.capabilities.read_only
    )
  )

  /** 可在资源管理器内删除的类型（供 canDelete 判断，选中项单独判定） */
  function isDeletable(p: { supports_upload?: boolean; capabilities?: ResourceCapabilities }): boolean {
    return Boolean(p.supports_upload && p.capabilities?.mutable && !p.capabilities.read_only)
  }

  /** 资源分组导航：ProviderInfo.nav === 'resources'（model/mcp/agent/skill 等可管理类型） */
  const resourceNav = computed(() =>
    sortedProviders.value.filter((p) => p.nav === NAV_RESOURCES)
  )

  /** 设置分组导航：ProviderInfo.nav === 'settings'（当前为 setting，左侧"设置"入口据此动态生成） */
  const settingsNav = computed(() =>
    sortedProviders.value.filter((p) => p.nav === NAV_SETTINGS)
  )

  return {
    providers: sortedProviders,
    resourceNav,
    settingsNav,
    getProvider,
    labelOf,
    capabilitiesOf,
    creatableProviders,
    isDeletable,
  }
}

/**
 * 解析路由 `:types` 参数 → 有序活动 ProviderInfo[]（运行时，基于已加载 provider）。
 *
 * - `undefined / '' / 'all'` → 所有 `supports_upload` 的类型（可管理的资源；session 等
 *   不可管理类型默认不进聚合页，但可通过显式 kind 访问）
 * - 逗号分隔 → trim / 去重 / 过滤未知 kind（记 warn 后丢弃）
 * - 过滤后为空 → 回退 all
 */
export function resolveActiveTypes(
  providersList: readonly ProviderInfo[],
  typesParam?: string
): ProviderInfo[] {
  const sorted = [...providersList].sort((a, b) => a.order - b.order)
  const defaultTypes = sorted.filter((p) => p.supports_upload)
  const raw = (typesParam ?? '').trim()
  if (!raw || raw === 'all') return defaultTypes

  const seen = new Set<string>()
  const out: ProviderInfo[] = []
  for (const seg of raw.split(',')) {
    const kind = seg.trim()
    if (!kind || seen.has(kind)) continue
    const p = sorted.find((x) => x.kind === kind)
    if (p) {
      seen.add(kind)
      out.push(p)
    } else {
      logger.warn('useResourceProviders', `未注册的资源类型 "${kind}"，已忽略`)
    }
  }
  return out.length ? out.sort((a, b) => a.order - b.order) : defaultTypes
}
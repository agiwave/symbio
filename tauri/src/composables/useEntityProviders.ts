/**
 * 运行时实体 provider store（模块级单例）
 *
 * 前端在一处拉取后端 `entities/providers` 注册表，供多处共享：MainLayout
 * 据此动态生成左侧"实体"导航（useNavRailItems），WorkbenchView 据此得到统一
 * 实体页的类型集合与容器子类别。与 useToast 一样是模块级单例，无需 Pinia。
 *
 * 分层原则：
 * - 类型的**存在/能力/前缀/顺序/标签**来自后端 ProviderInfo（权威）
 * - 前端只补充 UI 映射（editor 组件、icon）——见 registry/entityTypes.ts
 */

import { computed, shallowRef } from 'vue'
import { fetchProviders } from '@/services/entities'
import type { ProviderInfo, EntityCapabilities } from '@/schemas/entities'
import { ENTITY_LABELS } from '@/schemas/entities'
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
export function useEntityProviders() {
  const sortedProviders = computed(() =>
    [...providers.value].sort((a, b) => a.order - b.order)
  )

  /** 某 kind 的 provider；未知类型返回 null */
  function getProvider(kind: string): ProviderInfo | null {
    return providers.value.find((p) => p.kind === kind) ?? null
  }

  /** kind 展示标签（后端 label 优先，兜底 ENTITY_LABELS，再兜底 kind） */
  function labelOf(kind: string, fallback?: string): string {
    const p = getProvider(kind)
    if (p?.label) return p.label
    return ENTITY_LABELS[kind] ?? fallback ?? kind
  }

  /** kind 能力（provider 不存在 / 未加载时返回只读空态） */
  function capabilitiesOf(kind: string) {
    return (
      getProvider(kind)?.capabilities ?? {
        zip_upload: false,
        independent_form: false,
        realtime_status: false,
        // 未登记类型：刷新幂等无害，保守保留入口（与后端 capabilities_for 兜底一致）
        refreshable: true,
        mutable: false,
        test_connection: false,
        read_only: true,
      }
    )
  }

  /** 可在实体管理器内创建/删除的类型（supports_upload && 可写 && 非只读） */
  const creatableProviders = computed(() =>
    sortedProviders.value.filter(
      (p) => p.supports_upload && p.capabilities.mutable && !p.capabilities.read_only
    )
  )

  /** 可在实体管理器内删除的类型（供 canDelete 判断，选中项单独判定） */
  function isDeletable(p: { supports_upload?: boolean; capabilities?: EntityCapabilities }): boolean {
    return Boolean(p.supports_upload && p.capabilities?.mutable && !p.capabilities.read_only)
  }

  return {
    providers: sortedProviders,
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
 * - `undefined / '' / 'all'` → 所有 `supports_upload` 的类型（可管理的实体；session 等
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
      logger.warn('useEntityProviders', `未注册的实体类型 "${kind}"，已忽略`)
    }
  }
  return out.length ? out.sort((a, b) => a.order - b.order) : defaultTypes
}

// 应用外壳导航已迁至 `composables/useNavRail.ts`（S4 起由 `.vdfs` 驱动）。

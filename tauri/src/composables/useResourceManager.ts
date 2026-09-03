/**
 * useResourceManager — 资源管理页通用状态 composable
 *
 * 适用于 ModelProviderView、McpView、SkillView、AgentView 等管理类页面。
 * 统一管理：
 * - loading / saving / testing / deleting 状态
 * - creating 模式（vs 选中已有项）
 * - selectedId 当前选中项
 * - toast 浮动消息
 *
 * 业务逻辑（loadAll、save、delete）由各 view 自行实现，composable 只负责
 * 状态变量和通用辅助函数（showToast、wrap with status）。
 */

import { ref } from 'vue'
import { logger } from '@/utils/logger'
import { useToast } from '@/composables/useToast'

export type ToastType = 'success' | 'error' | 'info'

export interface Toast {
  type: ToastType
  text: string
}

export function useResourceManager(options: {
  /** 日志 tag（用于区分不同 view） */
  logTag: string
  /** toast 默认显示时长（ms），默认 3000 */
  toastDuration?: number
} = { logTag: 'resource' }) {
  const loading = ref(false)
  const saving = ref(false)
  const testing = ref(false)
  const creating = ref(false)
  const selectedId = ref<string | null>(null)
  const deletingId = ref<string | null>(null)

  const { showToast: showGlobalToast } = useToast()

  /**
   * 显示一条浮动消息（委托给全局单例，见 composables/useToast.ts）
   */
  function showToast(type: ToastType, text: string) {
    showGlobalToast(type, text)
  }

  /**
   * 用 try/finally 包裹一个操作，自动管理 saving/loading 状态
   *
   * 用法：
   * ```ts
   * const data = await withStatus('saving', async () => {
   *   return await callPlugin(...)
   * })
   * ```
   */
  async function withStatus<T>(
    which: 'loading' | 'saving' | 'testing',
    fn: () => Promise<T>,
  ): Promise<T | undefined> {
    const flag = which === 'loading' ? loading
      : which === 'saving' ? saving
      : testing
    flag.value = true
    try {
      return await fn()
    } catch (err) {
      logger.error(options.logTag, `${which} failed:`, err)
      showToast('error', `${actionName(which)}失败: ${err}`)
      return undefined
    } finally {
      flag.value = false
    }
  }

  /** 进入"新建"模式 */
  function enterCreateMode() {
    creating.value = true
    selectedId.value = null
  }

  /** 选中一个已存在的资源 */
  function select(id: string) {
    creating.value = false
    selectedId.value = id
  }

  /** 标记删除中（按 id） */
  function markDeleting(id: string | null) {
    deletingId.value = id
  }

  return {
    // 状态
    loading,
    saving,
    testing,
    creating,
    selectedId,
    deletingId,

    // 方法
    showToast,
    withStatus,
    enterCreateMode,
    select,
    markDeleting,
  }
}

function actionName(which: 'loading' | 'saving' | 'testing'): string {
  switch (which) {
    case 'loading': return '加载'
    case 'saving': return '保存'
    case 'testing': return '测试'
  }
}

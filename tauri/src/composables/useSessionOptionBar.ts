/**
 * 会话选项栏 —— 取值与落库（机制唯一实现）
 *
 * ## 定位
 *
 * 会话输入区下方的「选项行」由后端**随会话节点下发定义**（`node.schema`；
 * 新建草稿态走 `<根>/session` 的 `new_type.schema`）。本组合式函数实现**机制**：
 *
 * - 把定义里的字段拍平（`sections[].fields` → 一个有序数组）；
 * - 把一次字段选择落库 —— 会话态经 `vdfs/write(<根>/session/<id>, {"metadata": …})`
 *   （后端 `Session::merge_metadata_object` 是浅合并的唯一实现）；
 * - 草稿态（会话尚不存在）把补丁缓冲在 `draftMetadata`，由新建流程在创建会话时
 *   一次性写入——选择项与真实会话完全同构。
 *
 * ## 零业务
 *
 * 本文件不含任何业务字段名（workdir / agent_id / risk_level…）：它们全部由后端在
 * 定义里声明，前端只搬运 `field.key` 与字段值。**没有网络层**——旧的
 * `services/options.ts`（`options/list` 拉取）随旧机制一并下线。
 *
 * ## 为什么保存后要做本地镜射
 *
 * 后端写入后确实会广播 `vdfs` 变更，但那条链路是**防抖重拉清单**（800ms），
 * 让按钮上的当前值等一个往返才更新是可见的迟钝。故写成功后把同一份补丁
 * 镜射进会话 store（`applySessionMetadataPatch`，与后端浅合并语义一致），
 * 后端事件随后幂等收敛。
 */

import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { updateSession } from '@/services/session'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import type { SessionMetadata } from '@/schemas/session_meta'
import type { DetailDefinition, DetailField } from '@/schemas/vdfs'

export interface UseSessionOptionBar {
  /** 定义里的全部字段（按分区顺序拍平；定义缺失时为空数组） */
  fields: ComputedRef<DetailField[]>
  /** 保存进行中（驱动选项栏整体禁用，避免并发写同一会话） */
  busy: Ref<boolean>
  /** 草稿态已缓冲的 metadata 补丁（无会话时的选择暂存） */
  draftMetadata: Ref<Record<string, unknown>>
  /** 写入一个字段值：会话态落库 + 本地镜射；草稿态缓冲 */
  save: (key: string, value: unknown) => Promise<void>
}

/**
 * @param sessionId  当前会话 id 的取值函数；返回空 = 草稿态（新建会话前）
 * @param definition 当前定义（会话节点 `schema` / `new_type.schema`）
 */
export function useSessionOptionBar(opts: {
  sessionId: () => string | undefined
  definition: () => DetailDefinition | null | undefined
}): UseSessionOptionBar {
  const store = useSessionsStore()
  const busy = ref(false)
  const draftMetadata = ref<Record<string, unknown>>({})

  const fields = computed<DetailField[]>(() =>
    (opts.definition()?.sections ?? []).flatMap((s) => s.fields ?? []),
  )

  const hasSession = (): boolean => Boolean(opts.sessionId()?.trim())

  async function save(key: string, value: unknown): Promise<void> {
    const patch = { [key]: value }

    // 草稿态：无落库目标 → 缓冲补丁，创建会话时一次写入
    if (!hasSession()) {
      draftMetadata.value = { ...draftMetadata.value, ...patch }
      return
    }

    const sid = opts.sessionId()!.trim()
    busy.value = true
    try {
      await updateSession(sid, patch as SessionMetadata)
      store.applySessionMetadataPatch(sid, patch)
    } catch (e) {
      logger.error('useSessionOptionBar', `选项「${key}」保存失败`, e)
      throw e
    } finally {
      busy.value = false
    }
  }

  return { fields, busy, draftMetadata, save }
}

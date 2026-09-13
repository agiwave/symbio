<!--
  VdfsSessionDetail — VDFS `session` 渲染器（会话工作区）

  薄适配层：会话资源的呈现已由 `components/entities/Session.vue` 实现，
  VDFS 侧只需把节点映射为实体摘要并透传能力。同一份渲染器同时服务
  实体机制与 VDFS 机制——这正是「ext 决定渲染器」这一约定的价值：
  详情实现只有一份，两个机制共用。
-->
<template>
  <Session
    :item="item"
    :capabilities="capabilities"
    :mechanism-actions="mechanismActions"
    :saving="saving"
    @created="(id) => $emit('created', id)"
    @delete="$emit('delete')"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import Session from '@/components/entities/Session.vue'
import type { DetailAction, EntityCapabilities, EntitySummary } from '@/schemas/entities'
import { vdfsAccessOf, type VdfsFieldError, type VdfsNode } from '@/schemas/vdfs'

// 渲染器统一契约（详见 VdfsTextDetail 同名说明）
const props = defineProps<{
  node: VdfsNode
  data?: unknown
  error?: string
  fieldErrors?: VdfsFieldError[]
  saving?: boolean
}>()

defineEmits<{
  (e: 'created', id: string): void
  (e: 'delete'): void
  (e: 'save', payload: unknown): void
  (e: 'rename'): void
}>()

const item = computed<EntitySummary>(() => ({
  kind: props.node.kind || 'session',
  id: props.node.name,
  name: props.node.title || props.node.name,
  description: props.node.description,
  status: props.node.status,
  updated_at: props.node.updated_at,
}))

const capabilities = computed<EntityCapabilities>(() => {
  const access = vdfsAccessOf(props.node)
  return {
    zip_upload: false,
    independent_form: true,
    realtime_status: true,
    refreshable: true,
    mutable: access.write,
    test_connection: false,
    read_only: !access.write,
  }
})

/**
 * 机制动作注入（在 ChatMainPanel 头部与自身按钮并排渲染）。
 *
 * 能力判据是节点的访问位（`w` = 可写 ⇒ 可删），与实体机制同构：
 * 删除请求经 `@delete` 回到页面层，由页面统一走 `vdfs/delete`
 * （`VdfsProvider::delete` → `delete_session_internal`），本组件不直接发协议。
 */
const mechanismActions = computed<DetailAction[]>(() => {
  if (!capabilities.value.mutable) return []
  return [{ id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' }]
})
</script>

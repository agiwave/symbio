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
    @open-container="$emit('browse')"
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
  /** 进入会话内部（子会话 / 工作目录树）——由页面层 `enter(节点路径)` 完成 */
  (e: 'browse'): void
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

/** 访问位 → 能力（VDFS 里能力就是访问位，不存在类型特判） */
const capabilities = computed<EntityCapabilities>(() => ({
  mutable: vdfsAccessOf(props.node).write,
  test_connection: false,
}))

/**
 * 机制动作注入（在 ChatMainPanel 头部与自身按钮并排渲染）。
 *
 * - **浏览内部**：会话内部结构（子会话 / 工作目录树）在 VDFS 上是「会话同名
 *   目录」，由页面层 `enter(node.path)` 进入——取代原容器实体页。只读能力，
 *   与访问位无关，恒可见。
 * - **删除**：能力判据是节点的访问位（`w` = 可写 ⇒ 可删），与实体机制同构；
 *   删除请求经 `@delete` 回到页面层，统一走 `vdfs/delete`
 *   （`VdfsProvider::delete` → `delete_session_internal`）。
 *
 * 本组件不直接发协议——动作一律回到页面层执行。
 */
const mechanismActions = computed<DetailAction[]>(() => {
  const out: DetailAction[] = [
    { id: 'open-container', label: '浏览内部', style: 'primary', payload: { kind: 'session' } },
  ]
  if (capabilities.value.mutable) {
    out.push({ id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' })
  }
  return out
})
</script>

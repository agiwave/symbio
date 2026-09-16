<!--
  VdfsSessionDetail — VDFS `session` 渲染器（会话工作区）

  薄适配层：会话的呈现已由 `components/vdfs/Session.vue` 实现，本组件只做两件事
  —— 把节点原样交给它（不再映射成另一种摘要形状），以及把访问位翻译成能力与
  机制动作。这正是「ext 决定渲染器」这一约定的价值：详情实现只有一份。
-->
<template>
  <Session
    :node="node"
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
import Session from './Session.vue'
import { vdfsAccessOf, type DetailAction, type VdfsFieldError, type VdfsNode } from '@/schemas/vdfs'

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

/** 访问位 → 能力（VDFS 里能力就是访问位，不存在类型特判） */
const capabilities = computed<Record<string, boolean>>(() => ({
  mutable: vdfsAccessOf(props.node).write,
  test_connection: false,
}))

/** 是否已落盘的会话（判据 = 有没有 id；草稿态没有名字，也就没有 id） */
const hasId = computed(() => Boolean(props.node?.name))

/**
 * 机制动作注入（在 ChatMainPanel 头部与自身按钮并排渲染）。
 *
 * - **浏览内部**：会话的内部结构（子会话 / 工作目录树）在 VDFS 上是「会话同名
 *   目录」，由页面层 `enter(node.path)` 进入。只读能力，
 *   与访问位无关，恒可见。
 * - **删除**：能力判据是节点的访问位（`w` = 可写 ⇒ 可删）；
 *   删除请求经 `@delete` 回到页面层，统一走 `vdfs/delete`
 *   （`VdfsProvider::delete` → `delete_session_internal`）。
 *
 * **草稿态（新建）两者都不给**：还没有落盘的东西，既无内部可浏览，也无从删除。
 *
 * 本组件不直接发协议——动作一律回到页面层执行。
 */
const mechanismActions = computed<DetailAction[]>(() => {
  if (!hasId.value) return []
  const out: DetailAction[] = [
    { id: 'open-container', label: '浏览内部', style: 'primary', payload: { kind: 'session' } },
  ]
  if (capabilities.value.mutable) {
    out.push({ id: 'delete', label: '删除', style: 'danger', busy_label: '删除中…' })
  }
  return out
})
</script>

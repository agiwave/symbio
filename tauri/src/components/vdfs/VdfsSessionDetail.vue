<!--
  VdfsSessionDetail — VDFS `session` 渲染器（会话工作区）

  薄适配层：会话的呈现已由 `components/vdfs/Session.vue` 实现，本组件只做两件事
  —— 把节点原样交给它（不再映射成另一种摘要形状），以及声明会话**自有**的动作。

  ## 动作归属

  - **浏览内部**（自有）：会话的内部结构（子会话 / 工作目录树）在 VDFS 上是
    「会话同名目录」，由页面层 `browseInto` 进入。纯导航、只读能力，与访问位
    无关，恒可见；草稿（新建态）没有内部可浏览，故不给。
  - **重命名 / 删除**（机制）：由页面按访问位单点算好，经 `mechanism-actions`
    注入——本组件不再自己算一遍（那曾是同一组动作的第 2 份实现）。
    会话是 `<根>` 系统资源，机制因此只注入「删除」，重命名由聊天头部按
    **会话标题**语义承载（改名与改标题不是一回事）。

  这正是「ext 决定渲染器」这一约定的价值：详情实现只有一份。
-->
<template>
  <Session
    :node="node"
    :actions="actions"
    :mechanism-actions="mechanismActions"
    :mechanism-busy="mechanismBusy"
    :saving="saving"
    @created="(id) => $emit('created', id)"
    @delete="$emit('delete')"
    @open-container="$emit('browse')"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import Session from './Session.vue'
import type { VdfsRendererProps } from './rendererContract'
import { isVdfsDraft, type DetailAction } from '@/schemas/vdfs'

const props = defineProps<VdfsRendererProps>()

defineEmits<{
  (e: 'created', id: string): void
  (e: 'delete'): void
  /** 进入会话内部（子会话 / 工作目录树）——由页面层 `browseInto(节点路径)` 完成 */
  (e: 'browse'): void
  (e: 'save', payload: unknown): void
  (e: 'rename'): void
}>()

/** 会话自有动作：浏览内部（草稿态没有内部可浏览） */
const actions = computed<DetailAction[]>(() =>
  isVdfsDraft(props.node)
    ? []
    : [{ id: 'open-container', label: '浏览内部', style: 'primary', payload: { kind: 'session' } }]
)
</script>

<!--
  VdfsSessionDetail — VDFS `session` 渲染器（会话工作区）

  薄适配层：会话的呈现已由 `components/vdfs/Session.vue` 实现，本组件只做两件事
  —— 把节点原样交给它（不再映射成另一种摘要形状），以及把会话**自有动作**投影出来。

  ## 动作归属

  - **自有动作**（`进入下一级`）**不在本组件声明**：它由后端随会话定义下发
    （`plugins/session/options::session_detail_actions`，与会话选项同一份 `schema`）。
    本组件只做**投影**——按机制键 `is_existing` 求值 `when` / `disabled_when`
    （唯一实现 `projectDetailActions`），草稿（新建态）因此自然不出现它。
    这与 agent / model / mcp 的 `detail_definition` 是同一条通道：**后端声明动作，
    前端只渲染**，本组件不再硬编码任何一条类型专属动作。
  - **重命名 / 删除**（机制）：由页面按访问位单点算好，经 `mechanism-actions`
    注入——本组件不再自己算一遍（那曾是同一组动作的第 2 份实现）。
    会话是 `<根>` 系统资源，机制因此只注入「删除」，重命名由聊天头部按
    **会话标题**语义承载（改名与改标题不是一回事）。

  这正是「ext 决定渲染器」这一约定的价值：详情实现只有一份。
-->
<template>
  <Session
    :node="node"
    :actions="projected.actions"
    :action-disabled="projected.disabled"
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
import { isVdfsDraft, type DetailDefinition } from '@/schemas/vdfs'
import { mechanismDetailValueOf, projectDetailActions } from '@/schemas/vdfs-form'

const props = defineProps<VdfsRendererProps>()

defineEmits<{
  (e: 'created', id: string): void
  (e: 'delete'): void
  /** 进入会话内部（子会话 / 工作目录树）——由页面层 `browseInto(节点路径)` 完成 */
  (e: 'browse'): void
  (e: 'save', payload: unknown): void
}>()

/**
 * 会话自有动作 = 会话节点 `schema` 里**后端声明的**动作（经 `when` / `disabled_when`
 * 投影）。
 *
 * 作用域只有机制键 `is_existing`（`schemas/vdfs-form.mechanismDetailValueOf`）：
 * 已落盘 = true、草稿 = false。于是「草稿不出现『进入下一级』」这条判据由**定义**
 * 表达（`when = is_existing`），前端不再按草稿特判。
 */
const projected = computed(() =>
  projectDetailActions(
    (props.node.schema as DetailDefinition | undefined)?.actions,
    mechanismDetailValueOf(!isVdfsDraft(props.node)),
  ),
)
</script>

<!--
  上下文压缩渲染器 —— 系统对历史的一次整理动作（**不是对话内容**）

  后端把压缩作为**消息节点**下发，而不是挂在窗口顶部的横幅。理由是位置与可回溯：
  它有自己在该轮中的位置（当前时刻）与状态（进行中 / 已完成），用户滚动到消息流
  中间也能看见「这里压缩过」，事后也能回溯。横幅做不到——它没有位置概念，滚动即消失。

  正文由后端给出：运行中「正在压缩上下文…」，完成后「已压缩上下文（X → Y 条）」。
  呈现上刻意弱化（`.compress-note`）：它是系统动作，不与对话正文争视觉重量。
-->
<template>
  <NodeShell
    :node="node"
    :facets="facets"
    :depth="depth"
    @retry="emit('retry', $event)"
    @delete="emit('delete', $event)"
    @edit="emit('edit', $event)"
  >
    <div class="compress-note">{{ text }}</div>
  </NodeShell>
</template>

<script setup lang="ts">
import type { ChatMessage } from '@/schemas/chat_message'
import { useMessageContent } from '@/composables/useMessageContent'
import type { MessageFacets } from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'

const props = defineProps<{
  node: ChatMessage
  facets: MessageFacets
  depth?: number
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const { text } = useMessageContent(
  () => props.node,
  () => props.facets,
)
</script>

<style scoped>
/* 压缩节点的正文：弱化到"系统动作"的观感，不与对话正文争视觉重量。
   不引入新的主题变量——压缩是低频节点，跟随父级文字色即可。 */
.compress-note {
  padding: 0.2rem 0 0.1rem;
  font-size: 0.82rem;
  opacity: 0.75;
}
</style>

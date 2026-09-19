<!--
  上下文压缩渲染器 —— 系统对历史的一次整理动作（**不是对话内容**）

  后端把压缩作为**消息节点**下发，而不是挂在窗口顶部的横幅。理由是位置与可回溯：
  它有自己在该轮中的位置（当前时刻）与状态（进行中 / 已完成），用户滚动到消息流
  中间也能看见「这里压缩过」，事后也能回溯。横幅做不到——它没有位置概念，滚动即消失。

  正文由后端给出：运行中「正在压缩上下文…」，完成后「已压缩上下文（X → Y 条）」。

  ## 两形态：常规（弱化）与失败（错误条 + 重试）

  压缩成功是系统动作，呈现上刻意弱化（`.compress-note`），不与对话正文争视觉重量。

  压缩**失败**则是另一回事：后端把「模型请求失败 / 模型没按格式输出 / 待压缩历史
  超出模型输入上限」连同**原因**写进节点正文，并且**一条历史都没动**。用户需要
  看到原因并决定下一步（换更大上下文的模型后重试 / 手动精简会话），所以这一形态
  走 `MessageErrorBox`：故障红 + 「重试」入口，与 Turn / 工具失败同一套观感与语义
  （重试 → resume `retry_compaction`，见 `registry/messageTypes.messageRetryTargetOf`）。

  失败形态**绝不复用弱化样式**——把"压缩没做成"渲染成一行淡淡的灰字，等于把
  一个需要用户决策的状态伪装成无事发生。
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
    <MessageErrorBox v-if="failed" :text="text" retryable @retry="emit('retry', node.id)" />
    <div v-else class="compress-note">{{ text }}</div>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ChatMessage } from '@/schemas/chat_message'
import { useMessageContent } from '@/composables/useMessageContent'
import { canRetryCompaction, type MessageFacets } from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageErrorBox from './MessageErrorBox.vue'

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

/** 失败判定与「能否重试」都来自注册表——本组件不解释 `status` 取值 */
const failed = computed(() => canRetryCompaction(props.facets))
</script>

<style scoped>
/* 压缩节点的正文：弱化到"系统动作"的观感，不与对话正文争视觉重量。
   不引入新的主题变量——压缩是低频节点，跟随父级文字色即可。
   注意：**仅常规形态**用这个样式，失败形态走 MessageErrorBox（见文件头）。 */
.compress-note {
  padding: 0.2rem 0 0.1rem;
  font-size: 0.82rem;
  opacity: 0.75;
}
</style>

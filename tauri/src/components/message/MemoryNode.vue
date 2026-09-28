<!--
  MemoryNode — 压缩后的**历史记忆**渲染器（系统产出的记忆，不是用户说的话）

  ## 它修的是什么

  后端把压缩后的记忆写成一条 `role = user` 的消息（多数 provider 要求对话以 user
  开头），正文以 `[CONTEXT SNAPSHOT — …]` 开头，并在 `meta.compacted` 上打标。
  没有这条渲染器时，它按「角色优先」被渲染成右对齐的**用户气泡**、头像是「你」——
  「系统替你说了一句话」是这里最容易被误读的一种呈现。

  ## 呈现三条

  - **默认收起**（策略在 `registry/messageTypes.messageDefaultOpen`）：摘要可以很长，
    展开会盖过对话本体，而折叠态的标题（「历史记忆」）已经把它交待清楚了；
  - **正文原样显示**：那就是模型看到的那份记忆，前端不改写它、也不删掉
    正文里的内部标记——这一页的意义是「可审计」，替用户修饰过的记忆就不可审计了；
  - **底下给事实行**：压缩后水位与「完整历史已转存」（字段存在才显示，
    见 `registry/messageTypes.messageMemoryStats`）。

  ## 没有的操作

  不给「编辑 / 删除」：它是系统记忆，改动它应当是显式动作，不该挂在悬停里
  （判据在 `NodeShell` —— 渲染器只声明自己特有的动作）。
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
    <div class="memory-body">
      <pre class="memory-text">{{ text }}</pre>
      <p v-if="facts.length" class="memory-facts">{{ facts.join(' · ') }}</p>
    </div>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ChatMessage } from '@/schemas/chat_message'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  formatTokens,
  messageMemoryStats,
  type MessageFacets,
} from '@/registry/messageTypes'
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

/** 事实行（有字段才出条目——旧后端 / 旧会话因此什么都不多显示） */
const facts = computed<string[]>(() => {
  const s = messageMemoryStats(props.node)
  const parts: string[] = []
  if (s.postTokens != null) parts.push(`压缩后水位 ${formatTokens(s.postTokens)}`)
  if (s.archived) parts.push('完整历史已转存')
  return parts
})
</script>

<style scoped>
/* 记忆体：弱化到「系统材料」的观感，不与对话正文争视觉重量。
   正文等宽 + 限高内滚：摘要里常有结构化条目，等宽更易读；限高保证
   展开它不会把对话本体推走。 */
.memory-body {
  padding: 0.2rem 0 0.1rem;
}

.memory-text {
  max-height: 16rem;
  overflow: auto;
  margin: 0;
  padding: 0.5rem 0.6rem;
  border-radius: 0.375rem;
  background: var(--surface-sunken);
  color: var(--text-secondary);
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.78rem;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.memory-facts {
  margin: 0.3rem 0 0;
  font-size: 0.74rem;
  color: var(--text-muted);
}
</style>

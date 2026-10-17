<!--
  折叠叶渲染器 —— `type = notice` 的全部形态（[T-155 B2a]，[ADR-070 §后果 2]）

  一个可折叠行（图标 + 标题 + 收起态摘要）+ 展开后的正文。思考 / 上下文压缩 /
  压缩后的历史记忆**同在这一格**——前端对它们渲染完全一样，不认「这坨叫思考还是压缩」。
  「是思考还是压缩」这一业务差异由后端在交互面折成 `title` / `icon` 数据（头部据此显示），
  本组件只按**形状 + 已有谓词**决定折叠体里放什么：

  - **压缩后的历史记忆**（`facets.compacted`，来自 `meta.compacted`）：正文原样显示
    （那是模型看到的那份记忆，前端不改写、不删内部标记——这一页的意义是可审计），
    底下给事实行（压缩后水位 / 完整历史已转存）。它是系统产出的记忆，**不是用户说的话**：
    分派器已把它路由到本折叠叶，而不是右对齐的用户气泡。
  - **一次压缩动作**（携带压缩统计字段）：正文一行系统说明 + 事实行；失败则走
    错误条 +「重试」（resume `retry_compaction`，判据来自后端下发的 `actions`）。
  - **其余**（思考 / 未知折叠叶）：Markdown 正文，默认收起成单行、流式中呼吸动效
    （动效与标题都由注册表按形态给出，本组件不判断）。

  读取 `meta` 字段一律经 `registry/messageTypes` 的谓词（`messageCompactionStats` /
  `messageMemoryStats`）——字段名只在那里出现，组件不碰后端字段名。
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
    <!-- 压缩后的历史记忆：正文原样（可审计）+ 事实行 -->
    <div v-if="facets.compacted" class="notice-memory">
      <pre class="notice-pre">{{ text }}</pre>
      <p v-if="memoryFacts.length" class="notice-facts">{{ memoryFacts.join(' · ') }}</p>
    </div>

    <!-- 一次压缩动作：失败给错误条 + 重试；成功给系统说明 + 事实行 -->
    <div v-else-if="isCompaction">
      <MessageErrorBox
        v-if="canRetryCompaction(facets)"
        :text="text"
        retryable
        @retry="emit('retry', node.id)"
      />
      <template v-else>
        <div class="notice-note">{{ text }}</div>
        <p v-if="compactionFacts.length" class="notice-facts">{{ compactionFacts.join(' · ') }}</p>
      </template>
    </div>

    <!-- 其余折叠叶（思考 / 未知）：Markdown 正文 -->
    <div v-else class="markdown-body" v-html="rendered" />
  </NodeShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ChatNode } from '@/schemas/chat_node'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  canRetryCompaction,
  compactionTriggerLabel,
  formatTokens,
  messageCompactionStats,
  messageMemoryStats,
  type MessageFacets,
} from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageErrorBox from './MessageErrorBox.vue'

const props = defineProps<{
  node: ChatNode
  facets: MessageFacets
  depth?: number
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const { text, rendered } = useMessageContent(
  () => props.node,
  () => props.facets,
)

/**
 * 是否「一次压缩动作」——判据是**携带压缩统计字段**（后端随 `meta` 下发）。
 * 缺字段（思考 / 未知折叠叶）一律不算压缩，落 Markdown 那一支。
 */
const isCompaction = computed(() => {
  const s = messageCompactionStats(props.node)
  return (
    s.trigger !== undefined ||
    s.limit !== undefined ||
    s.beforeTokens !== undefined ||
    s.afterTokens !== undefined ||
    s.dropped !== undefined
  )
})

/** 压缩事实行（缺字段就少一项，旧后端照常只显示正文那一行） */
const compactionFacts = computed<string[]>(() => {
  const s = messageCompactionStats(props.node)
  const parts: string[] = []
  const water = s.afterTokens ?? s.beforeTokens
  if (water != null) {
    parts.push(`${s.afterTokens != null ? '压缩后水位' : '当前水位'} ${formatTokens(water)}`)
  }
  if (s.limit != null) parts.push(`上限 ${formatTokens(s.limit)}`)
  if (s.trigger) parts.push(compactionTriggerLabel(s.trigger))
  if (s.dropped != null) parts.push(`压掉 ${s.dropped} 条`)
  return parts
})

/** 记忆事实行（有字段才出条目——旧后端 / 旧会话因此什么都不多显示） */
const memoryFacts = computed<string[]>(() => {
  const s = messageMemoryStats(props.node)
  const parts: string[] = []
  if (s.postTokens != null) parts.push(`压缩后水位 ${formatTokens(s.postTokens)}`)
  if (s.archived) parts.push('完整历史已转存')
  return parts
})
</script>

<style scoped>
/* 折叠叶正文：弱化到「系统材料」的观感，不与对话正文争视觉重量。 */
.notice-note {
  padding: 0.2rem 0 0.1rem;
  font-size: 0.82rem;
  opacity: 0.75;
}
/* 记忆体 / 压缩正文的等宽块：限高内滚，展开它不把对话本体推走。 */
.notice-memory {
  padding: 0.2rem 0 0.1rem;
}
.notice-pre {
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
.notice-facts {
  margin: 0.3rem 0 0;
  font-size: 0.74rem;
  color: var(--text-muted);
}
</style>

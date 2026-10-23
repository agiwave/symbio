<!--
  折叠叶渲染器 —— `type = notice` 的全部形态（[T-155 B2a]，[ADR-070 §决策 3]）

  一个可折叠行（图标 + 标题 + 收起态摘要）+ 展开后的正文。后端有多少种业务往这一格里写，
  前端都只有一格：叫什么、是什么颜色、配什么图标都由后端折成 `title` / `icon` 数据下发，
  本组件不认任何业务名，只按**已有哪些数据**决定折叠体里放什么：

  - **带记忆字段**（`facets.compacted`，来自 `meta.compacted`）：正文原样显示
    （那是模型看到的那份材料，前端不改写、不删内部标记——这一页的意义是可审计），
    底下给事实行（水位 / 完整历史已转存）。它是系统产出的，**不是用户说的话**：
    分派器已把它路由到本折叠叶，而不是右对齐的用户气泡。
  - **带读数行**（`node.stats` 有行）：正文一行说明 + 后端排好的事实行；
    失败则走错误条 +「重试」（恢复动作判据来自后端下发的 `actions`）。
  - **其余**：Markdown 正文，默认收起成单行、流式中呼吸动效
    （动效与标题都由注册表按形态给出，本组件不判断）。

  「水位怎么单位、触发叫什么词、压掉几条」都不在本组件——它们由后端在交付出口排成
  `stats.rows` 逐行下发（[D-111](../../../symbio/src/plugins/session/README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端)），本组件只做「有行就逐行排版」。
  仍要读 `meta` 的字段一律经 `registry/messageTypes` 的谓词，组件不碰后端字段名。
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
      <p v-if="factRows.length" class="notice-facts">{{ factRows.join(' · ') }}</p>
    </div>

    <!-- 一次压缩动作：失败给错误条 + 重试；成功给系统说明 + 事实行 -->
    <template v-else-if="factRows.length">
      <MessageErrorBox
        v-if="canRetryCompaction(facets)"
        :text="text"
        retryable
        @retry="emit('retry', node.id)"
      />
      <template v-else>
        <div class="notice-note">{{ text }}</div>
        <p class="notice-facts">{{ factRows.join(' · ') }}</p>
      </template>
    </template>

    <!-- 其余折叠叶：Markdown 正文 -->
    <div v-else class="markdown-body" v-html="rendered" />
  </NodeShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ChatNode } from '@/schemas/chat_node'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  canRetryCompaction,
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
 * 后端排好的**事实行**（`node.stats.rows`，[D-111](../../../../symbio/src/plugins/session/README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "symbio/src/plugins/session/README")）。
 *
 * 空数组 = 这个节点没有可报的数，折叠体因此只是一块正文——本组件不问它是什么业务，
 * 也不做数值格式化（单位与精度在产者那一侧定，见 `schemas/reading.ts`）。
 * 只有标签没有值的行（如「完整历史已转存」这类成对写法留给后端拆成 label + value）
 * 照常拼：值为空就不重复一个空格。
 */
const factRows = computed<string[]>(() =>
  (props.node.stats?.rows ?? []).map((r) => (r.value ? `${r.label} ${r.value}` : r.label)),
)
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

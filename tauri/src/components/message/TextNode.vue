<!--
  文本类渲染器 —— 折叠式内容节点：**用户气泡 / 思考 / 正文 / 工具请求 / 工具返回**

  这五种在结构上是同一件事（一个可折叠的头部 + 一段内容），差别只在内容怎么摆，
  因此共用一个渲染器、由 `facets` 分派到不同内容分支，而不是拆成五个组件
  （拆开会让「头部怎么画」跟着复制五遍）。

  内容分派（顺序有语义，与改造前的模板链一致）：
  1. 用户消息 → 右对齐气泡；
  2. 思考 → 单行行头 + 展开后的 Markdown（默认收起，见折叠策略）；
  3. 其余（正文 / 合成请求 / 工具返回）：
     · 自身失败的工具返回 → 交代条（父 ToolCall 已失败时不再重复报错）；
     · 内容为完整 JSON（或工具返回）→ 代码块高亮（请求参数加蓝色强调边）；
     · 否则 → Markdown 正文。

  「纯文本 / 思考节点**绝不**渲染服务端错误」：服务器错误只属于根级 Turn，
  工具调用在本地执行不会触发 API 错误，因此这里只在工具返回上呈现失败。
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
    <!-- 用户消息：右对齐气泡 -->
    <div v-if="isUser" class="user-row">
      <div class="bubble user-bubble">
        <div class="markdown-body" v-html="rendered" />
      </div>
    </div>

    <!-- 思考过程 -->
    <div v-else-if="isReasoning" class="markdown-body" v-html="rendered" />

    <!-- 正文 / 工具请求 / 工具返回 -->
    <template v-else>
      <MessageErrorBox v-if="showsOwnError" :text="errorText" />
      <pre
        v-else-if="renderAsJson || facets.toolResult"
        :class="['json', facets.toolRequest ? 'req' : '']"
        v-html="highlighted"
      />
      <div v-else class="markdown-body" v-html="rendered" />
    </template>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import {
  CHAT_ROLE_USER,
  MESSAGE_TYPE_REASONING,
  type ChatMessage,
} from '@/schemas/chat_message'
import { useMessageContent } from '@/composables/useMessageContent'
import { isFailedStatus, type MessageFacets } from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageErrorBox from './MessageErrorBox.vue'

const props = defineProps<{
  node: ChatMessage
  facets: MessageFacets
  depth?: number
  /** 父节点（ToolCall）是否已失败：失败只由"造成中止的节点"呈现一次 */
  parentFailed?: boolean
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const isUser = computed(() => props.facets.role === CHAT_ROLE_USER)
const isReasoning = computed(() => props.facets.type === MESSAGE_TYPE_REASONING)

/** 工具返回自身的失败：父 ToolCall 已失败时交由父级呈现，避免同一错误报两遍 */
const showsOwnError = computed(
  () => isFailedStatus(props.facets.status) && props.facets.toolResult && !props.parentFailed,
)

const { highlighted, rendered, renderAsJson, errorText } = useMessageContent(
  () => props.node,
  () => props.facets,
)
</script>

<style scoped>
/* ── 用户消息气泡 ── */
.user-row {
  display: flex;
  justify-content: flex-end;
}
/* 气泡基类：结构属性在此，配色/圆角由 user-bubble 等变体提供 */
.bubble {
  max-width: 80%;
  padding: 0.6rem 0.85rem;
  word-break: break-word;
}
.user-bubble {
  max-width: 80%;
  background: var(--color-user-bubble);
  color: var(--color-user-bubble-fg);
  border-radius: 0.875rem 0.875rem 0.25rem 0.875rem;
  padding: 0.6rem 0.85rem;
}
/* 气泡内的 markdown 直接继承白色，避免被全局深色 .markdown-body 规则覆盖。
   基样式在 `styles/markdown.css`（全局，(0,1,0)），此处 scoped 覆写为 (0,3,0)，
   特异性稳定压过基样式——不依赖样式表的先后顺序。 */
.user-bubble :deep(.markdown-body),
.user-bubble :deep(.markdown-body h1),
.user-bubble :deep(.markdown-body h2),
.user-bubble :deep(.markdown-body h3),
.user-bubble :deep(.markdown-body h4),
.user-bubble :deep(.markdown-body p),
.user-bubble :deep(.markdown-body li),
.user-bubble :deep(.markdown-body blockquote) {
  color: #fff;
}
.user-bubble :deep(.markdown-body a) {
  color: #fff;
  text-decoration: underline;
}
.user-bubble :deep(.markdown-body code) {
  background: rgba(255, 255, 255, 0.22);
  color: #fff;
}
.user-bubble :deep(.markdown-body pre) {
  background: rgba(15, 23, 42, 0.45);
  color: #e2e8f0;
}
.user-bubble :deep(.markdown-body pre code) {
  color: #e2e8f0;
}
.user-bubble :deep(.markdown-body blockquote) {
  border-left-color: rgba(255, 255, 255, 0.5);
}
.user-bubble :deep(.markdown-body p) {
  margin: 0 0 0.5rem;
}
.user-bubble :deep(.markdown-body > *:last-child) {
  margin-bottom: 0;
}
</style>

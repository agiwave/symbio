<!--
  正文叶渲染器 —— `type = body` 的全部形态

  折叠式内容节点的一个格：**用户气泡 / 助手正文 / 工具请求 / 工具返回**，
  以及未登记形态的只读兜底。它们在结构上是同一件事（一个可折叠头部 + 一段内容），
  差别只在内容怎么摆，因此共用一个渲染器、由 `facets` 的**上下文标记**（role /
  toolRequest / toolResult）分派，而不是拆成多个组件。

  **不含业务名**：这是 `body` 格，前端不认「这是思考还是正文」——思考属于折叠叶
  （`notice`），住在 `NoticeNode`。

  内容分派（顺序有语义）：
  1. 用户消息 → 右对齐气泡；
  2. 自身失败的工具返回 → 交代条（父骨架已失败时不再重复报错）；
  3. 完整 JSON（或工具返回）→ 代码块高亮（请求参数加蓝色强调边）；
  4. 否则 → Markdown 正文。
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
        <button
          v-if="text"
          class="copy-btn"
          type="button"
          :title="copied ? '已复制' : '复制正文'"
          :aria-label="copied ? '已复制' : '复制正文'"
          @click="copy"
        >{{ copied ? '✓' : '复制' }}</button>
      </div>
    </div>

    <!-- 正文 / 工具请求 / 工具返回 -->
    <template v-else>
      <MessageErrorBox v-if="showsOwnError" :text="errorText" />
      <!-- 请求视图（toolRequest）与工具返回同走 JSON 高亮：参数流式未闭合时
           高亮器退化为整段着色（比空白有用），不做 Markdown 渲染。 -->
      <div v-else-if="renderAsJson || facets.toolResult || facets.toolRequest" class="copy-wrap">
        <pre :class="['json', facets.toolRequest ? 'req' : '']" v-html="highlighted" />
        <button
          v-if="text"
          class="copy-btn"
          type="button"
          :title="copied ? '已复制' : '复制内容'"
          :aria-label="copied ? '已复制' : '复制内容'"
          @click="copy"
        >{{ copied ? '✓' : '复制' }}</button>
      </div>
      <div v-else class="copy-wrap">
        <div class="markdown-body" v-html="rendered" />
        <button
          v-if="text"
          class="copy-btn"
          type="button"
          :title="copied ? '已复制' : '复制正文'"
          :aria-label="copied ? '已复制' : '复制正文'"
          @click="copy"
        >{{ copied ? '✓' : '复制' }}</button>
      </div>
    </template>

    <!-- 子流：本形态不折叠它（收起用户气泡只收起气泡，回答留在流里）。
         按需转发——空槽同样会被外壳当成「有子流」而画出空区域。 -->
    <template v-for="name in ($slots.subtree ? ['subtree'] : [])" :key="name" #[name]>
      <slot name="subtree" />
    </template>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { type ChatNode } from '@/schemas/chat_node'
import { useMessageContent } from '@/composables/useMessageContent'
import { useToast } from '@/composables/useToast'
import { copyText } from '@/composables/useClipboard'
import { isFailedStatus, isUserFacet, type MessageFacets } from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageErrorBox from './MessageErrorBox.vue'

const props = defineProps<{
  node: ChatNode
  facets: MessageFacets
  depth?: number
  /** 父骨架是否已失败：失败只由"造成中止的节点"呈现一次 */
  parentFailed?: boolean
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const isUser = computed(() => isUserFacet(props.facets))

/** 工具返回自身的失败：父骨架已失败时交由父级呈现，避免同一错误报两遍 */
const showsOwnError = computed(
  () => isFailedStatus(props.facets.status) && props.facets.toolResult && !props.parentFailed,
)

const { text, highlighted, rendered, renderAsJson, errorText } = useMessageContent(
  () => props.node,
  () => props.facets,
)

const { showToast } = useToast()
const copied = ref(false)
let copiedTimer: ReturnType<typeof setTimeout> | null = null
async function copy() {
  const ok = await copyText(text.value)
  if (!ok) {
    showToast('error', '复制失败，请手动选中文本')
    return
  }
  copied.value = true
  if (copiedTimer) clearTimeout(copiedTimer)
  copiedTimer = setTimeout(() => {
    copied.value = false
    copiedTimer = null
  }, 1500)
}
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

/* 一键复制：贴着内容块右上角，悬停/聚焦才显形，不占正文空间 */
.copy-wrap {
  position: relative;
}
.copy-btn {
  position: absolute;
  top: 0.3rem;
  right: 0.3rem;
  opacity: 0;
  border: 1px solid var(--color-code-border, var(--border-default));
  background: var(--surface-panel);
  color: var(--text-secondary);
  border-radius: 0.3rem;
  display: inline-flex;
  align-items: center;
  min-height: 1.5rem;
  padding: 0.15rem 0.5rem;
  font-size: 0.7rem;
  cursor: pointer;
  transition: opacity 0.12s ease;
  z-index: 1;
}
.copy-wrap:hover > .copy-btn,
.copy-wrap:focus-within > .copy-btn,
.user-bubble:hover > .copy-btn,
.user-bubble:focus-within > .copy-btn,
.copy-btn:focus-visible {
  opacity: 1;
}
.copy-btn:hover {
  color: var(--text-primary);
}

/* 无悬停设备：复制键常显（理由同 NodeShell 的操作区——hover 在触屏上不存在）。
   但**不能沿用右上角浮层**：悬停态是瞬时的，常显态是永久的，浮层会一直压住正文
   第一行的末尾字符（实测 375/768 宽下盖住「…正常。」的尾部）。常显时它回到流内、
   排在正文之后，占自己的位置。 */
@media (hover: none) {
  .copy-btn {
    opacity: 1;
    position: static;
    display: flex;
    width: fit-content;
    margin: 0.25rem 0 0 auto;
    min-height: 1.75rem;
    /* 常显后每条消息都多一个框，视觉噪声比悬停态重得多：去掉边框和底色，
       只留一行可点的低对比度文字，触控目标仍是 1.75rem。 */
    border: none;
    background: transparent;
    color: var(--text-muted);
  }
}
.user-bubble {
  position: relative;
}
.user-bubble > .copy-btn {
  color: rgba(255, 255, 255, 0.85);
  border-color: rgba(255, 255, 255, 0.3);
  background: rgba(0, 0, 0, 0.15);
}
</style>

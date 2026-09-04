<template>
  <Transition name="ctx-fade">
    <div v-if="visible" class="context-card" :class="{ expanded }">
      <!-- 头部：标签 + 文件 + 行号 + 操作 -->
      <div class="ctx-header">
        <div class="ctx-header-left">
          <span class="ctx-icon" title="选中的代码上下文">✨</span>
          <span class="ctx-label">选中的内容</span>
          <span v-if="fileName" class="ctx-file" :title="context?.filePath">
            <span class="ctx-sep">·</span>
            <span class="ctx-file-icon">📄</span>
            <span class="ctx-file-name">{{ fileName }}</span>
          </span>
          <span v-if="lineRange" class="ctx-lines">
            <span class="ctx-sep">·</span>
            <span class="ctx-lines-icon">📍</span>
            <span>行 {{ lineRange }}</span>
          </span>
        </div>

        <div class="ctx-header-right">
          <span v-if="context?.sessionId && context.sessionId !== sessionId" class="ctx-session-warn" :title="`来源会话 ${context.sessionId}，当前会话 ${sessionId}`">
            跨会话
          </span>
          <button
            class="ctx-icon-btn"
            :title="expanded ? '收起' : '展开全文'"
            @click="expanded = !expanded"
          >
            <svg v-if="!expanded" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="6 9 12 15 18 9" />
            </svg>
            <svg v-else width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="6 15 12 9 18 15" />
            </svg>
          </button>
          <button
            class="ctx-icon-btn"
            title="从 AI 上下文中移除"
            @click="onClear"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      </div>

      <!-- 内容：选区文本（默认折叠，显示前几行） -->
      <div v-if="context?.selectedText" class="ctx-body" :class="{ 'is-expanded': expanded }" @click="onBodyClick">
        <pre class="ctx-code"><code>{{ context.selectedText }}</code></pre>
        <div v-if="!expanded && hasMore" class="ctx-fade-mask" />
      </div>

      <!-- 仅有文件，没有选区 -->
      <div v-else-if="context?.filePath" class="ctx-body ctx-body-file">
        <span class="ctx-file-large">📄 {{ context.filePath }}</span>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
/**
 * AI 上下文预览卡片
 *
 * 灵感来自 ModelSelectionDialog 的 selected-context 区块：
 * - 头部展示：标签 + 文件名 + 行号范围
 * - 主体：选区代码（等宽字体 + 卡片背景）
 * - 交互：
 *   · 点击主体 / 折叠按钮 → 展开/收起全文
 *   · 关闭按钮 → 从 AI 上下文中移除（resetModelContext）
 *   · 跨会话提示：当 context.sessionId 与当前 sessionId 不一致时显示
 */
import { computed, ref } from 'vue'
import { resetModelContext } from '@/composables/useModelContext'

const props = defineProps<{
  context: any | null
  visible: boolean
  sessionId?: string
}>()

const expanded = ref(false)

const fileName = computed(() => {
  const p = props.context?.filePath
  if (!p) return ''
  const parts = p.split(/[\\/]/).filter(Boolean)
  return parts.length > 2 ? '…/' + parts.slice(-2).join('/') : p
})

const lineRange = computed(() => {
  const { startLine, endLine } = props.context || {}
  if (!startLine) return ''
  return endLine && endLine !== startLine ? `${startLine}-${endLine}` : `${startLine}`
})

const PREVIEW_LINES = 3
const PREVIEW_LINE_CHARS = 80

const hasMore = computed(() => {
  const text = props.context?.selectedText || ''
  if (!text) return false
  const lines = text.split('\n')
  if (lines.length > PREVIEW_LINES) return true
  return lines.some((l: string) => l.length > PREVIEW_LINE_CHARS)
})

function onClear() {
  // 仅清空"选区"相关信息，保留文件路径（如果用户只想看代码上下文，可以再点 ×）
  resetModelContext()
}

function onBodyClick() {
  if (hasMore.value) expanded.value = !expanded.value
}
</script>

<style scoped>
/* 整体卡片：和 ModelSelectionDialog.selected-context 的色系保持一致 */
.context-card {
  position: relative;
  margin-bottom: 0.5rem;
  padding: var(--space-2) var(--space-3) 0.625rem;
  background: linear-gradient(135deg, rgba(124, 58, 237, 0.06), rgba(37, 99, 235, 0.06));
  border: 1px solid rgba(124, 58, 237, 0.18);
  border-radius: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
  transition: box-shadow 0.15s, border-color 0.15s;
}

.context-card:hover {
  border-color: rgba(124, 58, 237, 0.32);
  box-shadow: 0 2px 8px rgba(124, 58, 237, 0.08);
}

.context-card.expanded {
  background: linear-gradient(135deg, rgba(124, 58, 237, 0.09), rgba(37, 99, 235, 0.09));
}

/* 头部 */
.ctx-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  min-height: 1.375rem;
}

.ctx-header-left {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  min-width: 0;
  flex: 1;
  flex-wrap: wrap;
  font-size: var(--font-size-xs);
}

.ctx-header-right {
  display: flex;
  align-items: center;
  gap: var(--space-05);
  flex-shrink: 0;
}

.ctx-icon { font-size: 0.75rem; }

.ctx-label {
  font-size: 0.625rem;
  color: var(--color-chip-reasoning-fg);
  text-transform: uppercase;
  letter-spacing: 0.0313rem;
  font-weight: 700;
}

.ctx-sep {
  color: var(--color-text-muted);
  margin: 0 var(--space-05);
  opacity: 0.6;
}

.ctx-file,
.ctx-lines {
  display: inline-flex;
  align-items: center;
  gap: var(--space-05);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: var(--color-text-secondary);
}

.ctx-file-name {
  max-width: 11.25rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ctx-session-warn {
  font-size: 0.5625rem;
  color: var(--warning-fg);
  background: rgba(245, 158, 11, 0.12);
  border: 1px solid rgba(245, 158, 11, 0.32);
  border-radius: 0.1875rem;
  padding: 1px 0.3125rem;
  margin-right: var(--space-1);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.0188rem;
}

/* 操作按钮 */
.ctx-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.375rem;
  height: 1.375rem;
  border: none;
  background: transparent;
  border-radius: 0.25rem;
  cursor: pointer;
  color: var(--color-text-muted);
  transition: all 0.12s;
}

.ctx-icon-btn:hover {
  background: rgba(124, 58, 237, 0.12);
  color: var(--color-chip-reasoning-fg);
}

/* 主体：代码块 */
.ctx-body {
  position: relative;
  padding: 0.375rem 0.625rem;
  background: var(--color-supply-bg);
  border: 1px solid rgba(124, 58, 237, 0.1);
  border-radius: 0.3125rem;
  cursor: text;
  max-height: 3.75rem;
  overflow: hidden;
  transition: max-height 0.2s ease;
}

.context-card.expanded .ctx-body {
  max-height: 15rem;
  overflow: auto;
}

.ctx-body.is-expanded {
  cursor: default;
}

.ctx-code {
  margin: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.7188rem;
  line-height: 1.5;
  color: var(--text-primary);
  white-space: pre;
  word-break: normal;
  overflow-wrap: normal;
}

.ctx-code code { font-family: inherit; }

.ctx-fade-mask {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 1.375rem;
  background: linear-gradient(to bottom, transparent, var(--surface-fade));
  pointer-events: none;
}

.ctx-body-file {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--font-size-sm);
  color: var(--color-text-secondary);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.ctx-file-large {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 进入动画 */
.ctx-fade-enter-active,
.ctx-fade-leave-active {
  transition: opacity 0.18s, transform 0.18s;
}

.ctx-fade-enter-from,
.ctx-fade-leave-to {
  opacity: 0;
  transform: translateY(-0.25rem);
}
</style>

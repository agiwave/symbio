<template>
  <div class="notion-editor" ref="containerRef">
    <!-- 编辑器容器 -->
    <div ref="editorRef" class="editor-root"></div>
    
    <!-- 选中文本悬浮工具栏 -->
    <Teleport to="body">
      <Transition name="fade">
        <div v-if="showFloatingToolbar" class="floating-toolbar" :style="floatingToolbarStyle">
          <button @click="formatText('strong')" title="粗体 (Ctrl+B)">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M15.6 10.79c.97-.67 1.65-1.77 1.65-2.79 0-2.26-1.75-4-4-4H7v14h7.04c2.09 0 3.71-1.7 3.71-3.79 0-1.52-.86-2.82-2.15-3.42zM10 6.5h3c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5h-3v-3zm3.5 9H10v-3h3.5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5z"/></svg>
          </button>
          <button @click="formatText('em')" title="斜体 (Ctrl+I)">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M10 4v3h2.21l-3.42 8H6v3h8v-3h-2.21l3.42-8H18V4z"/></svg>
          </button>
          <button @click="formatText('strike')" title="删除线">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M10 19h4v-3h-4v3zM5 4v3h5v3h4V7h5V4H5zM3 14h18v-2H3v2z"/></svg>
          </button>
          <div class="toolbar-divider"></div>
          <button @click="formatText('code')" title="行内代码">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
          </button>
          <button @click="formatText('link')" title="链接">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>
          </button>
          <div class="toolbar-divider"></div>
          <button @click="openAIWithSelection" class="ai-button" title="AI 助手">
            <span>✨</span>
            <span>AI</span>
          </button>
        </div>
      </Transition>
    </Teleport>
    
    <!-- AI 对话框 -->
    <Teleport to="body">
      <Transition name="dialog">
        <div v-if="showAIDialog" class="ai-dialog-overlay" @click.self="closeAIDialog">
          <div class="ai-dialog">
            <div class="ai-dialog-header">
              <span class="ai-header-icon">✨</span>
              <span class="ai-dialog-title">AI 助手</span>
              <button class="ai-dialog-close" @click="closeAIDialog">×</button>
            </div>
            <div class="ai-dialog-body">
              <div class="ai-messages" ref="messagesRef">
                <div v-for="(msg, idx) in aiMessages" :key="idx" :class="['ai-msg', msg.role]">
                  <div class="ai-msg-content" v-html="renderMarkdown(msg.content)"></div>
                </div>
                <div v-if="aiLoading" class="ai-msg assistant loading">
                  <div class="ai-msg-content">
                    <span class="typing-dots">...</span>
                  </div>
                </div>
              </div>
            </div>
            <div class="ai-dialog-footer">
              <textarea
                v-model="aiInput"
                placeholder="输入问题... (Enter 发送)"
                @keydown.enter.exact.prevent="sendAIMessage"
                @keydown.escape.exact="closeAIDialog"
                ref="aiInputRef"
                rows="1"
              ></textarea>
              <button @click="sendAIMessage" :disabled="!aiInput.trim() || aiLoading" class="ai-send-btn">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="22" y1="2" x2="11" y2="13"></line>
                  <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
                </svg>
              </button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
    
    <!-- 快捷键提示 -->
    <Transition name="fade">
      <div v-if="!showAIDialog && !showFloatingToolbar" class="shortcut-hint">
        <kbd>Ctrl</kbd><kbd>K</kbd> AI 助手
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, shallowRef } from 'vue'
import { Editor, rootCtx, defaultValueCtx, editorViewCtx, parserCtx } from '@milkdown/kit/core'
import { commonmark } from '@milkdown/kit/preset/commonmark'
import { gfm } from '@milkdown/kit/preset/gfm'
import { history } from '@milkdown/kit/plugin/history'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { callPlugin } from '@/services/plugin'
import { marked } from 'marked'

const props = defineProps<{
  modelValue: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

// DOM refs
const containerRef = ref<HTMLElement | null>(null)
const editorRef = ref<HTMLElement | null>(null)
const messagesRef = ref<HTMLElement | null>(null)
const aiInputRef = ref<HTMLTextAreaElement | null>(null)

// Editor instance
const editor = shallowRef<Editor | null>(null)

// Floating toolbar
const showFloatingToolbar = ref(false)
const floatingToolbarStyle = ref({ left: '0px', top: '0px' })
const selectedText = ref('')

// AI dialog
const showAIDialog = ref(false)
const aiInput = ref('')
const aiMessages = ref<{ role: 'user' | 'assistant'; content: string }[]>([])
const aiLoading = ref(false)

// Initialize editor
async function initEditor() {
  if (!editorRef.value) return
  
  const defaultContent = props.modelValue || `# 开始创作

欢迎使用编辑器。直接输入内容，或使用 Markdown 语法。

- **粗体** 和 *斜体*
- \`行内代码\` 和代码块
- [链接](https://example.com)
- 列表和引用

按 **Ctrl+K** 呼出 AI 助手。
`

  editor.value = await Editor.make()
    .config((ctx) => {
      ctx.set(rootCtx, editorRef.value)
      ctx.set(defaultValueCtx, defaultContent)
      ctx.get(listenerCtx).markdownUpdated((ctx, markdown) => {
        emit('update:modelValue', markdown)
      })
    })
    .use(commonmark)
    .use(gfm)
    .use(history)
    .use(listener)
    .create()
  
  // Setup selection listener
  document.addEventListener('selectionchange', handleSelectionChange)
}

// Handle selection change for floating toolbar
function handleSelectionChange() {
  const selection = window.getSelection()
  
  if (!selection || selection.isCollapsed || !selection.toString().trim()) {
    showFloatingToolbar.value = false
    return
  }
  
  // Check if selection is within editor
  const editorEl = editorRef.value
  if (!editorEl) return
  
  let node = selection.anchorNode
  while (node && node !== editorEl) {
    node = node.parentNode as Node
  }
  if (node !== editorEl) {
    showFloatingToolbar.value = false
    return
  }
  
  selectedText.value = selection.toString()
  
  // Position toolbar
  const range = selection.getRangeAt(0)
  const rect = range.getBoundingClientRect()
  
  floatingToolbarStyle.value = {
    left: `${Math.max(8, Math.min(rect.left + rect.width / 2 - 140, window.innerWidth - 300))}px`,
    top: `${Math.max(8, rect.top - 44)}px`,
  }
  
  showFloatingToolbar.value = true
}

// Format text
function formatText(format: string) {
  // Would integrate with Milkdown commands
  showFloatingToolbar.value = false
}

// Open AI with selection
function openAIWithSelection() {
  if (selectedText.value) {
    aiInput.value = selectedText.value
  }
  showFloatingToolbar.value = false
  openAIDialog()
}

// AI Dialog
function openAIDialog() {
  showAIDialog.value = true
  nextTick(() => aiInputRef.value?.focus())
}

function closeAIDialog() {
  showAIDialog.value = false
}

async function sendAIMessage() {
  if (!aiInput.value.trim() || aiLoading.value) return
  
  const userMessage = aiInput.value.trim()
  aiMessages.value.push({ role: 'user', content: userMessage })
  aiInput.value = ''
  aiLoading.value = true
  
  try {
    const response = await callPlugin<{ content: string }>('/agent/chat', {
      action: 'send',
      messages: aiMessages.value.map(m => ({ role: m.role, content: m.content }))
    })
    aiMessages.value.push({ role: 'assistant', content: response.content || '抱歉，无法处理请求。' })
  } catch (error) {
    aiMessages.value.push({ role: 'assistant', content: `错误: ${error}` })
  } finally {
    aiLoading.value = false
    nextTick(() => {
      messagesRef.value?.scrollTo({ top: messagesRef.value.scrollHeight, behavior: 'smooth' })
    })
  }
}

function renderMarkdown(content: string): string {
  return marked(content) as string
}

// Keyboard shortcuts
function handleKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault()
    openAIDialog()
  }
}

// Destroy editor
async function destroyEditor() {
  document.removeEventListener('selectionchange', handleSelectionChange)
  if (editor.value) {
    try {
      await editor.value.destroy()
    } catch (e) {
      console.error('Destroy error:', e)
    }
    editor.value = null
  }
}

// Lifecycle
onMounted(() => {
  initEditor()
  document.addEventListener('keydown', handleKeydown)
})

onUnmounted(() => {
  destroyEditor()
  document.removeEventListener('keydown', handleKeydown)
})

defineExpose({ openAIDialog })
</script>

<style scoped>
.notion-editor {
  position: relative;
  height: 100%;
  width: 100%;
  background: #fff;
  display: flex;
  flex-direction: column;
}

.editor-root {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
  min-height: 0;
}

/* Milkdown Editor Styles - Notion-like */
.editor-root :deep(.milkdown) {
  font-family: -apple-system, BlinkMacMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 1.6;
  color: #37352f;
  outline: none;
  min-height: 100%;
}

.editor-root :deep(.milkdown .ProseMirror) {
  outline: none;
  min-height: 100%;
}

/* Headings */
.editor-root :deep(.milkdown h1) {
  font-size: 2.25rem;
  font-weight: 700;
  margin: 0 0 0.5rem;
  line-height: 1.2;
  letter-spacing: -0.03em;
  color: #37352f;
}

.editor-root :deep(.milkdown h2) {
  font-size: 1.5rem;
  font-weight: 600;
  margin: 1rem 0 0.375rem;
  line-height: 1.3;
}

.editor-root :deep(.milkdown h3) {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0.75rem 0 0.25rem;
}

/* Paragraph */
.editor-root :deep(.milkdown p) {
  margin: 0.25rem 0;
}

/* Code */
.editor-root :deep(.milkdown code) {
  background: rgba(135, 131, 120, 0.15);
  color: #eb5757;
  padding: 0.2em 0.4em;
  border-radius: 3px;
  font-family: 'SF Mono', 'Fira Code', Consolas, monospace;
  font-size: 85%;
}

.editor-root :deep(.milkdown pre) {
  background: #f7f6f3;
  border-radius: 4px;
  padding: 16px;
  margin: 8px 0;
  overflow-x: auto;
}

.editor-root :deep(.milkdown pre code) {
  background: transparent;
  color: inherit;
  padding: 0;
  font-size: 14px;
}

/* Blockquote */
.editor-root :deep(.milkdown blockquote) {
  border-left: 3px solid #37352f;
  padding-left: 16px;
  margin: 8px 0;
  color: #37352f;
}

/* Lists */
.editor-root :deep(.milkdown ul),
.editor-root :deep(.milkdown ol) {
  margin: 4px 0;
  padding-left: 24px;
}

.editor-root :deep(.milkdown li) {
  margin: 2px 0;
}

.editor-root :deep(.milkdown li p) {
  margin: 0;
}

/* Tables */
.editor-root :deep(.milkdown table) {
  border-collapse: collapse;
  width: 100%;
  margin: 8px 0;
}

.editor-root :deep(.milkdown th),
.editor-root :deep(.milkdown td) {
  border: 1px solid #e0e0e0;
  padding: 8px 12px;
  text-align: left;
}

.editor-root :deep(.milkdown th) {
  background: #f7f6f3;
  font-weight: 600;
}

/* Links */
.editor-root :deep(.milkdown a) {
  color: #2383e2;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.editor-root :deep(.milkdown a:hover) {
  color: #0077d4;
}

/* HR */
.editor-root :deep(.milkdown hr) {
  border: none;
  border-top: 1px solid #e0e0e0;
  margin: 16px 0;
}

/* Images */
.editor-root :deep(.milkdown img) {
  max-width: 100%;
  border-radius: 4px;
  margin: 8px 0;
}

/* Selection highlight */
.editor-root :deep(.milkdown ::selection) {
  background: rgba(35, 131, 226, 0.28);
}

/* Floating Toolbar */
.floating-toolbar {
  position: fixed;
  display: flex;
  align-items: center;
  gap: 2px;
  background: #1f1f1f;
  border-radius: 6px;
  padding: 4px 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
  z-index: 1000;
}

.floating-toolbar button {
  background: transparent;
  border: none;
  color: #fff;
  padding: 6px;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.1s;
}

.floating-toolbar button:hover {
  background: rgba(255, 255, 255, 0.12);
}

.floating-toolbar .toolbar-divider {
  width: 1px;
  height: 16px;
  background: rgba(255, 255, 255, 0.2);
  margin: 0 4px;
}

.floating-toolbar .ai-button {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 10px;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  border-radius: 4px;
  font-size: 13px;
  font-weight: 500;
}

/* Shortcut Hint */
.shortcut-hint {
  position: fixed;
  bottom: 16px;
  right: 16px;
  background: #1f1f1f;
  color: #fff;
  padding: 6px 12px;
  border-radius: 6px;
  font-size: 12px;
  pointer-events: none;
  z-index: 100;
}

.shortcut-hint kbd {
  background: rgba(255, 255, 255, 0.15);
  padding: 2px 6px;
  border-radius: 4px;
  margin: 0 2px;
  font-family: inherit;
}

/* AI Dialog */
.ai-dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
}

.ai-dialog {
  width: 480px;
  max-width: 90vw;
  max-height: 75vh;
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.ai-dialog-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 16px;
  border-bottom: 1px solid #e5e5e5;
}

.ai-header-icon {
  font-size: 18px;
}

.ai-dialog-title {
  font-weight: 600;
  font-size: 15px;
  flex: 1;
}

.ai-dialog-close {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 20px;
  color: #666;
  display: flex;
  align-items: center;
  justify-content: center;
}

.ai-dialog-close:hover {
  background: #f0f0f0;
}

.ai-dialog-body {
  flex: 1;
  overflow: hidden;
  min-height: 200px;
}

.ai-messages {
  height: 100%;
  overflow-y: auto;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ai-msg {
  max-width: 88%;
  padding: 10px 14px;
  border-radius: 12px;
  font-size: 14px;
  line-height: 1.5;
}

.ai-msg.user {
  align-self: flex-end;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  color: #fff;
  border-bottom-right-radius: 4px;
}

.ai-msg.assistant {
  align-self: flex-start;
  background: #f4f4f5;
  color: #18181b;
  border-bottom-left-radius: 4px;
}

.ai-msg.assistant.loading .ai-msg-content {
  opacity: 0.6;
}

.ai-msg-content :deep(p) { margin: 0; }
.ai-msg-content :deep(p+p) { margin-top: 8px; }
.ai-msg-content :deep(code) {
  background: rgba(0,0,0,0.1);
  padding: 2px 5px;
  border-radius: 3px;
  font-size: 13px;
}
.ai-msg-content :deep(pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 10px 12px;
  border-radius: 6px;
  margin: 8px 0;
  overflow-x: auto;
}
.ai-msg-content :deep(pre code) {
  background: transparent;
  padding: 0;
}

.typing-dots {
  animation: dotPulse 1s infinite;
}

@keyframes dotPulse {
  0%, 100% { opacity: 0.3; }
  50% { opacity: 1; }
}

.ai-dialog-footer {
  display: flex;
  gap: 8px;
  padding: 12px 16px;
  border-top: 1px solid #e5e5e5;
  background: #fafafa;
}

.ai-dialog-footer textarea {
  flex: 1;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 10px 14px;
  font-size: 14px;
  resize: none;
  outline: none;
  font-family: inherit;
  line-height: 1.4;
  max-height: 120px;
}

.ai-dialog-footer textarea:focus {
  border-color: #7c3aed;
}

.ai-send-btn {
  width: 40px;
  height: 40px;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  border: none;
  border-radius: 8px;
  color: #fff;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.ai-send-btn:hover:not(:disabled) {
  transform: scale(1.02);
}

.ai-send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Transitions */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.dialog-enter-active,
.dialog-leave-active {
  transition: all 0.2s ease;
}

.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}

.dialog-enter-from .ai-dialog,
.dialog-leave-to .ai-dialog {
  transform: translateY(16px) scale(0.98);
}

/* Responsive */
@media (max-width: 768px) {
  .editor-root {
    padding: 16px 12px;
  }
}
</style>
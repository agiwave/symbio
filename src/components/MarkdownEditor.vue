<template>
  <div class="notion-editor" ref="containerRef">
    <!-- 主编辑区 -->
    <div class="editor-scroll-container">
      <div ref="editorRef" class="editor-content"></div>
    </div>
    
    <!-- 段落悬浮工具 (hover 在左侧时显示) -->
    <div v-if="hoverBlockInfo.show" class="block-handle" :style="hoverBlockInfo.style">
      <button class="block-handle-btn" @click="showBlockMenu = true" title="点击添加内容块">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="12" y1="5" x2="12" y2="19"></line>
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
      </button>
    </div>
    
    <!-- 选中文本悬浮工具栏 -->
    <Teleport to="body">
      <div v-if="showFloatingToolbar" class="floating-toolbar" :style="floatingToolbarStyle">
        <button @click="formatBlock('bold')" title="粗体 (Ctrl+B)">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M15.6 10.79c.97-.67 1.65-1.77 1.65-2.79 0-2.26-1.75-4-4-4H7v14h7.04c2.09 0 3.71-1.7 3.71-3.79 0-1.52-.86-2.82-2.15-3.42zM10 6.5h3c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5h-3v-3zm3.5 9H10v-3h3.5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5z"/></svg>
        </button>
        <button @click="formatBlock('italic')" title="斜体 (Ctrl+I)">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M10 4v3h2.21l-3.42 8H6v3h8v-3h-2.21l3.42-8H18V4z"/></svg>
        </button>
        <button @click="formatBlock('underline')" title="下划线 (Ctrl+U)">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M12 17c3.31 0 6-2.69 6-6V3h-2.5v8c0 1.93-1.57 3.5-3.5 3.5S8.5 12.93 8.5 11V3H6v8c0 3.31 2.69 6 6 6zm-7 2v2h14v-2H5z"/></svg>
        </button>
        <button @click="formatBlock('strikethrough')" title="删除线">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M10 19h4v-3h-4v3zM5 4v3h5v3h4V7h5V4H5zM3 14h18v-2H3v2z"/></svg>
        </button>
        <div class="toolbar-divider"></div>
        <button @click="showLinkInput = true" title="链接">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>
        </button>
        <button @click="formatBlock('code')" title="代码">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
        </button>
        <div class="toolbar-divider"></div>
        <button @click="openAIWithSelection" class="ai-button" title="AI 助手">
          <span class="ai-sparkle">✨</span>
          <span>询问 AI</span>
        </button>
      </div>
    </Teleport>
    
    <!-- AI 对话框 -->
    <Teleport to="body">
      <Transition name="dialog">
        <div v-if="showAIDialog" class="ai-dialog-overlay" @click.self="closeAIDialog">
          <div class="ai-dialog">
            <div class="ai-dialog-header">
              <div class="ai-header-icon">✨</div>
              <span class="ai-dialog-title">AI 助手</span>
              <button class="ai-dialog-close" @click="closeAIDialog">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="18" y1="6" x2="6" y2="18"></line>
                  <line x1="6" y1="6" x2="18" y2="18"></line>
                </svg>
              </button>
            </div>
            <div class="ai-dialog-body">
              <div class="ai-messages" ref="messagesRef">
                <div v-for="(msg, idx) in aiMessages" :key="idx" :class="['ai-msg', msg.role]">
                  <div class="ai-msg-content" v-html="renderMarkdown(msg.content)"></div>
                </div>
                <div v-if="aiLoading" class="ai-msg assistant">
                  <div class="ai-msg-content">
                    <span class="typing-dots">
                      <span>.</span><span>.</span><span>.</span>
                    </span>
                  </div>
                </div>
              </div>
            </div>
            <div class="ai-dialog-footer">
              <textarea
                v-model="aiInput"
                placeholder="输入问题..."
                @keydown.enter.exact.prevent="sendAIMessage"
                @keydown.escape.exact="closeAIDialog"
                @keydown.enter.shift.exact.stop
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
    <div class="shortcut-badge" v-if="!showAIDialog">
      <kbd>⌘</kbd><kbd>K</kbd> AI
    </div>
    
    <!-- 代码执行结果 -->
    <Transition name="slide-up">
      <div v-if="executionResult" class="exec-result">
        <div class="exec-header">
          <span :class="['exec-status', executionResult.status]">
            {{ executionResult.status === 'success' ? '✓' : '✗' }}
          </span>
          <span class="exec-time">{{ executionResult.duration_ms }}ms</span>
          <button class="exec-close" @click="executionResult = null">×</button>
        </div>
        <pre class="exec-output" v-if="executionResult.stdout">{{ executionResult.stdout }}</pre>
        <pre class="exec-error" v-if="executionResult.stderr">{{ executionResult.stderr }}</pre>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick, reactive } from 'vue'
import { Crepe, CrepeFeature } from '@milkdown/crepe'
import { marked } from 'marked'
import { callPlugin } from '@/services/plugin'

const props = defineProps<{
  modelValue: string
  theme?: 'light' | 'dark'
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
const crepeInstance = ref<Crepe | null>(null)

// Block hover state (Notion style)
const hoverBlockInfo = reactive({
  show: false,
  style: { top: '0px' }
})

// Floating toolbar state
const showFloatingToolbar = ref(false)
const floatingToolbarStyle = ref({ left: '0px', top: '0px' })
const selectedText = ref('')

// AI dialog state
const showAIDialog = ref(false)
const aiInput = ref('')
const aiMessages = ref<{ role: 'user' | 'assistant'; content: string }[]>([])
const aiLoading = ref(false)

// Execution result
const executionResult = ref<{
  status: 'success' | 'failed'
  stdout: string
  stderr: string
  duration_ms: number
} | null>(null)

// Initialize editor
async function initEditor() {
  if (!editorRef.value) return
  
  try {
    const defaultContent = props.modelValue || `# 开始创作

欢迎使用所见即所得编辑器。直接输入内容，或使用以下快捷键：

- **Ctrl+B** 粗体
- **Ctrl+I** 斜体  
- **Ctrl+K** AI 助手

---

开始你的创作吧！
`
    
    crepeInstance.value = new Crepe({
      root: editorRef.value,
      defaultValue: defaultContent,
      features: {
        [CrepeFeature.CodeMirror]: true,
        [CrepeFeature.ListItem]: true,
        [CrepeFeature.LinkTooltip]: true,
        [CrepeFeature.ImageBlock]: true,
        [CrepeFeature.BlockEdit]: true,
      }
    })
    
    crepeInstance.value.on((api) => {
      api.markdownUpdated((ctx, markdown) => {
        emit('update:modelValue', markdown)
      })
    })
    
    await crepeInstance.value.create()
    
    // Setup event listeners
    setupEventListeners()
  } catch (error) {
    console.error('Failed to initialize editor:', error)
  }
}

// Setup event listeners
function setupEventListeners() {
  document.addEventListener('selectionchange', handleSelectionChange)
  document.addEventListener('mousemove', handleMouseMove)
}

function handleSelectionChange() {
  const selection = window.getSelection()
  
  if (!selection || selection.isCollapsed || !selection.toString().trim()) {
    showFloatingToolbar.value = false
    return
  }
  
  // Check if in editor
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
  
  // Get selection text
  selectedText.value = selection.toString()
  
  // Position toolbar
  const range = selection.getRangeAt(0)
  const rect = range.getBoundingClientRect()
  
  floatingToolbarStyle.value = {
    left: `${Math.max(8, Math.min(rect.left + rect.width / 2 - 140, window.innerWidth - 300))}px`,
    top: `${Math.max(8, rect.top - 48)}px`,
  }
  
  showFloatingToolbar.value = true
}

function handleMouseMove(e: MouseEvent) {
  // Notion-style block handle on left hover
  const editorEl = editorRef.value
  if (!editorEl) return
  
  const rect = editorEl.getBoundingClientRect()
  const x = e.clientX
  
  // Show block handle when hovering near left edge
  if (x < rect.left + 30 && x > rect.left - 30) {
    hoverBlockInfo.show = true
    hoverBlockInfo.style = {
      top: `${e.clientY - rect.top}px`
    }
  } else if (x > rect.left + 100) {
    hoverBlockInfo.show = false
  }
}

// Format block actions
function formatBlock(format: string) {
  // Would integrate with Milkdown commands
  // For now, close toolbar
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

// AI Dialog functions
function openAIDialog() {
  showAIDialog.value = true
  nextTick(() => {
    aiInputRef.value?.focus()
  })
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
      messages: aiMessages.value.map(m => ({
        role: m.role,
        content: m.content
      }))
    })
    
    aiMessages.value.push({ 
      role: 'assistant', 
      content: response.content || '抱歉，我无法处理这个请求。' 
    })
  } catch (error) {
    aiMessages.value.push({ role: 'assistant', content: `错误: ${error}` })
  } finally {
    aiLoading.value = false
    nextTick(() => {
      messagesRef.value?.scrollTo({
        top: messagesRef.value.scrollHeight,
        behavior: 'smooth'
      })
    })
  }
}

function renderMarkdown(content: string): string {
  return marked(content) as string
}

// Keyboard shortcuts
function handleKeydown(e: KeyboardEvent) {
  // Ctrl/Cmd + K for AI
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault()
    openAIDialog()
  }
}

// Destroy editor
async function destroyEditor() {
  document.removeEventListener('selectionchange', handleSelectionChange)
  document.removeEventListener('mousemove', handleMouseMove)
  
  if (crepeInstance.value) {
    try {
      await crepeInstance.value.destroy()
    } catch (error) {
      console.error('Failed to destroy editor:', error)
    }
    crepeInstance.value = null
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

.editor-scroll-container {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
}

.editor-content {
  min-height: 100%;
  padding: 48px 96px;
  max-width: 100%;
}

/* ===== Crepe Editor Theme (Notion-like) ===== */
.editor-content :deep(.crepe) {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, 'Apple Color Emoji', Arial, sans-serif, 'Segoe UI Emoji', 'Segoe UI Symbol';
  font-size: 16px;
  line-height: 1.5;
  color: #37352f;
}

.editor-content :deep(.crepe .editor) {
  outline: none;
  min-height: 100%;
  padding-left: 24px;
}

/* Headings */
.editor-content :deep(.crepe h1) {
  font-size: 2.25rem;
  font-weight: 700;
  margin: 1rem 0 0.25rem;
  line-height: 1.2;
  color: #37352f;
  letter-spacing: -0.02em;
}

.editor-content :deep(.crepe h2) {
  font-size: 1.5rem;
  font-weight: 600;
  margin: 0.75rem 0 0.25rem;
  line-height: 1.3;
}

.editor-content :deep(.crepe h3) {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0.5rem 0 0.25rem;
}

/* Paragraph */
.editor-content :deep(.crepe p) {
  margin: 0.125rem 0;
  line-height: 1.5;
  min-height: 1.5em;
}

/* Inline code */
.editor-content :deep(.crepe code) {
  background: rgba(135, 131, 120, 0.15);
  color: #eb5757;
  padding: 0.2em 0.4em;
  border-radius: 3px;
  font-family: 'SFMono-Regular', Menlo, Consolas, 'PT Mono', monospace;
  font-size: 85%;
}

/* Code block */
.editor-content :deep(.crepe pre) {
  background: #f7f6f3;
  border-radius: 4px;
  padding: 1rem;
  margin: 0.25rem 0;
  overflow-x: auto;
}

.editor-content :deep(.crepe pre code) {
  background: transparent;
  color: inherit;
  padding: 0;
  font-size: 14px;
}

/* Blockquote */
.editor-content :deep(.crepe blockquote) {
  border-left: 3px solid #37352f;
  padding-left: 1rem;
  margin: 0.25rem 0;
  color: #37352f;
}

/* Lists */
.editor-content :deep(.crepe ul),
.editor-content :deep(.crepe ol) {
  margin: 0.125rem 0;
  padding-left: 1.5rem;
}

.editor-content :deep(.crepe li) {
  margin: 0.0625rem 0;
  padding-left: 0.25rem;
}

/* Tables */
.editor-content :deep(.crepe table) {
  border-collapse: collapse;
  width: 100%;
  margin: 0.25rem 0;
  font-size: 14px;
}

.editor-content :deep(.crepe th),
.editor-content :deep(.crepe td) {
  border: 1px solid #e0e0e0;
  padding: 8px 12px;
  text-align: left;
}

.editor-content :deep(.crepe th) {
  background: #f7f6f3;
  font-weight: 600;
}

/* Links */
.editor-content :deep(.crepe a) {
  color: #2383e2;
  text-decoration: underline;
  text-underline-offset: 2px;
}

/* HR */
.editor-content :deep(.crepe hr) {
  border: none;
  border-top: 1px solid #e0e0e0;
  margin: 1rem 0;
}

/* Images */
.editor-content :deep(.crepe img) {
  max-width: 100%;
  border-radius: 4px;
  margin: 0.25rem 0;
}

/* ===== Block Handle (Notion style) ===== */
.block-handle {
  position: absolute;
  left: 4px;
  z-index: 100;
  transform: translateY(-50%);
}

.block-handle-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: #9b9a97;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.block-handle-btn:hover {
  background: rgba(55, 53, 47, 0.08);
  color: #37352f;
}

/* ===== Floating Toolbar ===== */
.floating-toolbar {
  position: fixed;
  display: flex;
  align-items: center;
  gap: 2px;
  background: #1f1f1f;
  border-radius: 6px;
  padding: 4px 6px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
  z-index: 1000;
  animation: toolbarIn 0.15s ease;
}

@keyframes toolbarIn {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
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
  background: rgba(255, 255, 255, 0.1);
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

.floating-toolbar .ai-sparkle {
  font-size: 12px;
}

/* ===== Shortcut Badge ===== */
.shortcut-badge {
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

.shortcut-badge kbd {
  background: rgba(255, 255, 255, 0.15);
  padding: 2px 6px;
  border-radius: 4px;
  margin: 0 2px;
  font-family: inherit;
}

/* ===== AI Dialog ===== */
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
  width: 520px;
  max-width: 90vw;
  max-height: 80vh;
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 24px 48px rgba(0, 0, 0, 0.2);
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
  display: flex;
  align-items: center;
  justify-content: center;
  color: #666;
  transition: all 0.1s;
}

.ai-dialog-close:hover {
  background: #f0f0f0;
  color: #000;
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

.typing-dots span {
  animation: dotPulse 1s infinite;
  animation-fill-mode: both;
}
.typing-dots span:nth-child(2) { animation-delay: 0.2s; }
.typing-dots span:nth-child(3) { animation-delay: 0.4s; }

@keyframes dotPulse {
  0%, 80%, 100% { opacity: 0.3; }
  40% { opacity: 1; }
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

/* ===== Transitions ===== */
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

.slide-up-enter-active,
.slide-up-leave-active {
  transition: all 0.2s ease;
}

.slide-up-enter-from,
.slide-up-leave-to {
  opacity: 0;
  transform: translateY(20px);
}

/* ===== Exec Result ===== */
.exec-result {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  max-height: 200px;
  background: #1e1e1e;
  border-top: 1px solid #333;
  z-index: 50;
}

.exec-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  background: #252525;
  border-bottom: 1px solid #333;
}

.exec-status {
  font-size: 14px;
}

.exec-status.success { color: #4ade80; }
.exec-status.failed { color: #f87171; }

.exec-time {
  font-size: 12px;
  color: #666;
}

.exec-close {
  margin-left: auto;
  background: transparent;
  border: none;
  color: #666;
  cursor: pointer;
  font-size: 16px;
}

.exec-output,
.exec-error {
  margin: 0;
  padding: 10px 12px;
  font-family: 'SFMono-Regular', Menlo, monospace;
  font-size: 12px;
  overflow: auto;
  max-height: 150px;
}

.exec-output { color: #d4d4d4; }
.exec-error { color: #f87171; border-top: 1px solid #333; }

/* ===== Responsive ===== */
@media (max-width: 768px) {
  .editor-content {
    padding: 24px 16px;
  }
}
</style>

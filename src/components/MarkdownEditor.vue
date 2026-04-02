<template>
  <div class="editor-wrapper" ref="containerRef">
    <!-- 主编辑区 -->
    <div class="editor-main">
      <div ref="editorRef" class="editor-content"></div>
    </div>
    
    <!-- 悬浮工具栏 (选中文字时显示) -->
    <Teleport to="body">
      <div v-if="showToolbar" class="floating-toolbar" :style="toolbarStyle">
        <button @click="formatText('bold')" title="粗体"><strong>B</strong></button>
        <button @click="formatText('italic')" title="斜体"><em>I</em></button>
        <button @click="formatText('strike')" title="删除线"><s>S</s></button>
        <div class="toolbar-divider"></div>
        <button @click="formatText('heading')" title="标题">H</button>
        <button @click="formatText('quote')" title="引用">"</button>
        <button @click="formatText('code')" title="代码">&lt;/&gt;</button>
        <button @click="formatText('link')" title="链接">🔗</button>
        <div class="toolbar-divider"></div>
        <button @click="openAIPrompt" title="AI 助手" class="ai-btn">✨ AI</button>
      </div>
    </Teleport>
    
    <!-- 悬浮 AI 对话框 -->
    <Teleport to="body">
      <div v-if="showAIDialog" class="ai-dialog-overlay" @click.self="closeAIDialog">
        <div class="ai-dialog">
          <div class="ai-dialog-header">
            <span class="ai-dialog-title">✨ AI 助手</span>
            <button class="ai-dialog-close" @click="closeAIDialog">×</button>
          </div>
          <div class="ai-dialog-content">
            <div class="ai-messages" ref="messagesRef">
              <div v-for="(msg, idx) in aiMessages" :key="idx" :class="['ai-message', msg.role]">
                <div class="ai-message-content" v-html="renderMarkdown(msg.content)"></div>
              </div>
              <div v-if="aiLoading" class="ai-message assistant loading">
                <div class="ai-message-content">
                  <span class="typing-indicator">●●●</span>
                </div>
              </div>
            </div>
          </div>
          <div class="ai-dialog-input">
            <textarea
              v-model="aiInput"
              placeholder="输入问题... (Enter 发送)"
              @keydown.enter.exact.prevent="sendAIMessage"
              @keydown.escape="closeAIDialog"
              ref="aiInputRef"
            ></textarea>
            <button @click="sendAIMessage" :disabled="!aiInput.trim() || aiLoading" class="send-btn">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M22 2L11 13M22 2l-7 20-4-9-9-4 20-7z"/>
              </svg>
            </button>
          </div>
        </div>
      </div>
    </Teleport>
    
    <!-- AI 快捷提示 -->
    <div class="ai-hint" v-if="!showAIDialog && !showToolbar">
      <kbd>Ctrl</kbd> + <kbd>K</kbd> AI 助手
    </div>
    
    <!-- 执行结果面板 -->
    <div v-if="executionResult" class="result-panel">
      <div class="result-header">
        <span :class="['status-badge', executionResult.status]">
          {{ executionResult.status === 'success' ? '✓ 成功' : '✗ 失败' }}
        </span>
        <span class="result-duration">{{ executionResult.duration_ms }}ms</span>
        <button class="result-close" @click="executionResult = null">×</button>
      </div>
      <div class="result-body">
        <pre v-if="executionResult.stdout" class="result-output">{{ executionResult.stdout }}</pre>
        <pre v-if="executionResult.stderr" class="result-error">{{ executionResult.stderr }}</pre>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick, watch } from 'vue'
import { Crepe } from '@milkdown/crepe'
import { marked } from 'marked'
import { callPlugin } from '@/services/plugin'

const props = defineProps<{
  modelValue: string
  theme?: 'light' | 'dark'
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'selection-change': [selection: { from: number; to: number; text: string } | null]
}>()

// DOM refs
const containerRef = ref<HTMLElement | null>(null)
const editorRef = ref<HTMLElement | null>(null)
const messagesRef = ref<HTMLElement | null>(null)
const aiInputRef = ref<HTMLTextAreaElement | null>(null)

// Editor
const crepeInstance = ref<Crepe | null>(null)

// Toolbar state
const showToolbar = ref(false)
const toolbarPosition = ref({ x: 0, y: 0 })
const toolbarStyle = computed(() => ({
  left: `${toolbarPosition.value.x}px`,
  top: `${toolbarPosition.value.y}px`,
}))

// AI Dialog state
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

// Initialize Crepe editor
async function initEditor() {
  if (!editorRef.value) return
  
  try {
    crepeInstance.value = new Crepe({
      root: editorRef.value,
      defaultValue: props.modelValue || '# 开始写作\n\n输入 `/` 查看命令，或直接开始编辑...\n\n## 功能提示\n\n- 支持 **粗体**、*斜体*、~~删除线~~\n- 支持 `代码` 和代码块\n- 支持表格、列表\n- 选中文字显示格式工具栏\n- 按 `Ctrl+K` 呼出 AI 助手\n',
    })
    
    crepeInstance.value.on((api) => {
      api.markdownUpdated((ctx, markdown) => {
        emit('update:modelValue', markdown)
      })
    })
    
    await crepeInstance.value.create()
    
    // Setup selection listener for floating toolbar
    setupSelectionListener()
  } catch (error) {
    console.error('Failed to initialize Crepe editor:', error)
  }
}

// Selection listener for floating toolbar
function setupSelectionListener() {
  document.addEventListener('selectionchange', handleSelectionChange)
}

function handleSelectionChange() {
  const selection = window.getSelection()
  if (!selection || selection.isCollapsed || !selection.toString().trim()) {
    showToolbar.value = false
    return
  }
  
  // Check if selection is within editor
  const editorEl = editorRef.value
  if (!editorEl) return
  
  let node = selection.anchorNode
  while (node && node !== editorEl) {
    node = node.parentNode
  }
  if (node !== editorEl) {
    showToolbar.value = false
    return
  }
  
  // Calculate toolbar position
  const range = selection.getRangeAt(0)
  const rect = range.getBoundingClientRect()
  const toolbarWidth = 280
  const toolbarHeight = 40
  
  toolbarPosition.value = {
    x: Math.max(10, Math.min(rect.left + rect.width / 2 - toolbarWidth / 2, window.innerWidth - toolbarWidth - 10)),
    y: Math.max(10, rect.top - toolbarHeight - 10),
  }
  
  showToolbar.value = true
}

// Format text commands
function formatText(format: string) {
  // These would integrate with Milkdown commands
  // For now, we'll use the AI to help with formatting
  const selection = window.getSelection()
  if (!selection) return
  
  const text = selection.toString()
  if (!text) return
  
  // Simple format insertion via AI
  aiInput.value = `请将以下文字格式化为${format}格式：\n\n${text}`
  openAIDialog()
}

// Open AI prompt from toolbar
function openAIPrompt() {
  const selection = window.getSelection()
  if (selection && selection.toString().trim()) {
    aiInput.value = `帮我改进这段文字：\n\n${selection.toString()}`
  }
  showToolbar.value = false
  openAIDialog()
}

// Open AI dialog
function openAIDialog() {
  showAIDialog.value = true
  nextTick(() => {
    aiInputRef.value?.focus()
  })
}

// Close AI dialog
function closeAIDialog() {
  showAIDialog.value = false
}

// Send AI message
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
    
    aiMessages.value.push({ role: 'assistant', content: response.content || '抱歉，我无法处理这个请求。' })
  } catch (error) {
    aiMessages.value.push({ role: 'assistant', content: `错误: ${error}` })
  } finally {
    aiLoading.value = false
    nextTick(() => {
      if (messagesRef.value) {
        messagesRef.value.scrollTop = messagesRef.value.scrollHeight
      }
    })
  }
}

// Render markdown
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
  if (crepeInstance.value) {
    try {
      await crepeInstance.value.destroy()
    } catch (error) {
      console.error('Failed to destroy Crepe editor:', error)
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

// Expose methods
defineExpose({
  openAIDialog,
})
</script>

<style scoped>
.editor-wrapper {
  position: relative;
  height: 100%;
  width: 100%;
  background: #fff;
  display: flex;
  flex-direction: column;
}

.editor-main {
  flex: 1;
  overflow-y: auto;
  display: flex;
  justify-content: center;
}

.editor-content {
  width: 100%;
  max-width: 900px;
  min-height: 100%;
  padding: 60px 96px;
}

/* Crepe Editor Override Styles */
.editor-content :deep(.crepe) {
  min-height: 100%;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
  font-size: 16px;
  line-height: 1.7;
  color: #37352f;
}

.editor-content :deep(.crepe .editor) {
  outline: none;
  min-height: 100%;
}

.editor-content :deep(.crepe .editor:focus) {
  outline: none;
}

/* Headings */
.editor-content :deep(.crepe h1) {
  font-size: 2.5rem;
  font-weight: 700;
  margin: 1.5rem 0 0.5rem;
  line-height: 1.2;
  color: #37352f;
}

.editor-content :deep(.crepe h2) {
  font-size: 1.875rem;
  font-weight: 600;
  margin: 1.25rem 0 0.5rem;
  line-height: 1.3;
}

.editor-content :deep(.crepe h3) {
  font-size: 1.5rem;
  font-weight: 600;
  margin: 1rem 0 0.5rem;
  line-height: 1.4;
}

.editor-content :deep(.crepe h4),
.editor-content :deep(.crepe h5),
.editor-content :deep(.crepe h6) {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0.75rem 0 0.5rem;
}

/* Paragraph */
.editor-content :deep(.crepe p) {
  margin: 0.25rem 0;
  line-height: 1.7;
}

/* Code */
.editor-content :deep(.crepe code) {
  background: rgba(135, 131, 120, 0.15);
  color: #eb5757;
  padding: 0.2em 0.4em;
  border-radius: 3px;
  font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, Courier, monospace;
  font-size: 85%;
}

.editor-content :deep(.crepe pre) {
  background: #f7f6f3;
  border-radius: 4px;
  padding: 1rem;
  overflow-x: auto;
  margin: 0.5rem 0;
  font-size: 14px;
}

.editor-content :deep(.crepe pre code) {
  background: transparent;
  color: inherit;
  padding: 0;
  font-size: inherit;
}

/* Blockquote */
.editor-content :deep(.crepe blockquote) {
  border-left: 3px solid #37352f;
  margin: 0.5rem 0;
  padding: 0.25rem 0 0.25rem 1rem;
  color: #37352f;
}

/* Lists */
.editor-content :deep(.crepe ul),
.editor-content :deep(.crepe ol) {
  margin: 0.25rem 0;
  padding-left: 1.5rem;
}

.editor-content :deep(.crepe li) {
  margin: 0.125rem 0;
}

/* Tables */
.editor-content :deep(.crepe table) {
  border-collapse: collapse;
  width: 100%;
  margin: 0.5rem 0;
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

.editor-content :deep(.crepe tr:nth-child(even) td) {
  background: #fafafa;
}

/* Links */
.editor-content :deep(.crepe a) {
  color: #2383e2;
  text-decoration: none;
}

.editor-content :deep(.crepe a:hover) {
  text-decoration: underline;
}

/* Horizontal rule */
.editor-content :deep(.crepe hr) {
  border: none;
  border-top: 1px solid #e0e0e0;
  margin: 1rem 0;
}

/* Images */
.editor-content :deep(.crepe img) {
  max-width: 100%;
  height: auto;
  border-radius: 4px;
  margin: 0.5rem 0;
}

/* Floating Toolbar */
.floating-toolbar {
  position: fixed;
  display: flex;
  align-items: center;
  gap: 2px;
  background: #1a1a1a;
  border-radius: 8px;
  padding: 4px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
  z-index: 1000;
  animation: fadeIn 0.15s ease;
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}

.floating-toolbar button {
  background: transparent;
  border: none;
  color: #fff;
  padding: 6px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  transition: background 0.15s;
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 28px;
}

.floating-toolbar button:hover {
  background: rgba(255, 255, 255, 0.15);
}

.floating-toolbar .toolbar-divider {
  width: 1px;
  height: 20px;
  background: rgba(255, 255, 255, 0.2);
  margin: 0 4px;
}

.floating-toolbar .ai-btn {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: #fff;
  padding: 6px 12px;
}

.floating-toolbar .ai-btn:hover {
  opacity: 0.9;
}

/* AI Hint */
.ai-hint {
  position: fixed;
  bottom: 20px;
  right: 20px;
  background: rgba(0, 0, 0, 0.7);
  color: #fff;
  padding: 8px 14px;
  border-radius: 8px;
  font-size: 12px;
  pointer-events: none;
  z-index: 100;
}

.ai-hint kbd {
  background: rgba(255, 255, 255, 0.2);
  padding: 2px 6px;
  border-radius: 4px;
  margin: 0 2px;
  font-family: inherit;
}

/* AI Dialog */
.ai-dialog-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.4);
  backdrop-filter: blur(4px);
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
  animation: fadeIn 0.2s ease;
}

.ai-dialog {
  width: 480px;
  max-width: 90vw;
  max-height: 70vh;
  background: #fff;
  border-radius: 16px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: slideUp 0.25s ease;
}

@keyframes slideUp {
  from { opacity: 0; transform: translateY(20px); }
  to { opacity: 1; transform: translateY(0); }
}

.ai-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: #fff;
}

.ai-dialog-title {
  font-weight: 600;
  font-size: 15px;
}

.ai-dialog-close {
  background: rgba(255, 255, 255, 0.2);
  border: none;
  color: #fff;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  cursor: pointer;
  font-size: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.15s;
}

.ai-dialog-close:hover {
  background: rgba(255, 255, 255, 0.3);
}

.ai-dialog-content {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  min-height: 200px;
}

.ai-messages {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ai-message {
  max-width: 85%;
  padding: 10px 14px;
  border-radius: 12px;
  font-size: 14px;
  line-height: 1.5;
}

.ai-message.user {
  align-self: flex-end;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: #fff;
  border-bottom-right-radius: 4px;
}

.ai-message.assistant {
  align-self: flex-start;
  background: #f0f0f0;
  color: #333;
  border-bottom-left-radius: 4px;
}

.typing-indicator {
  animation: pulse 1s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 0.3; }
  50% { opacity: 1; }
}

.ai-message-content :deep(p) {
  margin: 0;
}

.ai-message-content :deep(p + p) {
  margin-top: 8px;
}

.ai-message-content :deep(code) {
  background: rgba(0, 0, 0, 0.1);
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 13px;
}

.ai-message-content :deep(pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 12px;
  border-radius: 8px;
  overflow-x: auto;
  margin: 8px 0;
}

.ai-message-content :deep(pre code) {
  background: transparent;
  padding: 0;
}

.ai-dialog-input {
  display: flex;
  gap: 10px;
  padding: 16px;
  border-top: 1px solid #eee;
  background: #fafafa;
}

.ai-dialog-input textarea {
  flex: 1;
  border: 1px solid #e0e0e0;
  border-radius: 12px;
  padding: 12px 16px;
  font-size: 14px;
  resize: none;
  height: 48px;
  outline: none;
  font-family: inherit;
  line-height: 1.4;
  transition: border-color 0.15s;
}

.ai-dialog-input textarea:focus {
  border-color: #667eea;
}

.ai-dialog-input .send-btn {
  width: 48px;
  height: 48px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  border: none;
  border-radius: 12px;
  cursor: pointer;
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: opacity 0.15s, transform 0.15s;
}

.ai-dialog-input .send-btn:hover:not(:disabled) {
  opacity: 0.9;
  transform: scale(1.02);
}

.ai-dialog-input .send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Result Panel */
.result-panel {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  max-height: 250px;
  background: #1e1e1e;
  border-top: 1px solid #333;
  display: flex;
  flex-direction: column;
  z-index: 50;
}

.result-header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  background: #252525;
  border-bottom: 1px solid #333;
}

.status-badge {
  font-size: 12px;
  font-weight: 500;
  padding: 4px 10px;
  border-radius: 4px;
}

.status-badge.success {
  background: #1a4d1a;
  color: #4ade80;
}

.status-badge.failed {
  background: #4d1a1a;
  color: #f87171;
}

.result-duration {
  font-size: 12px;
  color: #888;
}

.result-close {
  margin-left: auto;
  background: transparent;
  border: none;
  color: #888;
  cursor: pointer;
  font-size: 18px;
  padding: 4px;
}

.result-close:hover {
  color: #fff;
}

.result-body {
  flex: 1;
  overflow: auto;
}

.result-output,
.result-error {
  padding: 12px 16px;
  margin: 0;
  font-family: 'SFMono-Regular', Consolas, monospace;
  font-size: 13px;
  white-space: pre-wrap;
  word-break: break-all;
}

.result-output {
  color: #d4d4d4;
}

.result-error {
  color: #f87171;
  border-top: 1px solid #333;
}
</style>
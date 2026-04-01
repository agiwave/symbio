<template>
  <div class="markdown-editor-container" ref="containerRef">
    <div ref="editorRef" class="milkdown-editor"></div>
    
    <!-- 快捷键提示 -->
    <div class="shortcut-hint" v-if="!showAIDialog">
      <kbd>Ctrl</kbd> + <kbd>K</kbd> 呼出 AI 助手
    </div>
    
    <!-- 悬浮 AI 对话框 -->
    <Teleport to="body">
      <div v-if="showAIDialog" class="ai-dialog-overlay" @click.self="closeAIDialog">
        <div class="ai-dialog" :style="dialogStyle">
          <div class="ai-dialog-header">
            <span class="ai-dialog-title">AI 助手</span>
            <button class="ai-dialog-close" @click="closeAIDialog">×</button>
          </div>
          <div class="ai-dialog-content">
            <div class="ai-messages" ref="messagesRef">
              <div v-for="(msg, idx) in aiMessages" :key="idx" :class="['ai-message', msg.role]">
                <div class="ai-message-content" v-html="renderMarkdown(msg.content)"></div>
              </div>
              <div v-if="aiLoading" class="ai-message assistant loading">
                <div class="ai-message-content">思考中...</div>
              </div>
            </div>
          </div>
          <div class="ai-dialog-input">
            <textarea
              v-model="aiInput"
              placeholder="输入问题... (Enter 发送, Esc 关闭)"
              @keydown.enter.exact.prevent="sendAIMessage"
              @keydown.escape="closeAIDialog"
              ref="aiInputRef"
            ></textarea>
            <button @click="sendAIMessage" :disabled="!aiInput.trim() || aiLoading">
              发送
            </button>
          </div>
        </div>
      </div>
    </Teleport>
    
    <!-- 执行结果 -->
    <div v-if="executionResult" class="execution-result">
      <div class="result-header">
        <span :class="['status', executionResult.status]">
          {{ executionResult.status === 'success' ? '✓ 成功' : '✗ 失败' }}
        </span>
        <span class="duration">{{ executionResult.duration_ms }}ms</span>
        <button class="close-btn" @click="executionResult = null">×</button>
      </div>
      <pre v-if="executionResult.stdout" class="result-output">{{ executionResult.stdout }}</pre>
      <pre v-if="executionResult.stderr" class="result-error">{{ executionResult.stderr }}</pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { Crepe } from '@milkdown/crepe'
// 使用自定义样式代替 Crepe 默认主题
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

const containerRef = ref<HTMLElement | null>(null)
const editorRef = ref<HTMLElement | null>(null)
const crepeInstance = ref<Crepe | null>(null)
const executionResult = ref<{
  status: 'success' | 'failed'
  stdout: string
  stderr: string
  duration_ms: number
} | null>(null)

// AI 对话框状态
const showAIDialog = ref(false)
const aiInput = ref('')
const aiMessages = ref<{ role: 'user' | 'assistant'; content: string }[]>([])
const aiLoading = ref(false)
const messagesRef = ref<HTMLElement | null>(null)
const aiInputRef = ref<HTMLTextAreaElement | null>(null)
const dialogPosition = ref({ x: 0, y: 0 })

const dialogStyle = computed(() => ({
  left: `${Math.min(dialogPosition.value.x, window.innerWidth - 400)}px`,
  top: `${Math.min(dialogPosition.value.y, window.innerHeight - 400)}px`,
}))

// 初始化 Crepe 编辑器
async function initEditor() {
  if (!editorRef.value) return
  
  try {
    crepeInstance.value = new Crepe({
      root: editorRef.value,
      defaultValue: props.modelValue,
    })
    
    // 监听内容变化
    crepeInstance.value.on((api) => {
      api.markdownUpdated((ctx, markdown) => {
        emit('update:modelValue', markdown)
      })
    })
    
    await crepeInstance.value.create()
  } catch (error) {
    console.error('Failed to initialize Crepe editor:', error)
  }
}

// 销毁编辑器
async function destroyEditor() {
  if (crepeInstance.value) {
    try {
      await crepeInstance.value.destroy()
    } catch (error) {
      console.error('Failed to destroy Crepe editor:', error)
    }
    crepeInstance.value = null
  }
}

// 获取编辑器中的选中文本
function getEditorSelection() {
  // Crepe 内部使用 ProseMirror，暂时返回空
  return null
}

// 快捷键处理
function handleKeydown(e: KeyboardEvent) {
  // Ctrl/Cmd + K 呼出 AI 对话框
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault()
    openAIDialog()
  }
}

// 打开 AI 对话框
function openAIDialog() {
  // 计算对话框位置
  const rect = containerRef.value?.getBoundingClientRect()
  if (rect) {
    dialogPosition.value = {
      x: rect.left + rect.width / 2 - 175,
      y: rect.top + 100,
    }
  }
  
  showAIDialog.value = true
  nextTick(() => {
    aiInputRef.value?.focus()
  })
}

// 关闭 AI 对话框
function closeAIDialog() {
  showAIDialog.value = false
}

// 发送 AI 消息
async function sendAIMessage() {
  if (!aiInput.value.trim() || aiLoading.value) return
  
  const userMessage = aiInput.value.trim()
  aiMessages.value.push({ role: 'user', content: userMessage })
  aiInput.value = ''
  aiLoading.value = true
  
  try {
    // 调用后端 AI 接口
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

// 渲染 Markdown
function renderMarkdown(content: string): string {
  return marked(content) as string
}

// 监听 modelValue 变化（外部更新暂不支持，用户直接在编辑器中编辑）
// watch(() => props.modelValue, (newValue) => {
//   // Crepe 暂不支持外部设置内容
// })

// 键盘事件监听
onMounted(() => {
  initEditor()
  document.addEventListener('keydown', handleKeydown)
})

onUnmounted(() => {
  destroyEditor()
  document.removeEventListener('keydown', handleKeydown)
})

// 暴露方法供父组件使用
defineExpose({
  getSelection: getEditorSelection,
  openAIDialog,
})
</script>

<style scoped>
.markdown-editor-container {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: #fff;
}

.milkdown-editor {
  flex: 1;
  overflow: auto;
}

/* 覆盖 Crepe 默认样式 */
.milkdown-editor :deep(.crepe) {
  height: 100%;
}

.milkdown-editor :deep(.crepe .editor) {
  padding: 1.5rem;
  outline: none;
}

.milkdown-editor :deep(.crepe h1) {
  font-size: 2rem;
  font-weight: 700;
  margin: 1rem 0 0.5rem;
  padding-bottom: 0.3rem;
  border-bottom: 1px solid #eee;
}

.milkdown-editor :deep(.crepe h2) {
  font-size: 1.5rem;
  font-weight: 600;
  margin: 0.8rem 0 0.4rem;
  padding-bottom: 0.2rem;
  border-bottom: 1px solid #eee;
}

.milkdown-editor :deep(.crepe h3) {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0.6rem 0 0.3rem;
}

.milkdown-editor :deep(.crepe p) {
  margin: 0.5rem 0;
  line-height: 1.7;
}

.milkdown-editor :deep(.crepe code) {
  background: #f5f5f5;
  padding: 0.2rem 0.4rem;
  border-radius: 3px;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.9em;
}

.milkdown-editor :deep(.crepe pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 1rem;
  border-radius: 8px;
  overflow-x: auto;
  margin: 0.5rem 0;
}

.milkdown-editor :deep(.crepe pre code) {
  background: transparent;
  padding: 0;
  color: inherit;
}

.milkdown-editor :deep(.crepe blockquote) {
  border-left: 4px solid #ddd;
  margin: 0.5rem 0;
  padding: 0.5rem 1rem;
  background: #f9f9f9;
  color: #666;
}

.milkdown-editor :deep(.crepe ul),
.milkdown-editor :deep(.crepe ol) {
  margin: 0.5rem 0;
  padding-left: 1.5rem;
}

.milkdown-editor :deep(.crepe li) {
  margin: 0.25rem 0;
}

.milkdown-editor :deep(.crepe table) {
  border-collapse: collapse;
  width: 100%;
  margin: 0.5rem 0;
}

.milkdown-editor :deep(.crepe th),
.milkdown-editor :deep(.crepe td) {
  border: 1px solid #ddd;
  padding: 0.5rem;
  text-align: left;
}

.milkdown-editor :deep(.crepe th) {
  background: #f5f5f5;
  font-weight: 600;
}

.milkdown-editor :deep(.crepe a) {
  color: #0366d6;
  text-decoration: none;
}

.milkdown-editor :deep(.crepe a:hover) {
  text-decoration: underline;
}

.milkdown-editor :deep(.crepe hr) {
  border: none;
  border-top: 1px solid #eee;
  margin: 1rem 0;
}

.milkdown-editor :deep(.crepe img) {
  max-width: 100%;
  height: auto;
  border-radius: 4px;
}

/* 快捷键提示 */
.shortcut-hint {
  position: absolute;
  bottom: 1rem;
  right: 1rem;
  background: rgba(0, 0, 0, 0.6);
  color: #fff;
  padding: 0.35rem 0.75rem;
  border-radius: 6px;
  font-size: 0.75rem;
  opacity: 0.7;
  pointer-events: none;
}

.shortcut-hint kbd {
  background: rgba(255, 255, 255, 0.2);
  padding: 0.1rem 0.4rem;
  border-radius: 3px;
  margin: 0 0.1rem;
}

/* 悬浮 AI 对话框 */
.ai-dialog-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.3);
  z-index: 1000;
  display: flex;
  align-items: flex-start;
  justify-content: flex-start;
}

.ai-dialog {
  position: absolute;
  width: 350px;
  max-height: 450px;
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.ai-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.75rem 1rem;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: #fff;
}

.ai-dialog-title {
  font-weight: 600;
  font-size: 0.95rem;
}

.ai-dialog-close {
  background: none;
  border: none;
  color: #fff;
  font-size: 1.25rem;
  cursor: pointer;
  padding: 0;
  line-height: 1;
  opacity: 0.8;
}

.ai-dialog-close:hover {
  opacity: 1;
}

.ai-dialog-content {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.ai-messages {
  flex: 1;
  overflow-y: auto;
  padding: 0.75rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.ai-message {
  max-width: 90%;
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  font-size: 0.875rem;
  line-height: 1.5;
}

.ai-message.user {
  align-self: flex-end;
  background: #667eea;
  color: #fff;
}

.ai-message.assistant {
  align-self: flex-start;
  background: #f0f0f0;
  color: #333;
}

.ai-message.loading .ai-message-content {
  opacity: 0.6;
}

.ai-message-content :deep(p) {
  margin: 0;
}

.ai-message-content :deep(code) {
  background: rgba(0,0,0,0.1);
  padding: 0.1rem 0.3rem;
  border-radius: 3px;
  font-size: 0.85em;
}

.ai-dialog-input {
  display: flex;
  gap: 0.5rem;
  padding: 0.75rem;
  border-top: 1px solid #eee;
  background: #fafafa;
}

.ai-dialog-input textarea {
  flex: 1;
  border: 1px solid #ddd;
  border-radius: 8px;
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  resize: none;
  height: 60px;
  outline: none;
  font-family: inherit;
}

.ai-dialog-input textarea:focus {
  border-color: #667eea;
}

.ai-dialog-input button {
  padding: 0.5rem 1rem;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: #fff;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-weight: 500;
  transition: opacity 0.2s;
}

.ai-dialog-input button:hover:not(:disabled) {
  opacity: 0.9;
}

.ai-dialog-input button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 执行结果 */
.execution-result {
  margin-top: auto;
  border-top: 1px solid var(--color-border, #ddd);
  background: var(--color-surface, #f8f9fa);
  max-height: 300px;
  overflow: auto;
}

.result-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.5rem 1rem;
  background: #f8f9fa;
  border-bottom: 1px solid var(--color-border, #ddd);
}

.status {
  font-size: 0.875rem;
  font-weight: 500;
}

.status.success {
  color: #28a745;
}

.status.failed {
  color: #dc3545;
}

.duration {
  font-size: 0.75rem;
  color: var(--color-text-muted, #666);
}

.close-btn {
  margin-left: auto;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 18px;
  color: #666;
}

.result-output, .result-error {
  padding: 0.75rem 1rem;
  margin: 0;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.75rem;
  white-space: pre-wrap;
  word-break: break-all;
}

.result-output {
  background: #1e1e1e;
  color: #d4d4d4;
}

.result-error {
  background: #1e1e1e;
  color: #f87171;
  border-top: 1px solid var(--color-border, #333);
}
</style>

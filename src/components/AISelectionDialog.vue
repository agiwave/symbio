<template>
  <Teleport to="body">
    <Transition name="slide-up">
      <div 
        v-if="state.visible.value" 
        ref="dialogRef"
        class="ai-selection-dialog"
        :style="state.dialogStyle.value"
        :class="{ dragging: state.isDragging.value }"
      >
        <div 
          class="dialog-header" 
          @mousedown="handleDragStart"
        >
          <span class="header-icon">✨</span>
          <span class="dialog-title">AI 助手</span>
          <button class="dialog-close" @click.stop="state.close">×</button>
        </div>
        
        <!-- 选中的内容提示 -->
        <div v-if="state.selectedText.value" class="selected-context">
          <div class="context-header">
            <span class="context-label">选中的内容</span>
            <span v-if="selectionInfo.filePath" class="file-path">
              📄 {{ getRelativePath(selectionInfo.filePath) }}
            </span>
            <span v-if="selectionInfo.startLine && selectionInfo.endLine" class="line-range">
              📍 行 {{ selectionInfo.startLine }}-{{ selectionInfo.endLine }}
            </span>
          </div>
          <div class="context-text">
            {{ state.selectedText.value.slice(0, 100) }}{{ state.selectedText.value.length > 100 ? '...' : '' }}
          </div>
        </div>
        
        <div class="dialog-body">
          <div class="messages" ref="messagesRef">
            <div 
              v-for="(msg, idx) in state.messages.value" 
              :key="idx" 
              :class="['msg', msg.role]"
            >
              <div class="msg-content" v-html="renderMarkdown(msg.content)"></div>
            </div>
            <!-- 流式加载指示器 -->
            <div v-if="state.loading.value" class="msg assistant loading">
              <div class="msg-content">
                <!-- 显示流式内容 -->
                <div v-if="streamingContent" v-html="renderMarkdown(streamingContent)"></div>
                <!-- 否则显示打字指示器 -->
                <span v-else class="typing-dots">...</span>
              </div>
            </div>
          </div>
        </div>
        
        <div class="dialog-footer">
          <textarea
            v-model="state.input.value"
            :placeholder="state.selectedText.value ? '针对选中内容提问...' : '输入问题...'"
            @keydown.enter.exact.prevent="handleSend"
            @keydown.escape.exact="state.close"
            ref="inputRef"
            rows="1"
          ></textarea>
          <button 
            @click="handleSend" 
            :disabled="!state.input.value.trim() || state.loading.value" 
            class="send-btn"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="22" y1="2" x2="11" y2="13"></line>
              <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
            </svg>
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, nextTick, watch, computed } from 'vue'
import { marked } from 'marked'
import type { AISelectionReturn } from '@/composables/useAISelection'
import { sendMessageStream, type ChatMessage } from '@/services/ai'

const props = defineProps<{
  state: AISelectionReturn
  // 当前文档信息（由父组件传入）
  currentFilePath?: string
  currentFileContent?: string
}>()

const messagesRef = ref<HTMLElement | null>(null)
const inputRef = ref<HTMLTextAreaElement | null>(null)
const dialogRef = ref<HTMLElement | null>(null)
const streamingContent = ref('')

// 获取选区信息（从 savedSelection 获取）
const selectionInfo = computed(() => {
  const saved = props.state.savedSelection.value
  if (!saved) return {}
  return {
    filePath: saved.filePath,
    startLine: saved.startLine,
    endLine: saved.endLine,
    fullContent: saved.fullContent
  }
})

// 获取相对路径（去掉工作区前缀）
function getRelativePath(fullPath: string): string {
  // 简单处理：只保留最后两部分
  const parts = fullPath.split('/')
  if (parts.length > 2) {
    return '.../' + parts.slice(-2).join('/')
  }
  return fullPath
}

// 渲染 Markdown
function renderMarkdown(content: string): string {
  try {
    return marked.parse(content) as string
  } catch {
    return content
  }
}

// 滚动到底部
function scrollToBottom() {
  if (messagesRef.value) {
    messagesRef.value.scrollTop = messagesRef.value.scrollHeight
  }
}

// 拖拽处理
function handleDragStart(e: MouseEvent) {
  // 如果点击的是关闭按钮，不处理拖拽
  if ((e.target as HTMLElement).closest('.dialog-close')) return
  props.state.startDrag(e)
}

// 发送消息
async function handleSend() {
  const text = props.state.input.value.trim()
  if (!text || props.state.loading.value) return

  // 添加用户消息（保持原始文本用于显示）
  props.state.messages.value.push({ role: 'user', content: text })
  props.state.input.value = ''
  props.state.loading.value = true
  streamingContent.value = ''

  await nextTick()
  scrollToBottom()

  try {
    // 构建消息历史
    const chatMessages: ChatMessage[] = props.state.messages.value.map(m => ({
      role: m.role as 'user' | 'assistant',
      content: m.content
    }))

    // 流式发送消息
    const response = await sendMessageStream(
      chatMessages,
      props.state.sessionId,
      (chunk) => {
        if (chunk.data && typeof chunk.data === 'object') {
          const data = chunk.data as Record<string, unknown>
          if (data.content && typeof data.content === 'string') {
            streamingContent.value = data.content as string
          }
        }
        scrollToBottom()
      }
    )

    // 流完成 - 添加助手消息
    if (response.error) {
      props.state.messages.value.push({
        role: 'assistant',
        content: `错误: ${response.error}`
      })
    } else if (streamingContent.value) {
      props.state.messages.value.push({
        role: 'assistant',
        content: streamingContent.value
      })
    } else {
      props.state.messages.value.push({
        role: 'assistant',
        content: '抱歉，无法处理请求。'
      })
    }
  } catch (error) {
    props.state.messages.value.push({
      role: 'assistant',
      content: `错误: ${error}`
    })
  } finally {
    props.state.loading.value = false
    streamingContent.value = ''
    nextTick(() => scrollToBottom())
  }
}

// 监听可见性，更新 dialogRef 并自动 focus
watch(() => props.state.visible.value, (visible) => {
  if (visible) {
    nextTick(() => {
      props.state.dialogRef.value = dialogRef.value
      // 自动 focus 输入框
      inputRef.value?.focus()
    })
  }
})

// 监听选区变化，每次选择都自动 focus
watch(() => props.state.selectedText.value, () => {
  if (props.state.visible.value) {
    nextTick(() => {
      inputRef.value?.focus()
    })
  }
})
</script>

<style scoped>
.ai-selection-dialog {
  position: fixed;
  width: 360px;
  max-height: 60vh;
  background: rgba(255, 255, 255, 0.98);
  border-radius: 16px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.12), 0 0 0 1px rgba(0, 0, 0, 0.05);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  z-index: 2000;
  backdrop-filter: blur(12px);
  user-select: none;
}

.ai-selection-dialog.dragging {
  cursor: grabbing;
}

.dialog-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid rgba(0, 0, 0, 0.06);
  cursor: grab;
}

.ai-selection-dialog.dragging .dialog-header {
  cursor: grabbing;
}

.header-icon {
  font-size: 14px;
}

.dialog-title {
  font-weight: 600;
  font-size: 13px;
  flex: 1;
  color: #1a1a1a;
}

.dialog-close {
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 16px;
  color: #999;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.dialog-close:hover {
  background: rgba(0, 0, 0, 0.08);
  color: #333;
}

/* Selected context */
.selected-context {
  padding: 8px 14px;
  background: linear-gradient(135deg, rgba(124, 58, 237, 0.06), rgba(37, 99, 235, 0.06));
}

.context-header {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 4px;
}

.context-label {
  font-size: 10px;
  color: #888;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.file-path {
  font-size: 11px;
  color: #666;
  font-family: 'Fira Code', 'Consolas', monospace;
}

.line-range {
  font-size: 11px;
  color: #666;
  font-family: 'Fira Code', 'Consolas', monospace;
}

.context-text {
  font-size: 12px;
  color: #444;
  padding: 6px 10px;
  background: rgba(255, 255, 255, 0.8);
  border-radius: 6px;
  border-left: 2px solid #7c3aed;
}

.dialog-body {
  flex: 1;
  overflow: hidden;
  min-height: 100px;
  max-height: 280px;
}

.messages {
  height: 100%;
  overflow-y: auto;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* 滚动条样式 */
.messages::-webkit-scrollbar {
  width: 6px;
}

.messages::-webkit-scrollbar-track {
  background: transparent;
}

.messages::-webkit-scrollbar-thumb {
  background: rgba(0, 0, 0, 0.15);
  border-radius: 3px;
}

.messages::-webkit-scrollbar-thumb:hover {
  background: rgba(0, 0, 0, 0.25);
}

.msg {
  max-width: 88%;
  padding: 8px 12px;
  border-radius: 12px;
  font-size: 13px;
  line-height: 1.5;
}

.msg.user {
  align-self: flex-end;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  color: #fff;
  border-bottom-right-radius: 4px;
}

.msg.assistant {
  align-self: flex-start;
  background: #f4f4f5;
  color: #18181b;
  border-bottom-left-radius: 4px;
}

.msg.assistant.loading .msg-content {
  opacity: 0.6;
}

.msg-content :deep(p) { margin: 0; }
.msg-content :deep(p+p) { margin-top: 8px; }
.msg-content :deep(code) {
  background: rgba(0, 0, 0, 0.1);
  padding: 2px 5px;
  border-radius: 3px;
  font-size: 13px;
}
.msg-content :deep(pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 10px 12px;
  border-radius: 6px;
  margin: 8px 0;
  overflow-x: auto;
}
.msg-content :deep(pre code) {
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

.dialog-footer {
  display: flex;
  gap: 8px;
  padding: 10px 14px;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
}

.dialog-footer textarea {
  flex: 1;
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 10px;
  padding: 8px 12px;
  font-size: 13px;
  resize: none;
  outline: none;
  font-family: inherit;
  line-height: 1.4;
  max-height: 100px;
  background: rgba(0, 0, 0, 0.02);
  transition: all 0.15s;
}

.dialog-footer textarea:focus {
  border-color: #7c3aed;
  background: #fff;
}

.send-btn {
  width: 36px;
  height: 36px;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  border: none;
  border-radius: 10px;
  color: #fff;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.send-btn:hover:not(:disabled) {
  transform: scale(1.05);
  box-shadow: 0 2px 8px rgba(124, 58, 237, 0.3);
}

.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Transition */
.slide-up-enter-active {
  transition: all 0.2s cubic-bezier(0.16, 1, 0.3, 1);
}

.slide-up-leave-active {
  transition: all 0.15s cubic-bezier(0.4, 0, 1, 1);
}

.slide-up-enter-from {
  opacity: 0;
  transform: translateY(12px) scale(0.96);
}

.slide-up-leave-to {
  opacity: 0;
  transform: translateY(8px) scale(0.98);
}
</style>

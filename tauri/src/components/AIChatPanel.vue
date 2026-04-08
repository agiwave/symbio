<template>
  <div class="ai-chat-panel">
    <!-- 消息历史 -->
    <div class="chat-messages" ref="messagesRef">
      <div v-if="messages.length === 0" class="empty-chat">
        <p>开始与 AI 对话</p>
      </div>

      <div
        v-for="(msg, index) in messages"
        :key="`${sessionId}-${index}-${msg.timestamp}`"
        class="message"
        :class="msg.role"
      >
        <div class="message-avatar">
          {{ msg.role === 'user' ? '👤' : '🤖' }}
        </div>
        <div class="message-content">
          <div class="message-text" v-html="renderMarkdown(msg.content)"></div>
          <div class="message-time">{{ formatTime(msg.timestamp) }}</div>
        </div>
      </div>

      <!-- 流式加载指示器 -->
      <div v-if="isLoading" class="message assistant loading">
        <div class="message-avatar">🤖</div>
        <div class="message-content">
          <!-- 显示流式内容（只有字符串才渲染） -->
          <div v-if="typeof streamingContent === 'string' && streamingContent" class="message-text" v-html="renderMarkdown(streamingContent)"></div>
          <!-- 否则显示打字指示器 -->
          <div v-else class="typing-indicator">
            <span></span><span></span><span></span>
          </div>
        </div>
      </div>
    </div>

    <!-- 输入区域 -->
    <div class="chat-input">
      <!-- 当前上下文信息 -->
      <div v-if="hasContext" class="context-bar">
        <span class="context-label">当前上下文</span>
        <span v-if="context?.filePath" class="file-name">📄 {{ context.filePath.split(/[\\\/]/).pop() }}</span>
        <span v-if="context?.startLine" class="line-range">📍 行 {{ context.startLine }}{{ context.endLine && context.endLine !== context.startLine ? '-' + context.endLine : '' }}</span>
        <span v-if="context?.selectedText" class="selected-text">{{ context.selectedText.slice(0, 50) }}{{ context.selectedText.length > 50 ? '...' : '' }}</span>
      </div>
      <div class="input-wrapper">
        <textarea
          v-model="inputText"
          placeholder="输入消息..."
          @keydown.enter.exact="handleKeydown"
          rows="1"
        ></textarea>
        <button
          class="send-btn"
          :class="{ 'stop-btn': isLoading && !inputText.trim() }"
          @click="handleSendOrAbort()"
          :disabled="!isLoading && !inputText.trim()"
          :title="isLoading ? (inputText.trim() ? '发送新消息' : '停止') : '发送'"
        >
          <!-- 发送图标：没有加载 或 加载中有输入内容 -->
          <svg v-if="!isLoading || inputText.trim()" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="22" y1="2" x2="11" y2="13"></line>
            <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
          </svg>
          <!-- 停止图标：加载中且没有输入内容 -->
          <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="currentColor">
            <rect x="6" y="6" width="12" height="12" rx="2"></rect>
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, watch, computed } from 'vue'
import { marked } from 'marked'
import { useChatConnection } from '@/composables/useChatConnection'
import { useAIContext, buildContextualMessage } from '@/composables/useAIContext'
import type { ChatMessage, SessionMessage } from '@/services/ai'

// Props
const props = defineProps<{
  sessionId: string
  messages: SessionMessage[]
  onUpdateMessages: (messages: SessionMessage[]) => void
  onSendComplete?: () => void
  /** 是否显示上下文信息，默认 false */
  showContext?: boolean
}>()

// 使用全局 AI 上下文（仅在 showContext 为 true 时使用）
const { context } = props.showContext ? useAIContext() : { context: ref(null) }

// 计算是否有有效上下文
const hasContext = computed(() => {
  if (!props.showContext || !context.value) return false
  return !!(context.value.filePath || context.value.selectedText)
})

// 配置 marked
marked.setOptions({ breaks: true, gfm: true })

// 使用统一的聊天连接 composable
const chat = useChatConnection({
  sessionId: props.sessionId,
  messages: computed(() => props.messages),
  onUpdateMessages: props.onUpdateMessages,
  onSendComplete: props.onSendComplete,
})

// 解构 chat 对象，确保 Ref 在模板中正确解包
const { isLoading, streamingContent, toolCalls, error } = chat

// 输入文本
const inputText = ref('')
// 消息容器 DOM 引用
const messagesRef = ref<HTMLElement | null>(null)

// 处理发送或停止
function handleSendOrAbort() {
  if (isLoading.value) {
    // 正在加载
    if (inputText.value.trim()) {
      // 有输入内容，发送新消息（会自动中止旧请求）
      handleSend()
    } else {
      // 没有输入内容，仅中止
      chat.abort()
    }
  } else if (inputText.value.trim()) {
    // 未加载且有内容，直接发送
    handleSend()
  }
}

// 发送消息
function handleSend() {
  const text = inputText.value.trim()
  if (!text) return

  // 如果启用上下文且有上下文信息，则构建上下文消息
  const ctx = props.showContext ? context.value : null
  const contextualContent = ctx ? buildContextualMessage(text, ctx) : text

  const now = Math.floor(Date.now() / 1000)

  // 添加用户消息
  const userMessage: SessionMessage = {
    role: 'user',
    content: contextualContent,
    timestamp: now
  }
  props.onUpdateMessages([...props.messages, userMessage])

  inputText.value = ''
  nextTick(() => scrollToBottom())

  // 构建消息历史
  const chatMessages: ChatMessage[] = [...props.messages, userMessage].map(m => ({
    role: m.role as 'user' | 'assistant',
    content: m.content
  }))

  // 发送消息
  chat.send(chatMessages, props.sessionId)
}

// 键盘事件
function handleKeydown(e: KeyboardEvent) {
  if (!e.shiftKey) {
    e.preventDefault()
    handleSendOrAbort()
  }
}

// 渲染 Markdown
function renderMarkdown(content: string): string {
  try {
    return marked.parse(content) as string
  } catch {
    return content
  }
}

// 格式化时间
function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000)
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

// 滚动到底部
function scrollToBottom() {
  if (messagesRef.value) {
    messagesRef.value.scrollTop = messagesRef.value.scrollHeight
  }
}

// 监听消息变化，滚动到底部
watch(() => props.messages.length, () => {
  nextTick(() => scrollToBottom())
}, { flush: 'post', immediate: true })
</script>

<style scoped>
.ai-chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.chat-messages {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 1rem;
}

.empty-chat {
  text-align: center;
  color: var(--color-text-muted);
  padding: 3rem;
}

.message {
  display: flex;
  gap: 0.75rem;
  margin-bottom: 1rem;
}

.message.user {
  flex-direction: row-reverse;
}

.message-avatar {
  font-size: 1.25rem;
  flex-shrink: 0;
}

.message-content {
  max-width: 70%;
}

.message.user .message-content {
  text-align: right;
}

.message-text {
  padding: 0.75rem 1rem;
  border-radius: 12px;
  font-size: 0.875rem;
  line-height: 1.5;
  word-break: break-word;
  text-align: left;
}

.message.user .message-text {
  background: var(--color-primary);
  color: white;
  border-bottom-right-radius: 4px;
}

.message.assistant .message-text {
  background: #f0f0f0;
  color: var(--color-text);
  border-bottom-left-radius: 4px;
}

.message-time {
  font-size: 0.625rem;
  color: var(--color-text-muted);
  margin-top: 0.25rem;
}

/* Markdown 样式 */
.message-text :deep(pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 0.75rem;
  border-radius: 6px;
  overflow-x: auto;
  margin: 0.5rem 0;
}

.message-text :deep(code) {
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.8rem;
}

.message-text :deep(p) {
  margin: 0.5rem 0;
}

.message-text :deep(p:first-child) {
  margin-top: 0;
}

.message-text :deep(p:last-child) {
  margin-bottom: 0;
}

/* 打字指示器 */
.typing-indicator {
  display: flex;
  gap: 4px;
  padding: 0.5rem;
}

.typing-indicator span {
  width: 6px;
  height: 6px;
  background: var(--color-text-muted);
  border-radius: 50%;
  animation: bounce 1.4s infinite ease-in-out both;
}

.typing-indicator span:nth-child(1) { animation-delay: -0.32s; }
.typing-indicator span:nth-child(2) { animation-delay: -0.16s; }

@keyframes bounce {
  0%, 80%, 100% { transform: scale(0); }
  40% { transform: scale(1); }
}

/* 输入区域 */
.chat-input {
  padding: 0.75rem 1rem;
  border-top: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.context-bar {
  padding: 0.5rem 0.75rem;
  margin-bottom: 0.5rem;
  background: linear-gradient(135deg, rgba(124, 58, 237, 0.06), rgba(37, 99, 235, 0.06));
  border-radius: 6px;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.context-bar .context-label {
  font-size: 10px;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  font-weight: 600;
}

.context-bar .file-name,
.context-bar .line-range {
  font-size: 11px;
  color: var(--color-text-secondary);
  font-family: 'Fira Code', 'Consolas', monospace;
}

.context-bar .selected-text {
  font-size: 11px;
  color: var(--color-text-muted);
  font-style: italic;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.input-wrapper {
  display: flex;
  align-items: flex-end;
  gap: 0.5rem;
  background: #f5f5f5;
  border: 1px solid var(--color-border);
  border-radius: 12px;
  padding: 0.5rem;
  transition: border-color 0.2s;
}

.input-wrapper:focus-within {
  border-color: var(--color-primary);
  background: #fff;
}

.chat-input textarea {
  flex: 1;
  min-height: 24px;
  max-height: 120px;
  padding: 0.5rem;
  border: none;
  background: transparent;
  resize: none;
  font-size: 0.875rem;
  line-height: 1.5;
  outline: none;
  font-family: inherit;
}

.send-btn {
  width: 36px;
  height: 36px;
  min-width: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s;
  padding: 0;
}

.send-btn:hover:not(:disabled) {
  opacity: 0.9;
  transform: scale(1.05);
}

.send-btn:active:not(:disabled) {
  transform: scale(0.95);
}

.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* 停止按钮样式 */
.send-btn.stop-btn {
  background: #dc3545;
}

.send-btn.stop-btn:hover {
  background: #c82333;
  opacity: 1;
}
</style>

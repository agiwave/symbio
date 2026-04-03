<template>
  <div class="ai-chat-panel">
    <!-- 消息历史 -->
    <div class="chat-messages" ref="messagesRef">
      <div v-if="messages.length === 0 && !isLoading" class="empty-chat">
        <p>开始与 AI 对话</p>
      </div>

      <div
        v-for="(msg, index) in messages"
        :key="`${sessionId}-${index}-${msg.timestamp}`"
        class="message"
        :class="msg.role"
        v-show="msg.role !== 'tool'"
      >
        <div class="message-avatar">
          {{ msg.role === 'user' ? '👤' : '🤖' }}
        </div>
        <div class="message-content">
          <!-- 工具调用信息 -->
          <div v-if="msg.role === 'assistant' && getToolCalls(msg).length > 0" class="tool-calls">
            <div v-for="(tool, idx) in getToolCalls(msg)" :key="idx" class="tool-call">
              <div class="tool-call-header">
                <span class="tool-call-icon">🔧</span>
                <span class="tool-call-name">{{ tool.name }}</span>
              </div>
              <div v-if="tool.args" class="tool-call-args">
                <pre>{{ formatJson(tool.args) }}</pre>
              </div>
              <div v-if="tool.result" class="tool-call-result">
                <pre>{{ tool.result }}</pre>
              </div>
            </div>
          </div>

          <div class="message-text" v-html="renderMarkdown(msg.content)"></div>
          <div class="message-time">{{ formatTime(msg.timestamp) }}</div>
        </div>
      </div>

      <!-- 流式加载指示器 -->
      <div v-if="isLoading" class="message assistant loading">
        <div class="message-avatar">🤖</div>
        <div class="message-content">
          <!-- 工具调用信息 -->
          <div v-if="toolCalls.length > 0" class="tool-calls">
            <div v-for="(tool, idx) in toolCalls" :key="idx" class="tool-call">
              <div class="tool-call-header">
                <span class="tool-call-icon">🔧</span>
                <span class="tool-call-name">{{ tool.name }}</span>
              </div>
              <div v-if="tool.args" class="tool-call-args">
                <pre>{{ formatJson(tool.args) }}</pre>
              </div>
              <div v-if="tool.result" class="tool-call-result">
                <pre>{{ tool.result }}</pre>
              </div>
            </div>
          </div>
          
          <!-- 显示流式内容（如果有） -->
          <div v-if="streamingContent" class="message-text" v-html="renderMarkdown(streamingContent)"></div>
          <!-- 否则显示打字指示器 -->
          <div v-else-if="toolCalls.length === 0" class="typing-indicator">
            <span></span><span></span><span></span>
          </div>
        </div>
      </div>
    </div>

    <!-- 输入区域 -->
    <div class="chat-input">
      <div v-if="configError" class="config-error">
        {{ configError }}
      </div>
      <div class="input-wrapper">
        <textarea
          v-model="inputText"
          placeholder="输入消息..."
          @keydown.enter.exact="handleKeydown"
          :disabled="isLoading"
          rows="1"
        ></textarea>
        <button class="send-btn" @click="handleSend" :disabled="!inputText.trim() || isLoading" title="发送">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="22" y1="2" x2="11" y2="13"></line>
            <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, watch } from 'vue'
import { marked } from 'marked'
import { sendMessageStream, type ChatMessage, type StreamChunk } from '../services/ai'
import {
  getSession,
  createSessionId,
  type SessionMessage,
} from '../services/session'

// Props
const props = defineProps<{
  sessionId: string | null
  messages: SessionMessage[]
  onUpdateMessages: (messages: SessionMessage[]) => void
  onSendComplete?: () => void
}>()

// 配置 marked
marked.setOptions({
  breaks: true,
  gfm: true
})

// 状态
const inputText = ref('')
const isLoading = ref(false)
const messagesRef = ref<HTMLElement | null>(null)
const configError = ref<string | null>(null)
const streamingContent = ref('')
const toolCalls = ref<Array<{ name: string; args: string; result?: string }>>([])

// 渲染 Markdown
function renderMarkdown(content: string): string {
  try {
    return marked.parse(content) as string
  } catch {
    return content
  }
}

// 格式化 JSON
function formatJson(str: string): string {
  try {
    return JSON.stringify(JSON.parse(str), null, 2)
  } catch {
    return str
  }
}

// 格式化时间
function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000)
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

// 获取消息的工具调用信息
function getToolCalls(msg: SessionMessage) {
  if (msg.tool_calls && msg.tool_calls.length > 0) {
    return msg.tool_calls.map(tc => ({
      name: tc.function?.name || 'unknown',
      args: tc.function?.arguments || '',
      result: undefined
    }))
  }
  return []
}

// 滚动到底部
function scrollToBottom() {
  if (messagesRef.value) {
    messagesRef.value.scrollTop = messagesRef.value.scrollHeight
  }
}

// 发送消息
async function handleSend() {
  const text = inputText.value.trim()
  if (!text || isLoading.value || !props.sessionId) return

  const now = Math.floor(Date.now() / 1000)

  // 添加用户消息
  const userMessage: SessionMessage = {
    role: 'user',
    content: text,
    timestamp: now
  }

  const newMessages = [...props.messages, userMessage]
  props.onUpdateMessages(newMessages)

  inputText.value = ''
  isLoading.value = true
  configError.value = null
  streamingContent.value = ''
  toolCalls.value = []

  await nextTick()
  scrollToBottom()

  try {
    // 构建消息历史
    const chatMessages: ChatMessage[] = newMessages.map(m => ({
      role: m.role as 'user' | 'assistant',
      content: m.content
    }))

    // 流式发送消息
    const response = await sendMessageStream(
      chatMessages,
      props.sessionId || 'default',
      (chunk: StreamChunk) => {
        if (chunk.data && typeof chunk.data === 'object') {
          const data = chunk.data as Record<string, unknown>

          // 检查工具调用
          if (data.tool_calls && Array.isArray(data.tool_calls)) {
            toolCalls.value = data.tool_calls.map((tc: any) => ({
              name: tc.function?.name || tc.name || 'unknown',
              args: tc.function?.arguments || tc.arguments || '',
              result: tc.result
            }))
          }

          // 检查内容
          if (data.content && typeof data.content === 'string') {
            streamingContent.value = data.content as string
          }

          // 检查错误
          if (data.error && typeof data.error === 'string') {
            configError.value = data.error as string
          }
        }

        scrollToBottom()
      }
    )

    // 流完成 - 添加助手消息
    if (response.error) {
      configError.value = response.error
      props.onUpdateMessages([...newMessages, {
        role: 'assistant',
        content: `错误: ${response.error}`,
        timestamp: Math.floor(Date.now() / 1000)
      }])
    } else if (streamingContent.value) {
      props.onUpdateMessages([...newMessages, {
        role: 'assistant',
        content: streamingContent.value,
        timestamp: Math.floor(Date.now() / 1000)
      }])
    }

    props.onSendComplete?.()
  } catch (err) {
    configError.value = err instanceof Error ? err.message : '发送失败'
    props.onUpdateMessages([...newMessages, {
      role: 'assistant',
      content: `发送失败: ${configError.value}`,
      timestamp: Math.floor(Date.now() / 1000)
    }])
  } finally {
    isLoading.value = false
    streamingContent.value = ''
    toolCalls.value = []
    scrollToBottom()
  }
}

function handleKeydown(e: KeyboardEvent) {
  if (!e.shiftKey) {
    e.preventDefault()
    handleSend()
  }
}

// 监听消息变化，滚动到底部
watch(() => props.messages.length, () => {
  nextTick(scrollToBottom)
})
</script>

<style scoped>
.ai-chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.chat-messages {
  flex: 1;
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

/* 工具调用样式 */
.tool-calls {
  margin-bottom: 0.75rem;
}

.tool-call {
  background: #f0f0f0;
  border-radius: 8px;
  padding: 0.75rem;
  margin-bottom: 0.5rem;
  font-size: 0.8rem;
}

.tool-call:last-child {
  margin-bottom: 0;
}

.tool-call-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
}

.tool-call-icon {
  font-size: 1rem;
}

.tool-call-name {
  font-weight: 600;
  color: var(--color-text);
}

.tool-call-args,
.tool-call-result {
  background: #1e1e1e;
  color: #d4d4d4;
  border-radius: 4px;
  padding: 0.5rem;
  margin-top: 0.5rem;
  overflow-x: auto;
}

.tool-call-args pre,
.tool-call-result pre {
  margin: 0;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.75rem;
  line-height: 1.4;
  white-space: pre-wrap;
  word-break: break-word;
}

.tool-call-result {
  background: #e8f5e9;
  color: #2e7d32;
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

.config-error {
  padding: 0.5rem 0.75rem;
  margin-bottom: 0.5rem;
  background: #fef3cd;
  color: #856404;
  border-radius: 6px;
  font-size: 0.8rem;
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
  container-type: inline-size;
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

.chat-input textarea:disabled {
  opacity: 0.6;
}

/* 窄屏时隐藏 placeholder */
@container (max-width: 200px) {
  .chat-input textarea::placeholder {
    color: transparent;
  }
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
</style>

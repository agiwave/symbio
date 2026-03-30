<template>
  <aside class="ai-sidebar" :class="{ collapsed: !visible, 'full-width': fullWidth && visible }">
    <div class="sidebar-header">
      <span class="title">Agent</span>
      <button class="close-btn" @click="$emit('close')" title="隐藏">×</button>
    </div>
    
    <div class="chat-history" ref="historyRef">
      <div v-if="messages.length === 0" class="empty-state">
        <p>开始与 AI 对话</p>
        <p class="hint">输入问题或粘贴代码进行分析</p>
      </div>
      
      <div 
        v-for="(msg, index) in messages" 
        :key="index" 
        class="message"
        :class="msg.role"
      >
        <div class="message-avatar">
          {{ msg.role === 'user' ? '👤' : '🤖' }}
        </div>
        <div class="message-content">
          <div class="message-text">{{ msg.content }}</div>
          <div class="message-time">{{ formatTime(msg.timestamp) }}</div>
        </div>
      </div>
      
      <div v-if="isLoading" class="message assistant loading">
        <div class="message-avatar">🤖</div>
        <div class="message-content">
          <div class="typing-indicator">
            <span></span><span></span><span></span>
          </div>
        </div>
      </div>
    </div>
    
    <div class="input-area">
      <textarea
        v-model="inputText"
        class="input-field"
        placeholder="输入消息... (Enter 发送, Shift+Enter 换行)"
        @keydown.enter.exact.prevent="sendMessage"
        :disabled="isLoading"
      ></textarea>
      <button class="send-btn" @click="sendMessage" :disabled="!inputText.trim() || isLoading">
        发送
      </button>
    </div>
  </aside>
</template>

<script setup lang="ts">
import { ref, nextTick, watch } from 'vue'

interface Message {
  role: 'user' | 'assistant'
  content: string
  timestamp: number
}

const props = defineProps<{
  visible: boolean
  fullWidth?: boolean
}>()

const emit = defineEmits<{
  close: []
  send: [message: string]
}>()

const messages = ref<Message[]>([])
const inputText = ref('')
const isLoading = ref(false)
const historyRef = ref<HTMLElement | null>(null)

function formatTime(timestamp: number): string {
  const date = new Date(timestamp)
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

async function sendMessage() {
  const text = inputText.value.trim()
  if (!text || isLoading.value) return
  
  messages.value.push({
    role: 'user',
    content: text,
    timestamp: Date.now(),
  })
  
  inputText.value = ''
  isLoading.value = true
  
  await nextTick()
  scrollToBottom()
  
  emit('send', text)
}

function addAssistantResponse(content: string) {
  messages.value.push({
    role: 'assistant',
    content,
    timestamp: Date.now(),
  })
  isLoading.value = false
  scrollToBottom()
}

function scrollToBottom() {
  if (historyRef.value) {
    historyRef.value.scrollTop = historyRef.value.scrollHeight
  }
}

defineExpose({
  addResponse: addAssistantResponse,
  setLoading: (loading: boolean) => { isLoading.value = loading },
  clearMessages: () => { messages.value = [] },
})

watch(() => props.visible, (visible) => {
  if (visible) {
    nextTick(scrollToBottom)
  }
})
</script>

<style scoped>
.ai-sidebar {
  width: 320px;
  background: var(--color-surface);
  border-left: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  transition: width 0.3s ease;
  flex-shrink: 0;
}

.ai-sidebar.collapsed {
  width: 0;
  overflow: hidden;
  border-left: none;
}

/* 只显示AI时，占满整个窗口 */
.ai-sidebar.full-width {
  flex: 1;
  width: auto;
  border-left: none;
}

.sidebar-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
}

.title {
  font-weight: 600;
  font-size: 0.875rem;
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 1.25rem;
  color: var(--color-text-muted);
  border-radius: 4px;
}

.close-btn:hover {
  background: #f0f0f0;
}

.chat-history {
  flex: 1;
  overflow-y: auto;
  padding: 1rem;
}

.empty-state {
  text-align: center;
  color: var(--color-text-muted);
  padding: 2rem;
}

.empty-state p {
  margin: 0.5rem 0;
}

.empty-state .hint {
  font-size: 0.75rem;
  opacity: 0.7;
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
  max-width: 80%;
}

.message.user .message-content {
  text-align: right;
}

.message-text {
  padding: 0.5rem 0.75rem;
  border-radius: 12px;
  font-size: 0.875rem;
  line-height: 1.5;
  word-break: break-word;
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

.input-area {
  padding: 0.75rem;
  border-top: 1px solid var(--color-border);
}

.input-field {
  width: 100%;
  min-height: 60px;
  max-height: 120px;
  padding: 0.5rem;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  resize: none;
  font-size: 0.875rem;
  line-height: 1.5;
  outline: none;
}

.input-field:focus {
  border-color: var(--color-primary);
}

.send-btn {
  width: 100%;
  margin-top: 0.5rem;
  padding: 0.5rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 0.875rem;
  font-weight: 500;
  transition: opacity 0.2s;
}

.send-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.send-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 过渡动画 */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
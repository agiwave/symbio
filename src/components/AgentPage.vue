<template>
  <div class="agent-page">
    <!-- 左侧：对话历史列表 -->
    <aside class="history-list">
      <div class="history-header">
        <h3>对话历史</h3>
        <button class="icon-btn" @click="createNewChat" title="新对话">+</button>
      </div>
      <div class="history-content">
        <div v-if="chats.length === 0" class="empty-state">
          <p>暂无对话</p>
          <button @click="createNewChat">开始新对话</button>
        </div>
        <div
          v-for="chat in chats"
          :key="chat.id"
          class="history-item"
          :class="{ active: activeChatId === chat.id }"
          @click="selectChat(chat.id)"
        >
          <div class="chat-title">{{ chat.title }}</div>
          <div class="chat-meta">
            <span class="chat-time">{{ formatTime(chat.updatedAt) }}</span>
            <span class="chat-count">{{ chat.messageCount }} 条消息</span>
          </div>
        </div>
      </div>
    </aside>

    <!-- 右侧：对话区 -->
    <main class="chat-area">
      <div v-if="activeChat" class="chat-container">
        <header class="chat-header">
          <input
            v-model="activeChat.title"
            class="chat-title-input"
            placeholder="未命名对话"
          />
          <button class="delete-btn" @click="deleteChat(activeChat.id)" title="删除对话">
            🗑️
          </button>
        </header>
        
        <div class="chat-messages" ref="messagesRef">
          <div v-if="activeChat.messages.length === 0" class="empty-chat">
            <p>开始与 AI 对话</p>
            <p class="hint">输入问题或粘贴代码进行分析</p>
          </div>
          
          <div
            v-for="(msg, index) in activeChat.messages"
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
        
        <div class="chat-input">
          <div v-if="configError" class="config-error">
            {{ configError }}
          </div>
          <textarea
            v-model="inputText"
            placeholder="输入消息... (Enter 发送, Shift+Enter 换行)"
            @keydown.enter.exact="handleKeydown"
            :disabled="isLoading"
          ></textarea>
          <button @click="sendMessageToAI" :disabled="!inputText.trim() || isLoading">
            发送
          </button>
        </div>
      </div>
      
      <div v-else class="no-chat-selected">
        <p>选择一个对话或创建新对话</p>
        <button @click="createNewChat">创建新对话</button>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, nextTick, watch, onMounted } from 'vue'
import { sendMessage, getProviderConfig, type ChatMessage } from '../services/ai'

interface Message {
  role: 'user' | 'assistant'
  content: string
  timestamp: number
}

interface Chat {
  id: string
  title: string
  messages: Message[]
  createdAt: number
  updatedAt: number
  messageCount: number
}

// 状态
const chats = ref<Chat[]>([])
const activeChatId = ref<string | null>(null)
const inputText = ref('')
const isLoading = ref(false)
const messagesRef = ref<HTMLElement | null>(null)
const configError = ref<string | null>(null)

const activeChat = computed(() => 
  chats.value.find(c => c.id === activeChatId.value) || null
)

// 生成 ID
function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2)
}

// 格式化时间
function formatTime(timestamp: number): string {
  const date = new Date(timestamp)
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

// 创建新对话
function createNewChat() {
  const chat: Chat = {
    id: generateId(),
    title: '新对话',
    messages: [],
    createdAt: Date.now(),
    updatedAt: Date.now(),
    messageCount: 0
  }
  chats.value.unshift(chat)
  activeChatId.value = chat.id
}

// 选择对话
function selectChat(id: string) {
  activeChatId.value = id
}

// 删除对话
function deleteChat(id: string) {
  if (confirm('确定要删除此对话吗？')) {
    const index = chats.value.findIndex(c => c.id === id)
    if (index !== -1) {
      chats.value.splice(index, 1)
      if (activeChatId.value === id) {
        activeChatId.value = chats.value[0]?.id || null
      }
    }
  }
}

// 发送消息
async function sendMessageToAI() {
  const text = inputText.value.trim()
  if (!text || isLoading.value || !activeChat.value) return

  const message: Message = {
    role: 'user',
    content: text,
    timestamp: Date.now()
  }
  
  activeChat.value.messages.push(message)
  activeChat.value.updatedAt = Date.now()
  activeChat.value.messageCount = activeChat.value.messages.length
  
  // 更新标题（如果是第一条消息）
  if (activeChat.value.messages.length === 1) {
    activeChat.value.title = text.slice(0, 20) + (text.length > 20 ? '...' : '')
  }
  
  inputText.value = ''
  isLoading.value = true
  configError.value = null

  await nextTick()
  scrollToBottom()

  try {
    // 构建消息历史
    const messages: ChatMessage[] = activeChat.value.messages.map(m => ({
      role: m.role as 'user' | 'assistant',
      content: m.content
    }))

    const response = await sendMessage(messages)
    
    if (response.error) {
      configError.value = response.error
      if (activeChat.value) {
        activeChat.value.messages.push({
          role: 'assistant',
          content: `错误: ${response.error}`,
          timestamp: Date.now()
        })
      }
    } else if (response.content) {
      if (activeChat.value) {
        activeChat.value.messages.push({
          role: 'assistant',
          content: response.content,
          timestamp: Date.now()
        })
        activeChat.value.updatedAt = Date.now()
        activeChat.value.messageCount = activeChat.value.messages.length
      }
    }
  } catch (err) {
    configError.value = err instanceof Error ? err.message : '发送失败'
    if (activeChat.value) {
      activeChat.value.messages.push({
        role: 'assistant',
        content: `发送失败: ${configError.value}`,
        timestamp: Date.now()
      })
    }
  } finally {
    isLoading.value = false
    scrollToBottom()
  }
}

function handleKeydown(e: KeyboardEvent) {
  if (!e.shiftKey) {
    e.preventDefault()
    sendMessageToAI()
  }
}

function scrollToBottom() {
  if (messagesRef.value) {
    messagesRef.value.scrollTop = messagesRef.value.scrollHeight
  }
}

// 检查配置
async function checkConfig() {
  try {
    const result = await getProviderConfig()
    // openai 返回 { success, config: { api_key_set, ... } }
    const hasApiKey = result.config?.api_key_set ?? false
    if (!hasApiKey) {
      configError.value = '请先配置 API Key（在设置页面）'
    } else {
      configError.value = null
    }
  } catch (err) {
    configError.value = err instanceof Error ? err.message : '获取配置失败'
  }
}

watch(activeChatId, () => {
  nextTick(scrollToBottom)
})

onMounted(() => {
  checkConfig()
})
</script>

<style scoped>
.agent-page {
  display: flex;
  height: 100%;
  width: 100%;
}

/* 左侧历史列表 */
.history-list {
  width: 280px;
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.history-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.history-header h3 {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--color-text-secondary);
}

.icon-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: var(--color-primary);
  color: white;
  border-radius: 4px;
  cursor: pointer;
  font-size: 1rem;
  line-height: 1;
}

.history-content {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
}

.empty-state {
  text-align: center;
  padding: 2rem 1rem;
  color: var(--color-text-muted);
}

.empty-state button {
  margin-top: 1rem;
  padding: 0.5rem 1rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
}

.history-item {
  padding: 0.75rem;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.2s;
  margin-bottom: 0.25rem;
}

.history-item:hover {
  background: #f0f0f0;
}

.history-item.active {
  background: #e8e8f0;
}

.chat-title {
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chat-meta {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.25rem;
  font-size: 0.75rem;
  color: var(--color-text-muted);
}

/* 右侧对话区 */
.chat-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.chat-container {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.chat-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.chat-title-input {
  flex: 1;
  border: none;
  font-size: 1rem;
  font-weight: 600;
  background: transparent;
  outline: none;
}

.delete-btn {
  width: 32px;
  height: 32px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 1rem;
  transition: background 0.2s;
}

.delete-btn:hover {
  background: #fee2e2;
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

.empty-chat .hint {
  font-size: 0.75rem;
  opacity: 0.7;
  margin-top: 0.5rem;
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

.chat-input {
  padding: 1rem;
  border-top: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.config-error {
  padding: 0.5rem 0.75rem;
  margin-bottom: 0.75rem;
  background: #fef3cd;
  color: #856404;
  border-radius: 6px;
  font-size: 0.8rem;
}

.chat-input textarea {
  width: 100%;
  min-height: 80px;
  max-height: 160px;
  padding: 0.75rem;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  resize: none;
  font-size: 0.875rem;
  line-height: 1.5;
  outline: none;
}

.chat-input textarea:focus {
  border-color: var(--color-primary);
}

.chat-input button {
  width: 100%;
  margin-top: 0.75rem;
  padding: 0.75rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-weight: 500;
}

.chat-input button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.no-chat-selected {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
}

.no-chat-selected button {
  margin-top: 1rem;
  padding: 0.75rem 1.5rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
}
</style>

<template>
  <div class="agent-page">
    <!-- 左侧：对话历史列表 -->
    <aside class="history-list">
      <div class="history-header">
        <h3>对话历史</h3>
        <button class="icon-btn" @click="createNewChat" title="新对话">+</button>
      </div>
      <div class="history-content">
        <div v-if="loadingSessions" class="loading-state">
          加载中...
        </div>
        <div v-else-if="sessions.length === 0" class="empty-state">
          <p>暂无对话</p>
          <button @click="createNewChat">开始新对话</button>
        </div>
        <div
          v-for="session in sessions"
          :key="session.id"
          class="history-item"
          :class="{ active: activeSessionId === session.id }"
          @click="selectSession(session.id)"
        >
          <div class="chat-title">{{ getSessionTitle(session.id) }}</div>
          <div class="chat-meta">
            <span class="chat-time">{{ formatTime(session.updated_at) }}</span>
            <span class="chat-count">{{ session.message_count }} 条消息</span>
          </div>
        </div>
      </div>
    </aside>

    <!-- 右侧：对话区 -->
    <main class="chat-area">
      <div v-if="activeSession" class="chat-container">
        <header class="chat-header">
          <input
            v-model="sessionTitle"
            class="chat-title-input"
            placeholder="未命名对话"
            @blur="updateSessionTitle"
          />
          <button class="delete-btn" @click="deleteSession(activeSessionId)" title="删除对话">
            🗑️
          </button>
        </header>
        
        <div class="chat-messages" ref="messagesRef">
          <div v-if="activeSession.messages.length === 0" class="empty-chat">
            <p>开始与 AI 对话</p>
            <p class="hint">输入问题或粘贴代码进行分析</p>
          </div>
          
          <div
            v-for="(msg, index) in activeSession.messages"
            :key="`${activeSession.id}-${index}-${msg.timestamp}`"
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
import { marked } from 'marked'
import { sendMessage, getProviderConfig, type ChatMessage } from '../services/ai'
import { 
  listSessions, 
  getSession, 
  appendMessages, 
  clearSession,
  createSessionId,
  type Session,
  type SessionMessage,
  type SessionListItem
} from '../services/session'

// 配置 marked
marked.setOptions({
  breaks: true,
  gfm: true
})

// 状态
const sessions = ref<SessionListItem[]>([])
const activeSessionId = ref<string | null>(null)
const activeSession = ref<Session | null>(null)
const sessionTitles = ref<Map<string, string>>(new Map())
const sessionTitle = ref('')
const inputText = ref('')
const isLoading = ref(false)
const loadingSessions = ref(false)
const messagesRef = ref<HTMLElement | null>(null)
const configError = ref<string | null>(null)

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
  const date = new Date(timestamp * 1000) // 后端返回的是秒级时间戳
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

// 获取会话标题
function getSessionTitle(sessionId: string): string {
  return sessionTitles.value.get(sessionId) || '新对话'
}

// 加载会话列表
async function loadSessions() {
  loadingSessions.value = true
  try {
    sessions.value = await listSessions()
    // 加载每个会话的标题
    for (const s of sessions.value) {
      if (!sessionTitles.value.has(s.id)) {
        const session = await getSession(s.id)
        if (session.messages.length > 0) {
          const firstMsg = session.messages[0].content
          const title = firstMsg.slice(0, 20) + (firstMsg.length > 20 ? '...' : '')
          sessionTitles.value.set(s.id, title)
        }
      }
    }
  } catch (err) {
    console.error('加载会话列表失败:', err)
  } finally {
    loadingSessions.value = false
  }
}

// 创建新对话
async function createNewChat() {
  const id = createSessionId()
  const now = Math.floor(Date.now() / 1000)
  
  const newSession: Session = {
    id,
    messages: [],
    created_at: now,
    updated_at: now,
    metadata: {}
  }
  
  // 添加到列表
  sessions.value.unshift({
    id,
    message_count: 0,
    updated_at: now
  })
  
  sessionTitles.value.set(id, '新对话')
  activeSessionId.value = id
  activeSession.value = newSession
  sessionTitle.value = '新对话'
}

// 选择会话
async function selectSession(id: string) {
  if (activeSessionId.value === id) return
  
  activeSessionId.value = id
  
  try {
    const session = await getSession(id)
    console.log('[AgentPage] 加载会话:', id, session)
    activeSession.value = session
    sessionTitle.value = getSessionTitle(id)
    await nextTick()
    scrollToBottom()
  } catch (err) {
    console.error('加载会话失败:', err)
  }
}

// 删除会话
async function deleteSession(id: string | null) {
  if (!id) return
  if (!confirm('确定要删除此对话吗？')) return
  
  try {
    await clearSession(id)
    const index = sessions.value.findIndex(s => s.id === id)
    if (index !== -1) {
      sessions.value.splice(index, 1)
      sessionTitles.value.delete(id)
    }
    
    if (activeSessionId.value === id) {
      activeSessionId.value = sessions.value[0]?.id || null
      if (activeSessionId.value) {
        await selectSession(activeSessionId.value)
      } else {
        activeSession.value = null
      }
    }
  } catch (err) {
    console.error('删除会话失败:', err)
  }
}

// 更新会话标题
function updateSessionTitle() {
  if (activeSessionId.value && sessionTitle.value) {
    sessionTitles.value.set(activeSessionId.value, sessionTitle.value)
  }
}

// 发送消息
async function sendMessageToAI() {
  const text = inputText.value.trim()
  if (!text || isLoading.value || !activeSession.value || !activeSessionId.value) return

  const now = Math.floor(Date.now() / 1000)
  
  const userMessage: SessionMessage = {
    role: 'user',
    content: text,
    timestamp: now
  }
  
  activeSession.value.messages.push(userMessage)
  activeSession.value.updated_at = now
  
  // 更新标题（如果是第一条消息）
  if (activeSession.value.messages.length === 1) {
    const title = text.slice(0, 20) + (text.length > 20 ? '...' : '')
    sessionTitles.value.set(activeSessionId.value, title)
    sessionTitle.value = title
  }
  
  inputText.value = ''
  isLoading.value = true
  configError.value = null

  await nextTick()
  scrollToBottom()

  try {
    // 构建消息历史
    const messages: ChatMessage[] = activeSession.value.messages.map(m => ({
      role: m.role as 'user' | 'assistant',
      content: m.content
    }))

    const response = await sendMessage(messages)
    
    const assistantMessage: SessionMessage = {
      role: 'assistant',
      content: '',
      timestamp: Math.floor(Date.now() / 1000)
    }
    
    if (response.error) {
      configError.value = response.error
      assistantMessage.content = `错误: ${response.error}`
    } else if (response.content) {
      assistantMessage.content = response.content
    }
    
    activeSession.value.messages.push(assistantMessage)
    activeSession.value.updated_at = Math.floor(Date.now() / 1000)
    
    // 保存到后端
    await appendMessages(activeSessionId.value, [userMessage, assistantMessage])
    
    // 更新列表中的消息数
    const listItem = sessions.value.find(s => s.id === activeSessionId.value)
    if (listItem) {
      listItem.message_count = activeSession.value.messages.length
      listItem.updated_at = activeSession.value.updated_at
    }
  } catch (err) {
    configError.value = err instanceof Error ? err.message : '发送失败'
    if (activeSession.value) {
      activeSession.value.messages.push({
        role: 'assistant',
        content: `发送失败: ${configError.value}`,
        timestamp: Math.floor(Date.now() / 1000)
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
    const hasApiKey = result.config?.api_key && result.config.api_key.length > 0
    if (!hasApiKey) {
      configError.value = '请先配置 API Key（在设置页面）'
    } else {
      configError.value = null
    }
  } catch (err) {
    configError.value = err instanceof Error ? err.message : '获取配置失败'
  }
}

watch(activeSessionId, () => {
  console.log('[AgentPage] activeSessionId changed to:', activeSessionId.value)
  nextTick(scrollToBottom)
})

// 监听 activeSession 变化
watch(activeSession, (newVal) => {
  console.log('[AgentPage] activeSession changed:', newVal?.id, 'messages:', newVal?.messages?.length)
}, { deep: true })

onMounted(async () => {
  await checkConfig()
  await loadSessions()
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

.loading-state {
  text-align: center;
  padding: 2rem 1rem;
  color: var(--color-text-muted);
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

.message-text :deep(ul),
.message-text :deep(ol) {
  margin: 0.5rem 0;
  padding-left: 1.5rem;
}

.message-text :deep(li) {
  margin: 0.25rem 0;
}

.message-text :deep(blockquote) {
  border-left: 3px solid var(--color-primary);
  margin: 0.5rem 0;
  padding-left: 1rem;
  color: #666;
}

.message-text :deep(a) {
  color: var(--color-primary);
}

.message-text :deep(table) {
  border-collapse: collapse;
  margin: 0.5rem 0;
}

.message-text :deep(th),
.message-text :deep(td) {
  border: 1px solid #ddd;
  padding: 0.5rem;
}

.message-text :deep(th) {
  background: #f5f5f5;
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
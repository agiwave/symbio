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
          <div v-if="activeSession.messages.length === 0 && !isLoading" class="empty-chat">
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
              <!-- 工具调用信息 - 支持实时对话和历史加载两种情况 -->
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
        
        <div class="chat-input">
          <div v-if="configError" class="config-error">
            {{ configError }}
          </div>
          <div class="input-wrapper">
            <textarea
              v-model="inputText"
              placeholder="输入消息... (Enter 发送, Shift+Enter 换行)"
              @keydown.enter.exact="handleKeydown"
              :disabled="isLoading"
              rows="1"
            ></textarea>
            <button class="send-btn" @click="sendMessageToAI" :disabled="!inputText.trim() || isLoading" title="发送">
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="22" y1="2" x2="11" y2="13"></line>
                <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
              </svg>
            </button>
          </div>
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
import { ref, nextTick, watch, onMounted } from 'vue'
import { marked } from 'marked'
import { sendMessageStream, getProviderConfig, type ChatMessage, type StreamChunk } from '../services/ai'
import {
  listSessions,
  getSession,
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

// 流式消息状态
const streamingContent = ref('')
const toolCalls = ref<Array<{ name: string; args: string; result?: string }>>([])

// 获取消息的工具调用信息（支持实时和历史两种情况）
function getToolCalls(msg: SessionMessage) {
  // 如果是历史消息，从消息中获取 tool_calls
  if (msg.tool_calls && msg.tool_calls.length > 0) {
    return msg.tool_calls.map(tc => ({
      name: tc.function?.name || 'unknown',
      args: tc.function?.arguments || '',
      result: undefined // 历史消息中没有 result
    }))
  }
  
  return []
}

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

// 发送消息（流式）
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
  streamingContent.value = ''
  toolCalls.value = []

  await nextTick()
  scrollToBottom()

  try {
    // 构建消息历史
    const messages: ChatMessage[] = activeSession.value.messages.map(m => ({
      role: m.role as 'user' | 'assistant',
      content: m.content
    }))

    // 不创建临时消息，使用单独的流式显示区域
    // 流式内容会显示在 isLoading 区域的 streamingContent 中

    // 流式发送消息
    const response = await sendMessageStream(
      messages,
      activeSessionId.value || 'default',
      (chunk: StreamChunk) => {
        console.log('[AgentPage] Stream chunk:', chunk)

        // 处理工具调用信息
        if (chunk.data && typeof chunk.data === 'object') {
          const data = chunk.data as Record<string, unknown>

          // 检查是否有工具调用
          if (data.tool_calls && Array.isArray(data.tool_calls)) {
            toolCalls.value = data.tool_calls.map((tc: any) => ({
              name: tc.function?.name || tc.name || 'unknown',
              args: tc.function?.arguments || tc.arguments || '',
              result: tc.result
            }))
          }

          // 检查是否有内容 - 只更新 streamingContent
          if (data.content && typeof data.content === 'string') {
            streamingContent.value = data.content as string
          }

          // 检查 data 内部错误
          if (data.error && typeof data.error === 'string') {
            configError.value = data.error as string
          }
        }

        // 检查顶层错误（chunk.error）
        if (chunk.error && typeof chunk.error === 'string') {
          console.error('[AgentPage] Chunk top-level error:', chunk.error)
          configError.value = chunk.error
        }

        // 立即滚动到底部
        scrollToBottom()
      }
    )

    console.log('[AgentPage] Stream completed, response:', response)

    // 流完成 - 将最终内容作为新消息添加到消息列表
    if (response.error) {
      console.error('[AgentPage] Response error:', response.error)
      configError.value = response.error
      activeSession.value.messages.push({
        role: 'assistant',
        content: `错误: ${response.error}`,
        timestamp: Math.floor(Date.now() / 1000)
      })
    } else if (streamingContent.value) {
      // 使用流式内容作为新消息
      activeSession.value.messages.push({
        role: 'assistant',
        content: streamingContent.value,
        timestamp: Math.floor(Date.now() / 1000)
      })
    }

    activeSession.value.updated_at = Math.floor(Date.now() / 1000)

    // 后端 openai 插件已经自动将消息保存到 session，不需要前端再保存
    // 但需要更新列表中的消息数
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
    streamingContent.value = ''
    toolCalls.value = []
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
    console.log('[AgentPage] getProviderConfig result:', result)
    // openai 插件返回 { success: true, config: { api_key, ... } }
    const config = result.config as { api_key?: string; api_base?: string; model?: string } | undefined
    const hasApiKey = config?.api_key && config.api_key.length > 0
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
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

/* 左侧历史列表 */
.history-list {
  width: 280px;
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  min-height: 0;
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
  min-height: 0;
  overflow: hidden;
}

.chat-container {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
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
  min-height: 0;
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
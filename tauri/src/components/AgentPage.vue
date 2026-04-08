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
        
        <!-- 使用统一的 AIChatPanel 组件 -->
        <AIChatPanel
          :key="activeSessionId || 'default'"
          :sessionId="activeSessionId || 'default'"
          :messages="activeSession.messages"
          :onUpdateMessages="handleUpdateMessages"
          :onSendComplete="handleSendComplete"
        />
      </div>
      
      <div v-else class="no-chat-selected">
        <p>选择一个对话或创建新对话</p>
        <button @click="createNewChat">创建新对话</button>
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import AIChatPanel from './AIChatPanel.vue'
import { getProviderConfig } from '../services/ai'
import {
  listSessions,
  getSession,
  clearSession,
  createSessionId,
  type Session,
  type SessionMessage,
  type SessionListItem
} from '../services/session'

// 状态
const sessions = ref<SessionListItem[]>([])
const activeSessionId = ref<string | null>(null)
const activeSession = ref<Session | null>(null)
const sessionTitles = ref<Map<string, string>>(new Map())
const sessionTitle = ref('')
const loadingSessions = ref(false)
const configError = ref<string | null>(null)

// 格式化时间
function formatTime(timestamp: number): string {
  const date = new Date(timestamp * 1000)
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
    activeSession.value = session
    sessionTitle.value = getSessionTitle(id)
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

// 处理消息更新
function handleUpdateMessages(messages: SessionMessage[]) {
  if (activeSession.value) {
    activeSession.value.messages = messages
    activeSession.value.updated_at = Math.floor(Date.now() / 1000)
    
    // 更新标题（如果是第一条消息）
    if (messages.length === 1 && activeSessionId.value) {
      const firstMsg = messages[0].content
      const title = firstMsg.slice(0, 20) + (firstMsg.length > 20 ? '...' : '')
      sessionTitles.value.set(activeSessionId.value, title)
      sessionTitle.value = title
    }
  }
}

// 处理发送完成
function handleSendComplete() {
  if (activeSessionId.value && activeSession.value) {
    const listItem = sessions.value.find(s => s.id === activeSessionId.value)
    if (listItem) {
      listItem.message_count = activeSession.value.messages.length
      listItem.updated_at = activeSession.value.updated_at
    }
  }
}

// 检查配置
async function checkConfig() {
  try {
    const result = await getProviderConfig()
    console.log('[AgentPage] getProviderConfig result:', result)
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

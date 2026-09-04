<template>
  <div class="chat-main-panel">
    <header v-if="hasActive" class="chat-header">
      <div class="header-left">
        <h2 class="session-name">{{ store.activeTitle }}</h2>
        <span v-if="store.isActiveWorking" class="status-working">● AI 处理中</span>
      </div>
      <div class="header-right">
        <button class="header-btn" title="清空历史" @click="onClearHistory">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M3 6h18" />
            <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
            <path d="M10 11v6" />
            <path d="M14 11v6" />
          </svg>
        </button>
        <button class="header-btn" title="重命名" @click="onRename">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 20h9" />
            <path d="M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
          </svg>
        </button>
      </div>
    </header>

    <main class="chat-body">
      <template v-if="!hasActive">
        <div class="no-session">
          <p class="no-session-title">未选择会话</p>
          <p class="no-session-desc">从左侧选择或创建一个会话开始</p>
        </div>
      </template>

      <template v-else-if="loadError">
        <div class="load-error">
          <p class="load-error-icon">⚠</p>
          <p class="load-error-title">加载会话历史失败</p>
          <p class="load-error-desc">{{ loadError }}</p>
          <div class="load-error-actions">
            <button class="load-error-btn primary" @click="reloadCurrent">重试</button>
            <button class="load-error-btn" @click="dismissError">忽略</button>
          </div>
        </div>
      </template>

      <template v-else-if="!messagesReady || currentLoadedId !== store.activeId">
        <div class="chat-loading">
          <p>正在加载会话历史…</p>
        </div>
      </template>

      <!-- 工作目录判断：放在"会话详情已加载"之后（编辑器获得当前会话详情后再判定）。
           会话自身是否有 workdir 以详情/metadata 为准，不再仅凭列表卡片即时弹引导。 -->
      <template v-else-if="!store.activeWorkdir">
        <EmptyWorkdirState />
      </template>

      <template v-else>
        <ModelChatPanel
          :key="store.activeId ?? 'none'"
          :sessionId="store.activeId ?? ''"
        />
      </template>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import EmptyWorkdirState from './EmptyWorkdirState.vue'
import ModelChatPanel from '../ModelChatPanel.vue'

const store = useSessionsStore()

const hasActive = computed(() => !!store.activeId)

// 跟踪当前活跃会话的消息是否已经从后端拉取过。
// ModelChatPanel 只会在拿到"权威"历史后才挂载，避免：
// 1) UI 先用本地缓存的中间态（streaming/waiting）渲染，
// 2) 紧接着被后端的最终态（completed/failed）覆盖的闪烁。
const messagesReady = ref(false)
const currentLoadedId = ref<string | null>(null)
const loadError = ref<string | null>(null)

// 防止快切时的 stale guard：保存一个 sequence 编号，
// 每次 activeId 变化时递增；loadMessages 完成后比对，确认是当前 active 的响应
let loadSequence = 0

watch(
  () => store.activeId,
  async (id) => {
    const seq = ++loadSequence
    messagesReady.value = false
    currentLoadedId.value = null
    loadError.value = null
    if (id) {
      try {
        await store.loadMessages(id)
        // 检查是否被新的切换打断
        if (seq !== loadSequence) {
          logger.debug('ChatMainPanel', `loadMessages(${id}) was superseded by a later switch`)
          return
        }
        currentLoadedId.value = id
        messagesReady.value = true
      } catch (e) {
        if (seq !== loadSequence) return
        const msg = e instanceof Error ? e.message : String(e || '未知错误')
        loadError.value = msg
        // 仍然把 currentLoadedId 设为 id，避免 messagesReady 永久 false
        // 让用户可以"忽略"继续发送消息
        currentLoadedId.value = id
        messagesReady.value = true
        logger.error('ChatMainPanel', `loadMessages(${id}) failed`, e)
      }
    }
  },
  { immediate: true }
)

async function reloadCurrent() {
  if (!store.activeId) return
  loadError.value = null
  messagesReady.value = false
  try {
    await store.loadMessages(store.activeId)
    currentLoadedId.value = store.activeId
    messagesReady.value = true
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e || '未知错误')
    loadError.value = msg
    messagesReady.value = true
  }
}

function dismissError() {
  loadError.value = null
}

async function onRename() {
  if (!store.activeId) return
  const newTitle = prompt('新标题', store.activeTitle)
  if (!newTitle || newTitle.trim() === '') return
  await store.rename(store.activeId, newTitle.trim())
}

/** 清空当前会话的全部历史消息（保留会话本身）。破坏性操作，先确认。 */
async function onClearHistory() {
  if (!store.activeId) return
  if (!window.confirm('确定要清空当前会话的全部历史消息吗？此操作不可撤销。')) return
  try {
    await store.clearMessages(store.activeId)
  } catch (e) {
    logger.error('ChatMainPanel', '清空历史失败', e)
  }
}
</script>

<style scoped>
.chat-main-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  background: var(--color-surface);
  overflow: hidden;
}

.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
  min-height: 2.75rem;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-width: 0;
}

.session-name {
  font-size: 0.95rem;
  font-weight: 500;
  color: var(--color-text);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 16.25rem;
}

.status-working {
  font-size: 0.75rem;
  color: #22c55e;
}

.header-right {
  display: flex;
  gap: 0.25rem;
}

.header-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.15s;
}

.header-btn:hover {
  background: var(--surface-hover);
  color: var(--color-text);
}

.chat-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
}

.chat-body > * {
  flex: 1;
  min-height: 0;
}

.no-session {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  color: var(--color-text-muted);
  text-align: center;
  padding: 2rem;
}

.no-session-title {
  font-size: 1rem;
  margin-bottom: 0.4rem;
}

.no-session-desc {
  font-size: 0.85rem;
  opacity: 0.7;
}

.chat-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--color-text-muted);
  font-size: 0.9rem;
}

/* ═══════════════════════════════════════════════════════════
   Load error state
   ═══════════════════════════════════════════════════════════ */
.load-error {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  text-align: center;
  color: var(--color-text-muted);
  padding: 2rem;
  gap: 0.5rem;
}

.load-error-icon {
  font-size: 2.5rem;
  color: var(--color-banner-border);
  margin-bottom: 0.5rem;
}

.load-error-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.load-error-desc {
  font-size: 0.85rem;
  color: var(--color-text-muted);
  margin: 0;
  max-width: 30rem;
  word-break: break-word;
  font-family: 'Fira Code', 'Consolas', monospace;
  background: var(--color-msg-card);
  padding: 0.4rem 0.8rem;
  border-radius: 0.375rem;
}

.load-error-actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 1rem;
}

.load-error-btn {
  padding: 0.4rem 1rem;
  border-radius: 0.375rem;
  font-size: 0.85rem;
  border: 1px solid var(--color-border);
  background: var(--color-surface);
  color: var(--color-text);
  cursor: pointer;
  transition: all 0.15s ease;
}

.load-error-btn:hover {
  background: var(--surface-sunken);
}

.load-error-btn.primary {
  background: var(--color-primary);
  color: var(--text-on-accent);
  border-color: var(--color-primary);
}

.load-error-btn.primary:hover {
  opacity: 0.9;
}
</style>

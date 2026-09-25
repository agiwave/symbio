<template>
  <div class="chat-main-panel">
    <header v-if="hasActive" class="chat-header">
      <div class="header-left">
        <!-- 内联重命名：原地把标题变成输入框。不用浏览器原生 prompt——
             它阻塞主线程、样式与主题脱节、且无法做焦点管理。 -->
        <input
          v-if="renaming"
          ref="renameInputRef"
          v-model="draftTitle"
          class="rename-input"
          spellcheck="false"
          @keyup.enter="submitRename"
          @keyup.esc="cancelRename"
          @blur="cancelRename"
        />
        <template v-else>
          <h2 class="session-name">{{ store.activeTitle }}</h2>
          <span v-if="store.isActiveWorking" class="status-working">● AI 处理中</span>
        </template>
      </div>
      <div class="header-right">
        <!-- 动作区 = 会话自有动作（浏览内部）+ 机制动作（删除），
             去重合并由机制唯一实现（schemas/vdfs-form.mergeDetailActions） -->
        <VdfsActions
          v-if="merged.actions.length"
          :actions="merged.actions"
          :busy="merged.busy"
          @run="(a) => emit('action', a)"
        />
        <button class="header-btn" title="清空历史" @click="confirmClear.visible = true">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M3 6h18" />
            <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
            <path d="M10 11v6" />
            <path d="M14 11v6" />
          </svg>
        </button>
        <button class="header-btn" title="重命名" @click="startRename">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 20h9" />
            <path d="M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
          </svg>
        </button>
      </div>
    </header>

    <!-- 破坏性操作统一走自定义对话框（不用 window.confirm：同样是阻塞 + 主题脱节） -->
    <ConfirmDialog
      v-model:visible="confirmClear.visible"
      title="清空历史"
      message="确定要清空当前会话的全部历史消息吗？此操作不可撤销。"
      confirm-text="清空"
      icon="⚠"
      icon-kind="danger"
      danger
      :loading="confirmClear.busy"
      @confirm="onClearHistory"
    />

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
          :key="store.activeAddr || 'none'"
          :sessionId="store.activeId ?? ''"
          :sessionAddr="store.activeAddr"
        />
      </template>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import EmptyWorkdirState from './EmptyWorkdirState.vue'
import ModelChatPanel from '../ModelChatPanel.vue'
import VdfsActions from '@/components/vdfs/VdfsActions.vue'
import ConfirmDialog from '@/components/common/ConfirmDialog.vue'
import { useGenerationGuard } from '@/composables/useGenerationGuard'
import { mergeDetailActions, type DetailAction } from '@/schemas/vdfs'

const props = defineProps<{
  /** 会话自有动作（如「浏览内部」），由 Session ← VdfsSessionDetail 声明 */
  actions?: DetailAction[]
  /** 机制动作注入（页面单一定义点计算：删除等），与自身按钮并排渲染 */
  mechanismActions?: DetailAction[]
  /** 正在执行的机制动作 id（驱动其进行中文案） */
  mechanismBusy?: string | null
}>()

const emit = defineEmits<{
  /** 动作上抛（Session 分发到页面层机制通道：delete / open-container） */
  (e: 'action', action: DetailAction): void
}>()

/** 动作区 = 自有 + 机制（去重合并与忙态对齐由机制唯一实现） */
const merged = computed(() =>
  mergeDetailActions(
    props.actions ?? [],
    props.mechanismActions ?? [],
    [],
    [],
    props.mechanismBusy ?? null
  )
)

const store = useSessionsStore()

const hasActive = computed(() => !!store.activeId)

// 跟踪当前活跃会话的消息是否已经从后端拉取过。
// ModelChatPanel 只会在拿到"权威"历史后才挂载，避免：
// 1) UI 先用本地缓存的中间态（streaming/waiting）渲染，
// 2) 紧接着被后端的最终态（completed/failed）覆盖的闪烁。
const messagesReady = ref(false)
const currentLoadedId = ref<string | null>(null)
const loadError = ref<string | null>(null)

/**
 * 会话切换的**代次守卫**（机制唯一实现，见 `useGenerationGuard`）。
 *
 * 快速连切会话时，先发出的 `loadMessages` 可能后回来——它会用旧会话的消息
 * 覆盖新会话的界面。取号 → 响应回来比对 → 过期即丢。
 */
const loadGuard = useGenerationGuard()

watch(
  () => store.activeId,
  async (id) => {
    const rev = loadGuard.advance()
    messagesReady.value = false
    currentLoadedId.value = null
    loadError.value = null
    if (id) {
      try {
        await store.loadMessages(id)
        // 检查是否被新的切换打断
        if (loadGuard.revision() !== rev) {
          logger.debug('ChatMainPanel', `loadMessages(${id}) was superseded by a later switch`)
          return
        }
        currentLoadedId.value = id
        messagesReady.value = true
      } catch (e) {
        if (loadGuard.revision() !== rev) return
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

/**
 * 重试当前会话的历史加载。
 *
 * 同样过守卫：重试期间用户切走了会话，这次响应不得落地——原先直接读
 * `store.activeId`，会把**新会话**标成已加载，而装的其实是旧会话的消息。
 */
async function reloadCurrent() {
  const id = store.activeId
  if (!id) return
  const rev = loadGuard.advance()
  loadError.value = null
  messagesReady.value = false
  try {
    await store.loadMessages(id)
    if (loadGuard.revision() !== rev) return
    currentLoadedId.value = id
    messagesReady.value = true
  } catch (e) {
    if (loadGuard.revision() !== rev) return
    const msg = e instanceof Error ? e.message : String(e || '未知错误')
    loadError.value = msg
    messagesReady.value = true
  }
}

function dismissError() {
  loadError.value = null
}

/** 内联重命名（原地编辑标题；不用浏览器原生 prompt） */
const renaming = ref(false)
const draftTitle = ref('')
const renameInputRef = ref<HTMLInputElement | null>(null)

async function startRename() {
  if (!store.activeId) return
  draftTitle.value = store.activeTitle
  renaming.value = true
  await nextTick()
  renameInputRef.value?.select()
}

function cancelRename() {
  renaming.value = false
}

/** 提交重命名（空标题或与原名相同视为放弃——不做无意义的往返） */
async function submitRename() {
  const id = store.activeId
  if (!id || !renaming.value) return
  const title = draftTitle.value.trim()
  renaming.value = false
  if (!title || title === store.activeTitle) return
  await store.rename(id, title)
}

/** 清空历史的确认态（破坏性操作统一走 ConfirmDialog） */
const confirmClear = reactive({ visible: false, busy: false })

/** 清空当前会话的全部历史消息（保留会话本身） */
async function onClearHistory() {
  if (!store.activeId) return
  confirmClear.busy = true
  try {
    await store.clearMessages(store.activeId)
    confirmClear.visible = false
  } catch (e) {
    logger.error('ChatMainPanel', '清空历史失败', e)
  } finally {
    confirmClear.busy = false
  }
}
</script>

<style scoped>
.chat-main-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  background: var(--surface-panel);
  overflow: hidden;
}

.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border-default);
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
  color: var(--text-primary);
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

/* 内联重命名的输入框（替换标题位；宽度与 .session-name 一致，避免布局跳动） */
.rename-input {
  font-family: inherit;
  font-size: 0.95rem;
  font-weight: 500;
  color: var(--text-primary);
  background: var(--surface-sunken);
  border: 1px solid var(--accent);
  border-radius: 0.375rem;
  padding: 0.15rem 0.5rem;
  width: 16.25rem;
  max-width: 100%;
  outline: none;
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
  color: var(--text-secondary);
  transition: all 0.15s;
}

.header-btn:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
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
  color: var(--text-muted);
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
  color: var(--text-muted);
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
  color: var(--text-muted);
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
  color: var(--text-primary);
  margin: 0;
}

.load-error-desc {
  font-size: 0.85rem;
  color: var(--text-muted);
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
  border: 1px solid var(--border-default);
  background: var(--surface-panel);
  color: var(--text-primary);
  cursor: pointer;
  transition: all 0.15s ease;
}

.load-error-btn:hover {
  background: var(--surface-sunken);
}

.load-error-btn.primary {
  background: var(--accent);
  color: var(--text-on-accent);
  border-color: var(--accent);
}

.load-error-btn.primary:hover {
  opacity: 0.9;
}
</style>

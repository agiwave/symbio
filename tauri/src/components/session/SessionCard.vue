<!--
  SessionCard — 单个会话的"缩略窗口"卡片

  ## 设计目标

  - 在左侧列表中作为"该会话的实时缩略"展示
  - 不打开详细窗口也能看到：状态点（运行/思考/调用/等待/完成/失败）+ 最后一条消息预览
  - 点击 = 切换到详细窗口（ChatMainPanel）
  - 状态由 store.sessionStatuses[id] 驱动，bus 事件流触发自动刷新
-->
<template>
  <div
    class="session-card"
    :class="{
      active: isActive,
      working: liveStatus.is_working,
      waiting: liveStatus.is_waiting_approval,
      failed: liveStatus.last_failed && !liveStatus.is_working
    }"
    role="option"
    tabindex="0"
    :aria-selected="isActive"
    :aria-label="title"
    @click="onClick"
    @keydown.enter="onClick"
    @keydown.space.prevent="onClick"
  >
    <!-- 状态条 + 标题 -->
    <div class="card-header">
      <div class="status-area">
        <span v-if="liveStatus.is_working" class="status-dot running" :title="liveStatus.activity || '运行中'" />
        <span v-else-if="liveStatus.is_waiting_approval" class="status-dot waiting" :title="'等待审批'" />
        <span v-else-if="liveStatus.last_failed" class="status-dot failed" :title="'上次失败'" />
        <span v-else class="status-dot idle" :title="'空闲'" />
      </div>
      <div class="title-row">
        <span class="title-text" :title="title">
          {{ title }}
        </span>
        <span v-if="liveStatus.is_working" class="activity-text" :title="liveStatus.activity">
          {{ liveStatus.activity || '处理中…' }}
        </span>
        <span v-else-if="liveStatus.is_waiting_approval" class="activity-text">
          等待审批
        </span>
        <span v-else-if="liveStatus.last_failed" class="activity-text failed">
          上次失败
        </span>
      </div>
    </div>

    <!-- 预览行：实时显示最后一条 assistant 消息 -->
    <div class="card-preview" v-if="preview">
      <span class="preview-text">{{ preview }}</span>
    </div>

    <!-- 元信息行：workdir · 时间 · 消息数 -->
    <div class="card-meta">
      <span
        v-if="heartbeatEnabled"
        class="heartbeat-badge"
        :title="heartbeatTitle"
      >♥ 心跳</span>
      <span class="workdir" v-if="session.metadata?.workdir" :title="session.metadata.workdir">
        {{ basename(session.metadata.workdir) }}
      </span>
      <span v-if="session.metadata?.workdir" class="dot">·</span>
      <span class="time">{{ formatTime(session.updated_at) }}</span>
      <span class="dot">·</span>
      <span class="count">{{ session.message_count }} 条</span>
    </div>

    <button
      class="delete-btn"
      :class="{ disabled: session.is_working }"
      :title="session.is_working ? '请先停止会话' : '删除会话'"
      @click.stop="onDelete"
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="3 6 5 6 21 6" />
        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      </svg>
    </button>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useSessionsStore, type SessionLiveStatus } from '@/stores/sessions'
import type { SessionListItem } from '@/services/session'

const props = defineProps<{
  session: SessionListItem
  isActive: boolean
}>()

const emit = defineEmits<{
  click: []
  delete: []
}>()

const store = useSessionsStore()

// 实时状态：来自 store.sessionStatuses[id]，每次 bus 事件都会触发更新
const liveStatus = computed<SessionLiveStatus>(() =>
  store.getSessionStatus(props.session.id)
)

const title = computed(() => {
  return (
    store.titles[props.session.id] ||
    props.session.metadata?.title ||
    (props.session.message_count === 0
      ? '新对话'
      : props.session.metadata?.workdir
        ? basename(props.session.metadata.workdir)
        : '对话')
  )
})

const heartbeatEnabled = computed(() => !!props.session.metadata?.heartbeat?.enabled)

const heartbeatTitle = computed(() => {
  const hb = props.session.metadata?.heartbeat
  if (!hb) return ''
  const interval = Number(hb.interval_seconds) > 0 ? Number(hb.interval_seconds) : 300
  const history = hb.include_history === false ? '（不带历史）' : '（带历史）'
  return `心跳任务已开启：空闲 ${interval} 秒后自动触发${history}`
})

const preview = computed(() => {
  // 优先：live last_preview（流式 + 持久化都有）
  if (liveStatus.value.last_preview) return liveStatus.value.last_preview
  // 退化：当前 store 中最后一条 assistant 消息
  const msgs = store.getSessionMessages(props.session.id)
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i]
    if (m.role === 'assistant') {
      if (typeof m.content === 'string') return m.content.slice(0, 60)
      if (Array.isArray(m.content)) {
        const txt = (m.content as any[]).filter(p => p?.type === 'text').map(p => p?.text || '').join('')
        return txt.slice(0, 60)
      }
    }
  }
  return ''
})

function basename(p: string): string {
  if (!p) return ''
  return p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
}

function formatTime(ts: number): string {
  if (!ts) return ''
  const d = new Date(ts * 1000)
  const now = new Date()
  const diff = (now.getTime() - d.getTime()) / 1000
  if (diff < 60) return '刚刚'
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`
  if (diff < 86400 * 7) return `${Math.floor(diff / 86400)} 天前`
  return d.toLocaleDateString()
}

function onClick() { emit('click') }
function onDelete() { emit('delete') }
</script>

<style scoped>
.session-card {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.6rem 0.75rem 0.55rem 0.9rem;
  cursor: pointer;
  border-left: 0.125rem solid transparent;
  transition: background 0.12s;
  min-height: 3.5rem;
}

.session-card:hover {
  background: var(--surface-hover);
}

.session-card.active {
  background: rgba(102, 126, 234, 0.1);
  border-left-color: var(--color-primary);
}

.session-card.working {
  background: linear-gradient(to right, rgba(34, 197, 94, 0.06), transparent 40%);
}

.session-card.waiting {
  background: linear-gradient(to right, rgba(245, 158, 11, 0.08), transparent 40%);
}

.session-card.failed {
  background: linear-gradient(to right, rgba(239, 68, 68, 0.06), transparent 40%);
}

.card-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}

.status-area {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

.status-dot {
  display: inline-block;
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  flex-shrink: 0;
}

.status-dot.running {
  background: var(--success-solid);
  animation: pulse 1.4s ease-in-out infinite;
}

.status-dot.waiting {
  background: var(--warning-solid);
  animation: pulse-warn 1.6s ease-in-out infinite;
}

.status-dot.failed {
  background: var(--danger-solid);
}

.status-dot.idle {
  background: var(--text-disabled);
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

@keyframes pulse-warn {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.title-row {
  display: flex;
  align-items: baseline;
  gap: 0.4rem;
  flex: 1;
  min-width: 0;
  overflow: hidden;
}

.title-text {
  font-size: 0.85rem;
  color: var(--color-text);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.activity-text {
  font-size: 0.7rem;
  color: #22c55e;
  font-style: italic;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 0 1 auto;
  min-width: 0;
}

.activity-text.failed {
  color: #ef4444;
}

.card-preview {
  display: flex;
  align-items: center;
  min-width: 0;
}

.preview-text {
  font-size: 0.72rem;
  color: var(--color-text-muted);
  line-height: 1.35;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.card-meta {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  font-size: 0.68rem;
  color: var(--color-text-muted);
  overflow: hidden;
}

.card-meta .workdir {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 6.25rem;
}

.card-meta .dot {
  opacity: 0.4;
}

.heartbeat-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.1rem;
  font-size: 0.66rem;
  color: #ef4444;
  background: rgba(239, 68, 68, 0.1);
  border-radius: 0.25rem;
  padding: 0 0.3rem;
  flex-shrink: 0;
}

.delete-btn {
  position: absolute;
  top: 0.4rem;
  right: 0.4rem;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.375rem;
  height: 1.375rem;
  border: none;
  background: transparent;
  border-radius: 0.25rem;
  cursor: pointer;
  color: var(--color-text-muted);
  opacity: 0;
  transition: all 0.15s;
  z-index: 1;
}

.session-card:hover .delete-btn {
  opacity: 0.7;
}

.delete-btn:hover {
  opacity: 1 !important;
  background: rgba(239, 68, 68, 0.1);
  color: #ef4444;
}

.delete-btn.disabled {
  opacity: 0.2 !important;
  cursor: not-allowed;
}
</style>

<!--
  SessionSettingsDialog — 单会话设置弹窗

  ## 功能
  - 配置该会话的「心跳任务」：
    - 启用开关 (enabled)
    - 空闲间隔（秒）：会话空闲这么久后自动触发 (interval_seconds)
    - 任务提示词 (prompt)
    - 是否携带历史会话信息 (include_history)
  - 「立即执行一次」：手动触发一次心跳（走 worker/session/heartbeat/trigger）

  配置存储于 `Session.metadata.heartbeat`，由后端 SessionPlugin 后台调度器消费。
-->
<template>
  <Teleport to="body">
    <div class="hb-overlay" @click.self="onClose">
      <div class="hb-dialog" :class="{ 'is-working': liveWorking }">
        <header class="hb-header">
          <span class="hb-title">会话设置</span>
          <span class="hb-subtitle">{{ sessionTitle }}</span>
          <button class="hb-close" title="关闭" @click="onClose">×</button>
        </header>

        <div class="hb-body">
          <!-- 心跳任务分区 -->
          <section class="hb-section">
            <div class="hb-section-head">
              <div class="hb-section-title">
                <span class="hb-heart-icon" :class="{ on: form.enabled }">♥</span>
                心跳任务
              </div>
              <label class="hb-switch">
                <input type="checkbox" v-model="form.enabled" />
                <span class="hb-slider"></span>
              </label>
            </div>

            <p class="hb-hint">
              开启后，会话空闲达到设定间隔会自动以「任务提示词」触发一次对话。
              正在工作中的会话不会触发。
            </p>

            <div class="hb-fields" :class="{ disabled: !form.enabled }">
              <div class="hb-field">
                <label class="hb-label">
                  空闲间隔（秒）
                  <span class="hb-sub">会话无活动多久后自动触发</span>
                </label>
                <input
                  class="hb-input"
                  type="number"
                  min="10"
                  step="10"
                  v-model.number="form.interval_seconds"
                  :disabled="!form.enabled"
                />
              </div>

              <div class="hb-field">
                <label class="hb-label">
                  任务提示词
                  <span class="hb-sub">每次心跳自动发送给 AI 的内容</span>
                </label>
                <textarea
                  class="hb-textarea"
                  rows="4"
                  placeholder="例如：检查当前工作目录的待办，主动推进一项不依赖用户输入的小任务。"
                  v-model="form.prompt"
                  :disabled="!form.enabled"
                ></textarea>
              </div>

              <label class="hb-check">
                <input type="checkbox" v-model="form.include_history" :disabled="!form.enabled" />
                <span>
                  携带历史会话信息
                  <span class="hb-sub">关闭后，心跳触发时不带历史上下文（以全新上下文执行）。</span>
                </span>
              </label>
            </div>
          </section>

          <!-- 触发状态提示 -->
          <div v-if="triggerMsg" class="hb-trigger-msg" :class="triggerType">
            {{ triggerMsg }}
          </div>
        </div>

        <footer class="hb-footer">
          <button
            class="hb-btn ghost"
            :disabled="triggering || !canTrigger"
            :title="canTrigger ? '立即以当前提示词触发一次' : triggerDisabledReason"
            @click="onTrigger"
          >
            <span v-if="triggering" class="hb-spinner"></span>
            立即执行一次
          </button>
          <div class="hb-footer-right">
            <button class="hb-btn ghost" @click="onClose">取消</button>
            <button class="hb-btn primary" :disabled="saving" @click="onSave">
              {{ saving ? '保存中…' : '保存' }}
            </button>
          </div>
        </footer>
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import type { SessionListItem } from '@/services/session'
import type { SessionHeartbeatConfig } from '@/services/session'
import { triggerHeartbeat } from '@/services/session'
import { logger } from '@/utils/logger'

const props = defineProps<{
  session: SessionListItem
}>()

const emit = defineEmits<{
  close: []
}>()

const store = useSessionsStore()

const DEFAULT_INTERVAL = 300

function defaults(): SessionHeartbeatConfig {
  return { enabled: false, interval_seconds: DEFAULT_INTERVAL, prompt: '', include_history: true }
}

function fromMetadata(m: SessionListItem['metadata']): SessionHeartbeatConfig {
  const hb = m?.heartbeat
  if (!hb) return defaults()
  return {
    enabled: !!hb.enabled,
    interval_seconds: Number(hb.interval_seconds) > 0 ? Number(hb.interval_seconds) : DEFAULT_INTERVAL,
    prompt: typeof hb.prompt === 'string' ? hb.prompt : '',
    include_history: hb.include_history !== false
  }
}

const form = reactive<SessionHeartbeatConfig>(defaults())

// 触发状态提示（须在下方 watch immediate 之前声明，避免 TDZ 报错）
const triggerMsg = ref('')
const triggerType = ref<'info' | 'ok' | 'warn' | 'err'>('info')

// 每次打开（session 变化）时同步表单
watch(
  () => props.session,
  (s) => {
    Object.assign(form, fromMetadata(s.metadata))
    triggerMsg.value = ''
  },
  { immediate: true, deep: false }
)

const sessionTitle = computed(
  () => store.titles[props.session.id] || props.session.metadata?.title || '会话'
)

const liveWorking = computed(() => store.getSessionStatus(props.session.id).is_working)

const saving = ref(false)
const triggering = ref(false)

const canTrigger = computed(() => form.enabled && form.prompt.trim().length > 0)
const triggerDisabledReason = computed(() => {
  if (!form.enabled) return '需先启用心跳任务'
  if (!form.prompt.trim()) return '提示词不能为空'
  return ''
})

function onClose() {
  emit('close')
}

async function onSave() {
  saving.value = true
  try {
    const payload: SessionHeartbeatConfig = {
      enabled: form.enabled,
      interval_seconds: Number(form.interval_seconds) > 0 ? Number(form.interval_seconds) : DEFAULT_INTERVAL,
      prompt: form.prompt,
      include_history: form.include_history
    }
    await store.setHeartbeat(props.session.id, payload)
    emit('close')
  } catch (e) {
    triggerType.value = 'err'
    triggerMsg.value = '保存失败：' + (e instanceof Error ? e.message : String(e))
  } finally {
    saving.value = false
  }
}

async function onTrigger() {
  if (!canTrigger.value) return
  triggering.value = true
  triggerMsg.value = ''
  try {
    const res = await triggerHeartbeat(props.session.id)
    if (res.status === 'triggered') {
      triggerType.value = 'ok'
      triggerMsg.value = res.include_history
        ? '已触发心跳（携带历史上下文）'
        : '已触发心跳（不带历史上下文）'
    } else if (res.status === 'skipped') {
      triggerType.value = 'warn'
      triggerMsg.value = res.reason || '会话正在工作中，已跳过'
    }
  } catch (e) {
    triggerType.value = 'err'
    triggerMsg.value = '触发失败：' + (e instanceof Error ? e.message : String(e))
    logger.error('SessionSettingsDialog', 'triggerHeartbeat 失败', e)
  } finally {
    triggering.value = false
  }
}
</script>

<style scoped>
.hb-overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--z-dialog);
}

.hb-dialog {
  width: 100%;
  max-width: 28.75rem;
  background: var(--surface-overlay);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  display: flex;
  flex-direction: column;
  max-height: 88vh;
  overflow: hidden;
}

/* 后台任务运行中的状态强调（hb-dialog + is-working 组合） */
.hb-dialog.is-working {
  border: 1px solid var(--accent-subtle-border);
}

.hb-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
}

.hb-title {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}

.hb-subtitle {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.hb-close {
  border: none;
  background: transparent;
  font-size: 1.25rem;
  line-height: 1;
  cursor: pointer;
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  width: 1.75rem;
  height: 1.75rem;
}
.hb-close:hover { background: var(--surface-hover); color: var(--text-primary); }

.hb-body {
  padding: var(--space-4);
  overflow-y: auto;
  flex: 1;
  min-height: 0;
}

.hb-section {
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  padding: var(--space-3) var(--space-4);
}

.hb-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.hb-section-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}

.hb-heart-icon {
  color: var(--text-disabled);
  font-size: 1rem;
  transition: color var(--motion-base) var(--motion-ease);
}
.hb-heart-icon.on { color: var(--danger-solid); }

.hb-hint {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  line-height: var(--line-height-normal);
  margin: var(--space-2) 0 var(--space-3);
}

/* 开关 */
.hb-switch {
  position: relative;
  display: inline-block;
  width: 2.5rem;
  height: 1.375rem;
  flex-shrink: 0;
}
.hb-switch input { opacity: 0; width: 0; height: 0; }
.hb-slider {
  position: absolute;
  inset: 0;
  background: var(--border-strong);
  border-radius: var(--radius-full);
  transition: var(--motion-base) var(--motion-ease);
  cursor: pointer;
}
.hb-slider::before {
  content: '';
  position: absolute;
  width: 1rem;
  height: 1rem;
  left: 0.1875rem;
  top: 0.1875rem;
  background: var(--surface-overlay);
  border-radius: var(--radius-full);
  transition: var(--motion-base) var(--motion-ease);
}
.hb-switch input:checked + .hb-slider { background: var(--accent); }
.hb-switch input:checked + .hb-slider::before { transform: translateX(1.125rem); }
.hb-switch input:focus-visible + .hb-slider {
  outline: var(--focus-ring-width) solid var(--focus-ring-color);
  outline-offset: var(--focus-ring-offset);
}

.hb-fields { display: flex; flex-direction: column; gap: var(--space-3); margin-top: var(--space-2); }
.hb-fields.disabled { opacity: 0.5; pointer-events: none; }

.hb-field { display: flex; flex-direction: column; gap: var(--space-1); }
.hb-label {
  font-size: var(--font-size-sm);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}
.hb-sub { font-size: 0.68rem; font-weight: var(--font-weight-regular); color: var(--text-muted); }

.hb-input,
.hb-textarea {
  width: 100%;
  box-sizing: border-box;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: var(--font-size-sm);
  font-family: inherit;
  color: var(--text-primary);
  background: var(--surface-overlay);
  resize: vertical;
}
.hb-input:focus,
.hb-textarea:focus {
  outline: none;
  border-color: var(--accent);
}

.hb-check {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  cursor: pointer;
}
.hb-check input { margin-top: 0.15rem; flex-shrink: 0; }

.hb-trigger-msg {
  margin-top: var(--space-3);
  font-size: 0.78rem;
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  line-height: var(--line-height-normal);
}
.hb-trigger-msg.ok { background: var(--success-bg); color: var(--success-fg); }
.hb-trigger-msg.warn { background: var(--warning-bg); color: var(--warning-fg); }
.hb-trigger-msg.err { background: var(--danger-bg); color: var(--danger-fg); }
.hb-trigger-msg.info { background: var(--accent-subtle-bg); color: var(--accent); }

.hb-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--border-default);
  flex-shrink: 0;
}

.hb-footer-right { display: flex; gap: var(--space-2); }

.hb-btn {
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-md);
  font-size: var(--font-size-sm);
  cursor: pointer;
  border: 1px solid transparent;
  transition: background-color var(--motion-fast) var(--motion-ease),
    opacity var(--motion-fast) var(--motion-ease);
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
}
.hb-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.hb-btn.primary { background: var(--accent); color: var(--text-on-accent); }
.hb-btn.primary:hover:not(:disabled) { background: var(--accent-hover); }
.hb-btn.ghost { background: transparent; border-color: var(--border-default); color: var(--text-primary); }
.hb-btn.ghost:hover:not(:disabled) { background: var(--surface-hover); }

.hb-spinner {
  width: 0.75rem;
  height: 0.75rem;
  border: 0.125rem solid var(--accent-subtle-border);
  border-top-color: var(--text-on-accent);
  border-radius: 50%;
  animation: hb-spin 0.7s linear infinite;
}
@keyframes hb-spin { to { transform: rotate(360deg); } }
</style>

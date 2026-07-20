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
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.hb-dialog {
  width: 100%;
  max-width: 460px;
  background: var(--color-surface, #fff);
  border-radius: 14px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  max-height: 88vh;
  overflow: hidden;
}

.hb-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.85rem 1rem;
  border-bottom: 1px solid var(--color-border, #eee);
  flex-shrink: 0;
}

.hb-title {
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--color-text, #1f2937);
}

.hb-subtitle {
  font-size: 0.72rem;
  color: var(--color-text-muted, #9ca3af);
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
  color: var(--color-text-muted, #9ca3af);
  border-radius: 6px;
  width: 28px;
  height: 28px;
}
.hb-close:hover { background: rgba(0, 0, 0, 0.06); color: var(--color-text, #1f2937); }

.hb-body {
  padding: 1rem;
  overflow-y: auto;
  flex: 1;
  min-height: 0;
}

.hb-section {
  border: 1px solid var(--color-border, #eee);
  border-radius: 10px;
  padding: 0.9rem 1rem;
}

.hb-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.hb-section-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.9rem;
  font-weight: 600;
  color: var(--color-text, #1f2937);
}

.hb-heart-icon {
  color: #cbd5e1;
  font-size: 1rem;
  transition: color 0.2s;
}
.hb-heart-icon.on { color: #ef4444; }

.hb-hint {
  font-size: 0.75rem;
  color: var(--color-text-muted, #9ca3af);
  line-height: 1.5;
  margin: 0.5rem 0 0.75rem;
}

/* 开关 */
.hb-switch {
  position: relative;
  display: inline-block;
  width: 40px;
  height: 22px;
  flex-shrink: 0;
}
.hb-switch input { opacity: 0; width: 0; height: 0; }
.hb-slider {
  position: absolute;
  inset: 0;
  background: #d1d5db;
  border-radius: 999px;
  transition: 0.2s;
  cursor: pointer;
}
.hb-slider::before {
  content: '';
  position: absolute;
  width: 16px;
  height: 16px;
  left: 3px;
  top: 3px;
  background: #fff;
  border-radius: 50%;
  transition: 0.2s;
}
.hb-switch input:checked + .hb-slider { background: var(--color-primary, #667eea); }
.hb-switch input:checked + .hb-slider::before { transform: translateX(18px); }

.hb-fields { display: flex; flex-direction: column; gap: 0.85rem; margin-top: 0.5rem; }
.hb-fields.disabled { opacity: 0.5; pointer-events: none; }

.hb-field { display: flex; flex-direction: column; gap: 0.35rem; }
.hb-label {
  font-size: 0.8rem;
  font-weight: 500;
  color: var(--color-text, #1f2937);
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}
.hb-sub { font-size: 0.68rem; font-weight: 400; color: var(--color-text-muted, #9ca3af); }

.hb-input,
.hb-textarea {
  width: 100%;
  box-sizing: border-box;
  padding: 0.5rem 0.7rem;
  border: 1px solid var(--color-border, #e5e7eb);
  border-radius: 8px;
  font-size: 0.85rem;
  font-family: inherit;
  color: var(--color-text, #1f2937);
  background: var(--color-surface, #fff);
  resize: vertical;
}
.hb-input:focus,
.hb-textarea:focus {
  outline: none;
  border-color: var(--color-primary, #667eea);
}

.hb-check {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  font-size: 0.8rem;
  color: var(--color-text, #1f2937);
  cursor: pointer;
}
.hb-check input { margin-top: 0.15rem; flex-shrink: 0; }

.hb-trigger-msg {
  margin-top: 0.85rem;
  font-size: 0.78rem;
  padding: 0.5rem 0.7rem;
  border-radius: 8px;
  line-height: 1.4;
}
.hb-trigger-msg.ok { background: rgba(34, 197, 94, 0.12); color: #16a34a; }
.hb-trigger-msg.warn { background: rgba(245, 158, 11, 0.12); color: #d97706; }
.hb-trigger-msg.err { background: rgba(239, 68, 68, 0.12); color: #dc2626; }
.hb-trigger-msg.info { background: rgba(102, 126, 234, 0.1); color: #4f46e5; }

.hb-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  padding: 0.75rem 1rem;
  border-top: 1px solid var(--color-border, #eee);
  flex-shrink: 0;
}

.hb-footer-right { display: flex; gap: 0.5rem; }

.hb-btn {
  padding: 0.45rem 1rem;
  border-radius: 8px;
  font-size: 0.82rem;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.15s;
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
}
.hb-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.hb-btn.primary { background: var(--color-primary, #667eea); color: #fff; }
.hb-btn.primary:hover:not(:disabled) { opacity: 0.9; }
.hb-btn.ghost { background: transparent; border-color: var(--color-border, #e5e7eb); color: var(--color-text, #1f2937); }
.hb-btn.ghost:hover:not(:disabled) { background: rgba(0, 0, 0, 0.04); }

.hb-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid rgba(255, 255, 255, 0.4);
  border-top-color: #fff;
  border-radius: 50%;
  animation: hb-spin 0.7s linear infinite;
}
@keyframes hb-spin { to { transform: rotate(360deg); } }
</style>

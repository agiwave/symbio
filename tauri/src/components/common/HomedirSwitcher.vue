<!--
  HomedirSwitcher.vue

  系统目录 (homedir) 切换对话框
  - 显示当前 homedir 绝对路径与 bootstrap 文件位置
  - 用户可输入新路径（绝对路径或 `~` 前缀），或点"浏览"打开系统目录选择器
  - 点"切换"前弹出确认对话框（提示活跃 chat 会话将被关闭）
  - 调用 `home/reload` 路由触发后端热重载

  对应后端: symbio/src/plugins/home/plugin.rs::route "home/reload"
  对应服务: tauri/src/services/home.ts (switchHomedir / getHomedirInfo)
  对应 schema: tauri/src/schemas/home_reload.ts
-->
<template>
  <div v-if="open" class="modal-mask" @click.self="handleClose">
    <div class="modal" role="dialog" aria-labelledby="homedir-title">
      <header class="modal-header">
        <h3 id="homedir-title">系统目录 (homedir)</h3>
        <button class="close-btn" @click="handleClose" aria-label="关闭">×</button>
      </header>

      <div class="modal-body">
        <div class="row">
          <label class="label">当前 homedir</label>
          <div class="value mono">{{ info.homedir || '（加载中…）' }}</div>
        </div>

        <div class="row">
          <label class="label" for="homedir-input">切换到</label>
          <div class="input-row">
            <input
              id="homedir-input"
              v-model="newHomedir"
              class="text-input mono"
              type="text"
              placeholder="例如 ~/.symbio 或 /path/to/your/homedir"
              @keydown.enter="handleSubmit"
            />
            <button class="btn" @click="handleBrowse" :disabled="browsing">
              浏览…
            </button>
          </div>
          <div class="hint">
            切换后，Symbio 会从新 homedir 重新加载所有子插件（Agent / LLM / MCP / Skill / Session）。
            <br />
            bootstrap 写入位置: <span class="mono">{{ info.bootstrap_path || '—' }}</span>
          </div>
        </div>

        <div v-if="error" class="error">{{ error }}</div>
      </div>

      <footer class="modal-footer">
        <button class="btn" @click="handleClose" :disabled="busy">取消</button>
        <button
          class="btn btn-primary"
          @click="handleSubmit"
          :disabled="busy || !newHomedir.trim()"
        >
          {{ busy ? '切换中…' : '切换' }}
        </button>
      </footer>
    </div>

    <!-- 二次确认：提示活跃 chat 会话将被关闭 -->
    <ConfirmDialog
      :visible="confirmOpen"
      title="确认切换系统目录？"
      :message="confirmMessage"
      confirm-text="切换"
      cancel-text="取消"
      :loading="busy"
      @confirm="handleConfirm"
      @cancel="confirmOpen = false"
    />
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { getHomedirInfo, switchHomedir } from '@/services/home'
import { logger } from '@/utils/logger'
import ConfirmDialog from './ConfirmDialog.vue'
import type { Response as ReloadResponse } from '@/schemas/home_reload'

interface Props {
  open: boolean
}
const props = defineProps<Props>()
const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'reloaded', response: ReloadResponse): void
}>()

const newHomedir = ref('')
const info = ref<{ homedir: string; bootstrap_path: string }>({
  homedir: '',
  bootstrap_path: ''
})
const error = ref('')
const busy = ref(false)
const browsing = ref(false)
const confirmOpen = ref(false)

const confirmMessage = ref('')

async function refreshInfo() {
  error.value = ''
  const resp = await getHomedirInfo()
  info.value = resp
  if (!newHomedir.value) {
    newHomedir.value = resp.homedir
  }
}

watch(
  () => props.open,
  async (v) => {
    if (v) {
      await refreshInfo()
    }
  }
)

onMounted(async () => {
  if (props.open) {
    await refreshInfo()
  }
})

function handleClose() {
  if (busy.value) return
  emit('update:open', false)
  error.value = ''
  newHomedir.value = ''
}

async function handleBrowse() {
  browsing.value = true
  try {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: '选择系统目录 (homedir)'
    })
    if (typeof selected === 'string' && selected.length > 0) {
      newHomedir.value = selected
    }
  } catch (err) {
    logger.error('homedir-switcher', 'Browse failed:', err)
  } finally {
    browsing.value = false
  }
}

function handleSubmit() {
  if (busy.value) return
  const trimmed = newHomedir.value.trim()
  if (!trimmed) {
    error.value = '请输入目标 homedir 路径'
    return
  }
  if (trimmed === info.value.homedir) {
    error.value = '与当前 homedir 相同，无需切换'
    return
  }
  error.value = ''
  confirmMessage.value =
    `即将切换 homedir：\n\n` +
    `旧: ${info.value.homedir}\n` +
    `新: ${trimmed}\n\n` +
    `所有活跃 chat 会话将被关闭，UI 数据将被刷新。`
  confirmOpen.value = true
}

async function handleConfirm() {
  confirmOpen.value = false
  busy.value = true
  error.value = ''
  try {
    const resp = await switchHomedir(newHomedir.value.trim())
    if (!resp) {
      error.value = '切换失败：未收到后端响应'
      return
    }
    logger.info('homedir-switcher', '切换成功:', resp)
    emit('reloaded', resp)
    emit('update:open', false)
    newHomedir.value = ''
  } catch (err: any) {
    logger.error('homedir-switcher', 'switchHomedir failed:', err)
    error.value = `切换失败: ${err?.message ?? String(err)}`
  } finally {
    busy.value = false
  }
}
</script>

<style scoped>
.modal-mask {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--z-overlay);
}
.modal {
  width: min(35rem, 90vw);
  background: var(--surface-overlay);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  display: flex;
  flex-direction: column;
}
.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border-default);
}
.modal-header h3 {
  margin: 0;
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-semibold);
}
.close-btn {
  background: transparent;
  border: none;
  font-size: 1.4rem;
  line-height: 1;
  cursor: pointer;
  color: var(--text-secondary);
  padding: 0 var(--space-2);
  border-radius: var(--radius-sm);
}
.close-btn:hover { color: var(--text-primary); }
.modal-body {
  padding: var(--space-4) var(--space-5);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.row {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: var(--font-weight-medium);
}
.value {
  font-size: var(--font-size-base);
  padding: var(--space-2) var(--space-3);
  background: var(--surface-sunken);
  border-radius: var(--radius-md);
  word-break: break-all;
}
.mono { font-family: var(--font-mono); }
.input-row {
  display: flex;
  gap: var(--space-2);
}
.text-input {
  flex: 1;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  background: var(--surface-overlay);
  color: var(--text-primary);
  outline: none;
  transition: border-color var(--motion-fast) var(--motion-ease);
}
.text-input:focus { border-color: var(--accent); }
.hint {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  line-height: var(--line-height-normal);
}
.error {
  color: var(--danger-fg);
  font-size: var(--font-size-base);
  padding: var(--space-2) var(--space-3);
  background: var(--danger-bg);
  border-radius: var(--radius-md);
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-5);
  border-top: 1px solid var(--border-default);
}
.btn {
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  cursor: pointer;
  font-size: var(--font-size-base);
  color: var(--text-primary);
  transition: background-color var(--motion-fast) var(--motion-ease);
}
.btn:hover:not(:disabled) {
  background: var(--surface-hover);
}
.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.btn-primary {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}
.btn-primary:hover:not(:disabled) {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
}
</style>

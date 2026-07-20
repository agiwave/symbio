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
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.modal {
  width: min(560px, 90vw);
  background: var(--color-surface);
  border-radius: 12px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
}
.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid var(--color-border);
}
.modal-header h3 {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
}
.close-btn {
  background: transparent;
  border: none;
  font-size: 1.4rem;
  line-height: 1;
  cursor: pointer;
  color: var(--color-text-secondary);
  padding: 0 0.4rem;
}
.close-btn:hover { color: var(--color-text-primary); }
.modal-body {
  padding: 1rem 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.row {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}
.label {
  font-size: 0.85rem;
  color: var(--color-text-secondary);
  font-weight: 500;
}
.value {
  font-size: 0.9rem;
  padding: 0.4rem 0.6rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 6px;
  word-break: break-all;
}
.mono { font-family: 'Menlo', 'Consolas', monospace; }
.input-row {
  display: flex;
  gap: 0.5rem;
}
.text-input {
  flex: 1;
  padding: 0.5rem 0.6rem;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  font-size: 0.9rem;
  background: var(--color-bg, #fff);
  color: var(--color-text-primary);
  outline: none;
  transition: border-color 0.15s;
}
.text-input:focus { border-color: var(--color-primary); }
.hint {
  font-size: 0.75rem;
  color: var(--color-text-secondary);
  line-height: 1.5;
}
.error {
  color: #d33;
  font-size: 0.85rem;
  padding: 0.5rem 0.6rem;
  background: rgba(221, 51, 51, 0.08);
  border-radius: 6px;
}
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  padding: 0.75rem 1.25rem;
  border-top: 1px solid var(--color-border);
}
.btn {
  padding: 0.4rem 0.9rem;
  border: 1px solid var(--color-border);
  border-radius: 6px;
  background: transparent;
  cursor: pointer;
  font-size: 0.85rem;
  color: var(--color-text-primary);
  transition: all 0.15s;
}
.btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.04);
}
.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.btn-primary {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: #fff;
}
.btn-primary:hover:not(:disabled) {
  background: var(--color-primary-dark);
  border-color: var(--color-primary-dark);
}
</style>

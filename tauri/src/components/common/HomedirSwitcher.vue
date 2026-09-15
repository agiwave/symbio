<!--
  HomedirSwitcher.vue  —  系统目录 (System Location) 切换对话框

  统一「本地目录 / 远端地址」两类系统目录：
  - 本地目录：连本机后端（native），可切换本地 homedir 路径（沿用 home/reload）
  - 远端地址：连远端 Symbio 网关（HTTP/WS），地址字符串可内嵌 key（?token=）

  关键设计：所有控制面调用（home/reload / gateway/config/*）均走 forceNative，
  永远命中本机后端，因此即便当前已指向远端、或远端不可达，也能随时切回 / 修正，
  不会出现「切到坏地址后主页面卡死且无法恢复」的旧问题。

  对应后端: symbio/src/plugins/home/plugin.rs::route "home/reload"
            symbio/src/plugins/gateway/plugin.rs（入站配置 = `.vdfs/gateway/PLUGIN.yml`）
  对应服务: tauri/src/services/home.ts (switchHomedir / getHomedirInfo)
            tauri/src/services/systemLocation.ts (系统目录模型与持久化)
  对应 schema: tauri/src/schemas/home_reload.ts
-->
<template>
  <div v-if="open" class="modal-mask" @click.self="handleClose">
    <div class="modal" role="dialog" aria-labelledby="sysdir-title">
      <header class="modal-header">
        <h3 id="sysdir-title">系统目录</h3>
        <button class="close-btn" @click="handleClose" aria-label="关闭">×</button>
      </header>

      <div class="modal-body">
        <!-- 模式切换 -->
        <div class="seg">
          <button
            class="seg-btn"
            :class="{ active: mode === 'local' }"
            @click="mode = 'local'"
          >本地目录</button>
          <button
            class="seg-btn"
            :class="{ active: mode === 'remote' }"
            @click="mode = 'remote'"
          >远端地址</button>
        </div>

        <!-- 本地目录 -->
        <template v-if="mode === 'local'">
          <div class="row">
            <label class="label" for="sysdir-input">本地 homedir 路径</label>
            <div class="input-row">
              <input
                id="sysdir-input"
                v-model="localPath"
                class="text-input mono"
                type="text"
                placeholder="例如 ~/.symbio 或 /path/to/your/homedir"
                @keydown.enter="handleSubmit"
              />
              <button class="btn" @click="handleBrowse" :disabled="browsing">浏览…</button>
            </div>
            <div class="hint">
              切换后，Symbio 会从新 homedir 重新加载所有子插件（Agent / LLM / MCP / Skill / Session）。
              <br />
              bootstrap 写入位置: <span class="mono">{{ bootstrapPath || '—' }}</span>
            </div>
          </div>
        </template>

        <!-- 远端地址 -->
        <template v-else>
          <div class="row">
            <label class="label" for="sysdir-url">远端网关地址（可内含 key）</label>
            <input
              id="sysdir-url"
              v-model="remoteUrl"
              class="text-input mono"
              type="text"
              placeholder="http://host:port 或 http://host:port?token=KEY"
              @keydown.enter="handleSubmit"
            />
            <div class="hint">
              直接粘贴远端地址；若地址内含访问密钥，可写成
              <span class="mono">?token=KEY</span> 形式一并带入。
            </div>
          </div>
          <div class="row">
            <label class="label" for="sysdir-key">访问密钥（可选，也可写在地址里）</label>
            <input
              id="sysdir-key"
              v-model="remoteKey"
              class="text-input mono"
              type="text"
              placeholder="Bearer Token（留空则仅用地址中的 key）"
              @keydown.enter="handleSubmit"
            />
          </div>
          <div class="hint warn" v-if="currentIsRemote">
            当前已连接到远端：<span class="mono">{{ currentRemoteLabel }}</span>
          </div>
        </template>

        <div v-if="error" class="error">{{ error }}</div>
      </div>

      <footer class="modal-footer">
        <button class="btn" @click="handleClose" :disabled="busy">取消</button>
        <button
          class="btn btn-primary"
          @click="handleSubmit"
          :disabled="busy || !canSubmit"
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
import { computed, onMounted, ref, watch } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import {
  getHomedirInfo,
  switchHomedir,
} from '@/services/home'
import {
  loadSystemLocation,
  saveSystemLocation,
  currentLocation,
  parseRemoteAddress,
  isValidRemoteUrl,
  formatLocation,
  type SystemLocation,
} from '@/services/systemLocation'
import { reloadGatewayTransport } from '@/services/plugin'
import { logger } from '@/utils/logger'
import { useToast } from '@/composables/useToast'
import ConfirmDialog from './ConfirmDialog.vue'
import type { Response as ReloadResponse } from '@/schemas/home_reload'

interface Props {
  open: boolean
}
const props = defineProps<Props>()
const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'reloaded', response: ReloadResponse | null): void
}>()

const { showToast } = useToast()

const mode = ref<'local' | 'remote'>('local')
const localPath = ref('')
const remoteUrl = ref('')
const remoteKey = ref('')
const bootstrapPath = ref('')
const error = ref('')
const busy = ref(false)
const browsing = ref(false)
const confirmOpen = ref(false)
const confirmMessage = ref('')

const currentIsRemote = computed(() => currentLocation.value.kind === 'remote')
const currentRemoteLabel = computed(() => formatLocation(currentLocation.value))
const canSubmit = computed(() => {
  if (mode.value === 'local') return localPath.value.trim().length > 0
  const { url } = parseRemoteAddress(remoteUrl.value)
  return isValidRemoteUrl(url)
})

watch(
  () => props.open,
  async (v) => {
    if (v) await refresh()
  }
)

onMounted(async () => {
  if (props.open) await refresh()
})

async function refresh() {
  error.value = ''
  const saved = loadSystemLocation()
  mode.value = saved.kind
  if (saved.kind === 'remote') {
    remoteUrl.value = saved.remoteUrl ?? ''
    remoteKey.value = saved.remoteKey ?? ''
  } else {
    localPath.value = saved.localPath ?? ''
  }
  // 本地模式下顺带拉取当前 homedir 与 bootstrap 位置（native 调用，安全）
  if (saved.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      bootstrapPath.value = info.bootstrap_path || ''
      if (!localPath.value) localPath.value = info.homedir || ''
    } catch (err) {
      logger.warn('sysdir-switcher', '读取 homedir 信息失败（不影响切换）', err)
    }
  }
}

function handleClose() {
  if (busy.value) return
  emit('update:open', false)
  error.value = ''
  localPath.value = ''
  remoteUrl.value = ''
  remoteKey.value = ''
}

async function handleBrowse() {
  browsing.value = true
  try {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: '选择系统目录 (homedir)',
    })
    if (typeof selected === 'string' && selected.length > 0) {
      localPath.value = selected
    }
  } catch (err) {
    logger.error('sysdir-switcher', 'Browse failed:', err)
  } finally {
    browsing.value = false
  }
}

function handleSubmit() {
  if (busy.value) return
  error.value = ''

  if (mode.value === 'local') {
    const trimmed = localPath.value.trim()
    if (!trimmed) {
      error.value = '请输入目标 homedir 路径'
      return
    }
  } else {
    const { url } = parseRemoteAddress(remoteUrl.value)
    if (!isValidRemoteUrl(url)) {
      error.value = '远端地址格式无效（需以 http:// 或 https:// 开头且含 host）'
      return
    }
  }

  confirmMessage.value =
    '即将切换系统目录，所有活跃 chat 会话将被关闭，UI 数据将重新加载。是否继续？'
  confirmOpen.value = true
}

async function handleConfirm() {
  confirmOpen.value = false
  busy.value = true
  error.value = ''
  try {
    if (mode.value === 'local') {
      await applyLocal(localPath.value.trim())
    } else {
      await applyRemote()
    }
    emit('reloaded', null)
    emit('update:open', false)
    resetFields()
  } catch (err: any) {
    logger.error('sysdir-switcher', '切换失败:', err)
    error.value = `切换失败: ${err?.message ?? String(err)}`
  } finally {
    busy.value = false
  }
}

/** 本地目录：持久化 + 本机 home/reload（forceNative 命中本机后端） */
async function applyLocal(path: string) {
  const loc: SystemLocation = { kind: 'local', localPath: path }
  saveSystemLocation(loc)
  // 热重载本机 homedir（控制面，强制 native）
  const resp = await switchHomedir(path, { forceNative: true })
  reloadGatewayTransport()
  if (!resp) {
    showToast('info', `已切换到本地目录：${path}`)
  } else {
    showToast('success', `已切换到本地目录：${path}`)
  }
}

/** 远端地址：解析(含 key) + 校验 + 重载传输（不可达自动回退 native） */
async function applyRemote() {
  const { url, key: urlKey } = parseRemoteAddress(remoteUrl.value)
  const key = (remoteKey.value.trim() || urlKey).trim()
  if (!isValidRemoteUrl(url)) {
    throw new Error('远端地址格式无效')
  }
  const loc: SystemLocation = { kind: 'remote', remoteUrl: url, remoteKey: key }
  saveSystemLocation(loc)
  // 重载传输：内部做健康探测，不可达则安全回退 native 并置 locationError
  await reloadGatewayTransport()
  showToast('info', `已切换到远端：${url}（若不可达将自动回退本地）`)
}

function resetFields() {
  localPath.value = ''
  remoteUrl.value = ''
  remoteKey.value = ''
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
  width: min(38rem, 92vw);
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
.seg {
  display: flex;
  gap: var(--space-2);
}
.seg-btn {
  flex: 1;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-base);
  transition: all var(--motion-fast) var(--motion-ease);
}
.seg-btn.active {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}
.seg-btn:hover:not(.active) { background: var(--surface-hover); }
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
.hint.warn { color: var(--warning-fg, #b45309); }
.mono { font-family: var(--font-mono); }
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
.btn:hover:not(:disabled) { background: var(--surface-hover); }
.btn:disabled { opacity: 0.5; cursor: not-allowed; }
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

<!--
  HomedirSwitcher.vue — 「数据位置」切换对话框

  这是全 App 唯一能改「数据放在哪儿 / 连到哪儿」的入口。对用户的表述统一为
  「数据位置」，不再暴露 homedir / bootstrap / Server 等实现术语。

  两类目标（internal 仍沿用 local / remote 兼容层）：
  - local：本机数据目录（Web 交付物里就是当前服务的数据目录）
  - remote：另一台服务的 HTTP/WS 地址，密钥只走 Authorization 请求头

  设计取舍：
  - 路径只出现一次：当前值只在输入框里显示，bootstrap 路径不再重复展示。
  - 主操作被弱化：默认只引导「选择目录」，手动输入收进「手动填写」折叠区，
    避免一上来就是一条长路径 + 一段术语说明。
  - 破坏性操作仍必经二次确认，校验与失败回滚语义完全保留。
-->
<template>
  <BaseModal :visible="open" panel-class="modal" @close="handleClose">
    <header class="modal-header">
      <h3 id="sysdir-title">数据位置</h3>
      <button class="close-btn" @click="handleClose" aria-label="关闭">×</button>
    </header>

    <div class="modal-body">
      <!-- 目标类型 -->
      <div class="seg" role="group" aria-label="位置类型">
        <button
          class="seg-btn"
          :class="{ active: mode === 'local' }"
          :aria-pressed="mode === 'local'"
          @click="mode = 'local'"
        >本机目录</button>
        <button
          class="seg-btn"
          :class="{ active: mode === 'remote' }"
          :aria-pressed="mode === 'remote'"
          @click="mode = 'remote'"
        >远端服务</button>
      </div>

      <!-- 本机目录 -->
      <template v-if="mode === 'local'">
        <div class="row">
          <!-- 当前位置只显示一次：浏览态是一行摘要，手动态才换成输入框，不并排重复 -->
          <div v-if="!manualOpen" class="current-card">
            <span class="current-label">当前位置</span>
            <span class="current-path mono" :title="localPath">{{ localPath || '默认位置' }}</span>
          </div>
          <div v-else class="manual-row">
            <label class="label" for="sysdir-input">文件夹路径</label>
            <input
              id="sysdir-input"
              v-model="localPath"
              class="text-input mono"
              type="text"
              placeholder="粘贴一个文件夹路径"
              @keydown.enter="onKeyEnter"
            />
          </div>

          <div class="actions-row">
            <button class="primary-pick" type="button" @click="handleBrowse" :disabled="browsing">
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
              </svg>
              <span>{{ browsing ? '打开中…' : '选择文件夹' }}</span>
            </button>
            <button class="manual-toggle" type="button" :aria-expanded="manualOpen" @click="manualOpen = !manualOpen">
              {{ manualOpen ? '完成填写' : '手动填写' }}
            </button>
          </div>

          <p class="hint">
            切换后将从新位置加载模型、会话等数据，当前对话会结束并回到首页。
          </p>
        </div>
      </template>

      <!-- 远端服务 -->
      <template v-else>
        <div class="row">
          <label class="label" for="sysdir-url">服务地址</label>
          <input
            id="sysdir-url"
            v-model="remoteUrl"
            class="text-input mono"
            type="text"
            placeholder="http://主机:端口"
            @keydown.enter="onKeyEnter"
          />
          <label class="label" for="sysdir-key">访问密钥（可选）</label>
          <input
            id="sysdir-key"
            v-model="remoteKey"
            class="text-input mono"
            type="text"
            placeholder="可留空，也可写在地址后面"
            @keydown.enter="onKeyEnter"
          />
          <p class="hint">含密钥的地址请勿分享给他人。</p>
          <p v-if="currentIsRemote" class="hint warn">
            当前正在使用：{{ currentRemoteLabel }}
          </p>
        </div>
      </template>

      <div v-if="error" class="error" role="alert">{{ error }}</div>
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
  </BaseModal>

  <DirectoryPicker
    v-if="open && pickerOpen"
    :visible="true"
    title="选择位置文件夹"
    :initial-path="localPath"
    @close="pickerOpen = false"
    @select="onDirectorySelected"
  />

  <!-- 二次确认：提示活跃 chat 会话将被关闭。
       与 BaseModal 平级，用 `v-if="open"` 保持「父弹窗关了它也走」的原行为。 -->
  <ConfirmDialog
    v-if="open"
    :visible="confirmOpen"
    title="切换到这个数据位置？"
    :message="confirmMessage"
    confirm-text="切换"
    cancel-text="取消"
    :loading="busy"
    @confirm="handleConfirm"
    @cancel="confirmOpen = false"
  />
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
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
} from '@/services/systemLocation'
import { reloadGatewayTransport } from '@/services/plugin'
import { pickPath } from '@/services/nativePick'
import { logger } from '@/utils/logger'
import { isImeComposing } from '@/utils/keyboard'
import { useToast } from '@/composables/useToast'
import BaseModal from './BaseModal.vue'
import DirectoryPicker from './DirectoryPicker.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import type { home_reload } from '@/schemas/home'

interface Props {
  open: boolean
}
const props = defineProps<Props>()
const emit = defineEmits<{
  (e: 'update:open', value: boolean): void
  (e: 'reloaded', response: home_reload.Response | null): void
}>()

const { showToast } = useToast()

const mode = ref<'local' | 'remote'>('local')
const localPath = ref('')
const remoteUrl = ref('')
const remoteKey = ref('')
const error = ref('')
const busy = ref(false)
const browsing = ref(false)
const pickerOpen = ref(false)
const confirmOpen = ref(false)
const confirmMessage = ref('')
const manualOpen = ref(false)

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

async function refresh() {
  error.value = ''
  manualOpen.value = false
  const saved = loadSystemLocation()
  mode.value = saved.kind
  if (saved.kind === 'remote') {
    remoteUrl.value = saved.remoteUrl ?? ''
    remoteKey.value = saved.remoteKey ?? ''
  } else {
    localPath.value = saved.localPath ?? ''
    // saved 只有「用户手动选过的路径」；空值才向服务问当前数据位置，避免两处重复显示
    if (!localPath.value) {
      try {
        const info = await getHomedirInfo()
        localPath.value = info.homedir || ''
      } catch (err) {
        logger.warn('data-location', '读取当前数据位置失败（仍可手动选择）', err)
      }
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
  pickerOpen.value = false
  manualOpen.value = false
}

async function handleBrowse() {
  browsing.value = true
  try {
    const selected = await pickPath('directory', {
      title: '选择位置文件夹',
      initialPath: localPath.value,
      onBrowserPick: () => { pickerOpen.value = true },
    })
    if (selected) localPath.value = selected
  } catch (err) {
    logger.error('data-location', '选择文件夹失败:', err)
    showToast('error', `选择文件夹失败：${err instanceof Error ? err.message : String(err)}`)
  } finally {
    browsing.value = false
  }
}

function onDirectorySelected(path: string) {
  localPath.value = path
  pickerOpen.value = false
}

/** 路径可能含中文：回车先让给输入法上屏，再谈提交 */
function onKeyEnter(e: KeyboardEvent) {
  if (isImeComposing(e)) return
  handleSubmit()
}

function handleSubmit() {
  if (busy.value) return
  error.value = ''

  if (mode.value === 'local') {
    const trimmed = localPath.value.trim()
    if (!trimmed) {
      error.value = '请选择或填写一个文件夹'
      return
    }
  } else {
    const { url } = parseRemoteAddress(remoteUrl.value)
    if (!isValidRemoteUrl(url)) {
      error.value = '地址格式无效（需以 http:// 或 https:// 开头）'
      return
    }
  }

  confirmMessage.value =
    '切换后当前对话会结束，模型、会话等数据将从新位置加载。是否继续？'
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
  } catch (err: unknown) {
    logger.error('data-location', '切换失败:', err)
    error.value = `切换失败: ${err instanceof Error ? err.message : String(err)}`
  } finally {
    busy.value = false
  }
}

/** local：先切出站传输，再让新目标执行 home/reload；失败恢复原目标与传输。 */
async function applyLocal(path: string) {
  const previous = loadSystemLocation()
  const loc = { kind: 'local' as const, localPath: path }
  saveSystemLocation(loc)
  try {
    await reloadGatewayTransport()
    await switchHomedir(path)
  } catch (error) {
    saveSystemLocation(previous)
    try {
      await reloadGatewayTransport()
    } catch (rollbackError) {
      logger.error('data-location', '恢复原位置失败:', rollbackError)
    }
    throw error
  }
  showToast('success', `已切换到：${path}`)
}

/** 远端地址：解析(含 key) + 校验 + 重载传输；不可达时保留 HTTP 并显示连接错误。 */
async function applyRemote() {
  const { url, key: urlKey } = parseRemoteAddress(remoteUrl.value)
  const key = (remoteKey.value.trim() || urlKey).trim()
  if (!isValidRemoteUrl(url)) {
    throw new Error('地址格式无效')
  }
  saveSystemLocation({ kind: 'remote', remoteUrl: url, remoteKey: key })
  await reloadGatewayTransport()
  showToast('info', `已连接到：${url}（连接不上时会提示，不会自动切回本机）`)
}

function resetFields() {
  localPath.value = ''
  remoteUrl.value = ''
  remoteKey.value = ''
  manualOpen.value = false
}
</script>

<style scoped>
:global(.modal-panel.modal) {
  width: min(34rem, 92vw);
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
  min-height: 1.5rem;
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
.current-card {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
}
.current-label {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
.current-path {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  overflow-wrap: anywhere;
}
.actions-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}
.primary-pick {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-3);
  border: 1px dashed var(--accent);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--accent);
  font-size: var(--font-size-base);
  cursor: pointer;
  transition: background var(--motion-fast) var(--motion-ease);
}
.primary-pick:hover:not(:disabled) { background: var(--surface-hover); }
.primary-pick:disabled { opacity: 0.5; cursor: not-allowed; }
.manual-toggle {
  align-self: flex-start;
  padding: var(--space-1) 0;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}
.manual-toggle:hover { color: var(--text-primary); }
.manual-row {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
/* .text-input 共享定义见 src/styles/controls.css */
.label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: var(--font-weight-medium);
}
.hint {
  margin: 0;
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

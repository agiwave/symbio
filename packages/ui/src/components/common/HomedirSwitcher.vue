<!--
  HomedirSwitcher.vue  —  系统目录 (System Location) 切换对话框

  统一「当前 Server（或本机）/ 远端地址」两类连接目标：
  - local：embedded 连本机 native；browser 连同源 Server，可切换 Server homedir
  - remote：连另一台 Symbio Server（HTTP/WS），地址字符串可内嵌 key（?token=）

  控制面在 embedded 中走 native，在 browser 中走当前 Server HTTP；浏览器没有 IPC
  回退。目录浏览始终操作目标 Server 的文件系统，homedir 不是会话 workdir。

  对应后端: symbio/src/plugins/home/plugin.rs::route "home/reload"
            symbio/src/plugins/gateway/plugin.rs（入站配置 = `<根>/gateway/PLUGIN.yml`）
  对应服务: packages/ui/src/services/home.ts (switchHomedir / getHomedirInfo)
            packages/ui/src/services/systemLocation.ts (系统目录模型与持久化)
  对应 schema: packages/ui/src/schemas/home.ts (home_reload 命名空间)
-->
<template>
  <BaseModal :visible="open" panel-class="modal" @close="handleClose">
      <header class="modal-header">
        <h3 id="sysdir-title">系统目录</h3>
        <button class="close-btn" @click="handleClose" aria-label="关闭">×</button>
      </header>

      <div class="modal-body">
        <!-- 模式切换 -->
        <div class="seg" role="group" aria-label="连接方式">
          <button
            class="seg-btn"
            :class="{ active: mode === 'local' }"
            :aria-pressed="mode === 'local'"
            @click="mode = 'local'"
          >{{ inBrowser ? '当前 Server' : '本地目录' }}</button>
          <button
            class="seg-btn"
            :class="{ active: mode === 'remote' }"
            :aria-pressed="mode === 'remote'"
            @click="mode = 'remote'"
          >远端地址</button>
        </div>

        <!-- 本地目录 -->
        <template v-if="mode === 'local'">
          <div class="row">
            <label class="label" for="sysdir-input">{{ inBrowser ? '当前 Server homedir 路径' : '本地 homedir 路径' }}</label>
            <div class="input-row">
              <input
                id="sysdir-input"
                v-model="localPath"
                class="text-input mono"
                type="text"
                placeholder="例如 ~/.symbio 或 /path/to/your/homedir"
                @keydown.enter="onKeyEnter"
              />
              <button class="btn" @click="handleBrowse" :disabled="browsing">浏览…</button>
            </div>
            <div class="hint">
              切换后，Symbio 会从新 homedir 重新加载所有子插件（Agent / LLM / MCP / Skill / Session）。
              homedir 是配置根，不是会话工作目录。
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
              @keydown.enter="onKeyEnter"
            />
            <div class="hint">
              直接粘贴远端地址；若地址内含访问密钥，可写成
              <span class="mono">?token=KEY</span> 形式一并带入。
              含有密钥的 URL 不要分享；WS 不会把 token 放进 URL。
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
              @keydown.enter="onKeyEnter"
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
  </BaseModal>

  <DirectoryPicker
    v-if="open && pickerOpen"
    :visible="true"
    title="选择系统目录（homedir）"
    :initial-path="localPath"
    @close="pickerOpen = false"
    @select="onDirectorySelected"
  />

  <!-- 二次确认：提示活跃 chat 会话将被关闭。
       它原本嵌在遮罩内（靠更高的 z-index 盖在上面）；现在与 BaseModal 平级，
       用 `v-if="open"` 保持「父弹窗关了它也走」的原行为。 -->
  <ConfirmDialog
    v-if="open"
    :visible="confirmOpen"
    title="确认切换系统目录？"
    :message="confirmMessage"
    confirm-text="切换"
    cancel-text="取消"
    :loading="busy"
    @confirm="handleConfirm"
    @cancel="confirmOpen = false"
  />
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
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
  isBrowserEnvironment,
  type SystemLocation,
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
const inBrowser = isBrowserEnvironment()

const mode = ref<'local' | 'remote'>('local')
const localPath = ref('')
const remoteUrl = ref('')
const remoteKey = ref('')
const bootstrapPath = ref('')
const error = ref('')
const busy = ref(false)
const browsing = ref(false)
const pickerOpen = ref(false)
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
  // local 模式拉取目标后端（embedded 本机 / browser 当前 Server）的 homedir 与 bootstrap。
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
  pickerOpen.value = false
}

async function handleBrowse() {
  browsing.value = true
  try {
    const selected = await pickPath('directory', {
      title: '选择系统目录 (homedir)',
      initialPath: localPath.value,
      onBrowserPick: () => { pickerOpen.value = true },
    })
    if (selected) localPath.value = selected
  } catch (err) {
    logger.error('sysdir-switcher', 'Browse failed:', err)
    showToast('error', `选择目录失败：${err instanceof Error ? err.message : String(err)}`)
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
    '即将切换系统目录：所有活跃 chat 会话将被关闭，界面数据将重新加载，并回到资源根目录（当前浏览位置会重置）。是否继续？'
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
    logger.error('sysdir-switcher', '切换失败:', err)
    error.value = `切换失败: ${err instanceof Error ? err.message : String(err)}`
  } finally {
    busy.value = false
  }
}

/** local：先切出站传输，再让新目标执行 home/reload；失败恢复原目标与传输。 */
async function applyLocal(path: string) {
  const previous = loadSystemLocation()
  const loc: SystemLocation = { kind: 'local', localPath: path }
  saveSystemLocation(loc)
  try {
    await reloadGatewayTransport()
    await switchHomedir(path)
  } catch (error) {
    saveSystemLocation(previous)
    try {
      await reloadGatewayTransport()
    } catch (rollbackError) {
      logger.error('sysdir-switcher', '恢复原传输失败:', rollbackError)
    }
    throw error
  }
  const label = inBrowser ? '当前 Server' : '本地目录'
  showToast('success', `已切换到${label}：${path}`)
}

/** 远端地址：解析(含 key) + 校验 + 重载传输；不可达时保留 HTTP 并显示连接错误。 */
async function applyRemote() {
  const { url, key: urlKey } = parseRemoteAddress(remoteUrl.value)
  const key = (remoteKey.value.trim() || urlKey).trim()
  if (!isValidRemoteUrl(url)) {
    throw new Error('远端地址格式无效')
  }
  const loc: SystemLocation = { kind: 'remote', remoteUrl: url, remoteKey: key }
  saveSystemLocation(loc)
  // browser 与 embedded 都保留已选 Server HTTP；不可达由 locationError 显示。
  await reloadGatewayTransport()
  const suffix = '（不可达时会显示连接错误，不会回退本机）'
  showToast('info', `已切换到远端：${url}${suffix}`)
}

function resetFields() {
  localPath.value = ''
  remoteUrl.value = ''
  remoteKey.value = ''
}
</script>

<style scoped>
/* 遮罩与面板底色 / 圆角 / 阴影 / 层级由 `BaseModal` 统一提供；这里只写尺寸。
   顺带修掉一处层级错位：原遮罩用 `--z-overlay`(1000)，低于其它弹窗的
   `--z-dialog`(1500)，与内嵌确认框的层级关系反直觉。 */
:global(.modal-panel.modal) {
  width: min(38rem, 92vw);
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
.label {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: var(--font-weight-medium);
}
.input-row {
  display: flex;
  gap: var(--space-2);
}
/* .text-input 不再持副本：共享定义见 src/styles/controls.css（按本处的实际渲染取值） */
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

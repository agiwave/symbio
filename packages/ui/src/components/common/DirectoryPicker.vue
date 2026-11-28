<!--
  DirectoryPicker — 数据文件夹选择器。

  只通过宿主目录 API 浏览当前连接目标的文件系统；不读取浏览器本机，不使用
  <input type=file>。调用方负责维护 visible，并在 select 事件中写回所选路径。

  展示取舍：当前路径只出现在地址输入框里（不再另起一行重复一份）；目录条目
  只显示文件夹名，最近路径显示末段短名，完整路径保留在 title 里供悬停核对。
-->
<template>
  <BaseModal
    :visible="visible"
    panel-class="directory-picker"
    :labelledby="titleId"
    @close="close"
  >
    <header class="picker-header">
      <h3 :id="titleId">{{ title }}</h3>
      <button type="button" class="close-btn" aria-label="关闭" @click="close">×</button>
    </header>

    <div class="picker-body" :aria-busy="loading ? 'true' : undefined">
      <form class="path-form" @submit.prevent="goToPath">
        <input
          :id="pathInputId"
          v-model="pathDraft"
          aria-label="文件夹路径"
          class="path-input"
          type="text"
          spellcheck="false"
          autocomplete="off"
          :disabled="loading"
          placeholder="输入或粘贴文件夹路径"
          aria-describedby="pathHelpId"
        >
        <button type="submit" class="btn" :disabled="loading">前往</button>
      </form>
      <span :id="pathHelpId" class="path-help">支持 Windows、Unix 与 UNC 路径；也可用 ~/ 开头</span>
      <p v-if="pathError" class="error" role="alert">{{ pathError }}</p>

      <div v-if="recentPaths.length" class="recent" aria-label="最近使用的文件夹">
        <span class="path-label">最近使用</span>
        <div class="recent-list">
          <button
            v-for="path in recentPaths"
            :key="path"
            type="button"
            class="root-btn"
            :disabled="loading"
            :title="path"
            @click="navigate(path)"
          >{{ shortName(path) }}</button>
        </div>
      </div>

      <div v-if="roots.length" class="roots" aria-label="根目录">
        <button
          v-for="root in roots"
          :key="root.path"
          type="button"
          class="root-btn"
          :disabled="loading"
          :title="root.path"
          @click="navigate(root.path)"
        >{{ root.name }}</button>
      </div>

      <div class="toolbar">
        <button
          type="button"
          class="btn"
          :disabled="loading || !listing?.parent || listing.parent === listing.path"
          @click="navigate(listing!.parent!)"
        >上一级</button>
        <span v-if="loading" class="loading-text">正在读取文件夹…</span>
        <span v-else class="path-crumb" :title="listing?.path || ''">{{ listing?.path || '' }}</span>
      </div>

      <p v-if="error" class="error" role="alert">{{ error }}</p>
      <div v-else-if="!loading && listing && entries.length === 0" class="empty">
        这个文件夹里没有子文件夹
      </div>
      <ul v-else-if="listing" class="directory-list" aria-label="文件夹列表">
        <li v-for="entry in entries" :key="entry.path">
          <button
            type="button"
            class="directory-entry"
            :disabled="loading"
            :title="entry.path"
            @click="navigate(entry.path)"
          >
            <span class="folder-icon" aria-hidden="true">▸</span>
            <span class="entry-name">{{ entry.name }}</span>
          </button>
        </li>
      </ul>

      <button v-if="error" type="button" class="retry-btn" @click="load(listing?.path)">
        重试
      </button>
    </div>

    <footer class="picker-footer">
      <button type="button" class="btn" @click="close">取消</button>
      <button
        type="button"
        class="btn btn-primary"
        :disabled="loading || Boolean(error) || !listing"
        @click="selectCurrent"
      >选择当前文件夹</button>
    </footer>
  </BaseModal>
</template>

<script setup lang="ts">
import { ref, useId, watch } from 'vue'
import BaseModal from './BaseModal.vue'
import {
  getHostRuntime,
  listHostDirectories,
  type HostDirectoryListing,
  type HostDirectoryRoot,
} from '@/services/hostDirectories'

const props = withDefaults(defineProps<{
  visible: boolean
  title?: string
  initialPath?: string
  recentPaths?: string[]
}>(), {
  title: '选择文件夹',
  initialPath: '',
  recentPaths: () => [],
})

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'select', path: string): void
}>()

const titleId = useId()
const pathInputId = `${titleId}-path`
const pathHelpId = `${titleId}-help`
const pathDraft = ref('')
const pathError = ref('')
const listing = ref<HostDirectoryListing | null>(null)
const roots = ref<HostDirectoryRoot[]>([])
const entries = ref<HostDirectoryListing['entries']>([])
const loading = ref(false)
const error = ref('')
let requestSequence = 0

/** 路径末段：用于最近使用的短标签，完整路径仍在 title 里 */
function shortName(path: string): string {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] || path
}

/** 接受 Server 绝对路径与后端支持的 `~` / `~/` 简写。 */
function absoluteServerPath(path: string | undefined): string | undefined {
  const trimmed = path?.trim()
  return trimmed && /^(?:[A-Za-z]:[\\/]|\/|\\\\|~(?:[\\/]|$))/.test(trimmed) ? trimmed : undefined
}

async function load(requestedPath?: string): Promise<void> {
  const sequence = ++requestSequence
  loading.value = true
  error.value = ''
  try {
    let path = absoluteServerPath(requestedPath)
    if (!path) {
      const runtime = await getHostRuntime()
      if (sequence !== requestSequence) return
      if (!runtime.capabilities.directory_browser) {
        throw new Error('当前服务未开放文件夹浏览能力')
      }
      path = absoluteServerPath(runtime.homedir)
    }
    const next = await listHostDirectories(path)
    if (sequence !== requestSequence) return
    listing.value = next
    pathDraft.value = next.path
    roots.value = next.roots
    entries.value = next.entries
  } catch (cause) {
    if (sequence !== requestSequence) return
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (sequence === requestSequence) loading.value = false
  }
}

function goToPath(): void {
  pathError.value = ''
  const path = absoluteServerPath(pathDraft.value)
  if (!path) {
    pathError.value = '请输入完整路径（如 C:\\projects 或 /srv/projects），或 ~/ 开头的路径'
    return
  }
  void load(path)
}

function navigate(path: string): void {
  pathError.value = ''
  if (!loading.value) void load(path)
}

function close(): void {
  requestSequence += 1
  loading.value = false
  emit('close')
}

function selectCurrent(): void {
  if (!listing.value || loading.value || error.value) return
  emit('select', listing.value.path)
}

watch(
  () => props.visible,
  (visible) => {
    if (visible) {
      pathError.value = ''
      pathDraft.value = props.initialPath
      void load(props.initialPath)
    } else requestSequence += 1
  },
  { immediate: true },
)
</script>

<style scoped>
:global(.modal-panel.directory-picker) {
  width: min(38rem, 92vw);
  max-height: 88vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.picker-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border-default);
}
.picker-header h3 {
  margin: 0;
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-semibold);
}
.close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-size: 1.4rem;
  cursor: pointer;
}
.picker-body {
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-4) var(--space-5);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.path-label {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
.path-crumb {
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.path-form {
  display: flex;
  gap: var(--space-2);
}
.path-input {
  flex: 1;
  min-width: 0;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-family: var(--font-mono);
}
.path-input:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.path-help {
  color: var(--text-muted);
  font-size: var(--font-size-xs);
}
.recent {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.recent-list {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.roots,
.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}
.toolbar .path-crumb {
  flex: 1;
  min-width: 8rem;
}
.root-btn,
.retry-btn,
.btn {
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-primary);
  cursor: pointer;
}
.root-btn {
  max-width: 100%;
  overflow-wrap: anywhere;
  text-align: left;
}
.root-btn:disabled,
.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.loading-text,
.empty {
  color: var(--text-muted);
  font-size: var(--font-size-sm);
}
.directory-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.directory-entry {
  width: 100%;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-primary);
  text-align: left;
  cursor: pointer;
}
.directory-entry:hover:not(:disabled),
.directory-entry:focus-visible,
.root-btn:hover:not(:disabled),
.root-btn:focus-visible,
.retry-btn:hover,
.btn:hover:not(:disabled) {
  background: var(--surface-hover);
}
.directory-entry:focus-visible,
.root-btn:focus-visible,
.retry-btn:focus-visible,
.btn:focus-visible,
.close-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.directory-entry:disabled { opacity: 0.5; cursor: not-allowed; }
.folder-icon { color: var(--text-secondary); }
.entry-name { font-weight: var(--font-weight-medium); overflow-wrap: anywhere; }
.error {
  margin: 0;
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  background: var(--danger-bg);
  color: var(--danger-fg);
}
.retry-btn { align-self: flex-start; }
.picker-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-5);
  border-top: 1px solid var(--border-default);
}
.btn-primary {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}
</style>

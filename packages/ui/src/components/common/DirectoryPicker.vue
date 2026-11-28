<!--
  DirectoryPicker — 数据文件夹选择器（可展开的目录树）

  主体是一棵懒加载目录树：每个节点首次展开时才向服务请求子目录，展开过的
  节点缓存。点节点选中、双击确认；选中节点即作为返回值。

  辅助入口：「最近使用」快捷选中。选择文件夹由后端目录 API 完成，不读浏览器
  本机、不用 <input type=file>。

  展示取舍：树节点只显示文件夹名，完整路径保留在 title 上供悬停核对；底部
  「当前选中」条给出唯一一处完整路径确认。
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

    <div class="picker-body">
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
            @click="revealPath(path)"
          >{{ shortName(path) }}</button>
        </div>
      </div>

      <div class="tree-wrap" :aria-busy="loading ? 'true' : undefined">
        <p v-if="error" class="error" role="alert">{{ error }}</p>
        <button v-if="error" type="button" class="retry-btn" @click="initialize">重试</button>

        <template v-else-if="loading">
          <p class="loading-text">正在读取文件夹…</p>
          <ul v-if="treeRoots.length" class="tree-root" role="tree">
            <DirectoryTreeNode
              v-for="node in treeRoots"
              :key="node.path"
              :node="node"
              :selected-path="selectedPath"
              @select="selectedPath = $event"
              @confirm="emit('select', $event)"
            />
          </ul>
        </template>

        <ul v-else-if="treeRoots.length" class="tree-root" role="tree" aria-label="文件夹树">
          <DirectoryTreeNode
            v-for="node in treeRoots"
            :key="node.path"
            :node="node"
            :selected-path="selectedPath"
            @select="selectedPath = $event"
            @confirm="emit('select', $event)"
          />
        </ul>

        <p v-else class="empty">没有可浏览的文件夹</p>
      </div>

      <div v-if="selectedPath" class="selection-bar">
        <span class="selection-label">当前选中</span>
        <span class="selection-path" :title="selectedPath">{{ selectedPath }}</span>
      </div>
    </div>

    <footer class="picker-footer">
      <button type="button" class="btn" @click="close">取消</button>
      <button
        type="button"
        class="btn btn-primary"
        :disabled="!selectedPath || loading || Boolean(error)"
        @click="emit('select', selectedPath)"
      >选择此文件夹</button>
    </footer>
  </BaseModal>
</template>

<script setup lang="ts">
import { ref, useId, watch } from 'vue'
import BaseModal from './BaseModal.vue'
import DirectoryTreeNode, { type TreeNode } from './DirectoryTreeNode.vue'
import {
  getHostRuntime,
  listHostDirectories,
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
const loading = ref(false)
const error = ref('')
const selectedPath = ref('')
const treeRoots = ref<TreeNode[]>([])
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

function rootNodes(roots: HostDirectoryRoot[]): TreeNode[] {
  return roots.map((root) => ({ name: root.name, path: root.path, expandable: true }))
}

/** 初始化根节点列表：用目录 API 不带 path 的返回值（服务给出可浏览根） */
async function initialize(): Promise<void> {
  const sequence = ++requestSequence
  loading.value = true
  error.value = ''
  try {
    const runtime = await getHostRuntime()
    if (sequence !== requestSequence) return
    if (!runtime.capabilities.directory_browser) {
      throw new Error('当前服务未开放文件夹浏览能力')
    }
    const listing = await listHostDirectories()
    if (sequence !== requestSequence) return
    treeRoots.value = listing.roots.length
      ? rootNodes(listing.roots)
      : listing.entries.map((entry) => ({ name: entry.name, path: entry.path, expandable: true }))
    // 初始路径作为选中态；完整路径在底部选中条确认
    selectedPath.value = absoluteServerPath(props.initialPath) || ''
  } catch (cause) {
    if (sequence !== requestSequence) return
    error.value = cause instanceof Error ? cause.message : String(cause)
    treeRoots.value = []
  } finally {
    if (sequence === requestSequence) loading.value = false
  }
}

/** 最近使用快捷入口：直接选中该路径 */
function revealPath(path: string): void {
  if (loading.value) return
  selectedPath.value = path
}

function close(): void {
  requestSequence += 1
  loading.value = false
  emit('close')
}

watch(
  () => props.visible,
  (visible) => {
    if (visible) {
      error.value = ''
      void initialize()
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
.tree-wrap {
  min-height: 12rem;
  max-height: 42vh;
  overflow-y: auto;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
}
.tree-root {
  list-style: none;
  margin: 0;
  padding: 0;
}
.loading-text,
.empty {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--font-size-sm);
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
.selection-bar {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-panel);
}
.selection-label {
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: var(--font-size-xs);
}
.selection-path {
  min-width: 0;
  overflow-wrap: anywhere;
  font-family: var(--font-mono);
  font-size: var(--font-size-sm);
}
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
.root-btn:focus-visible,
.retry-btn:focus-visible,
.btn:focus-visible,
.close-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
</style>

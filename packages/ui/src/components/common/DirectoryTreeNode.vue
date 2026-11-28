<!--
  DirectoryTreeNode — 懒加载目录树的单个节点（递归组件）

  一个实例对应一个文件夹节点：点击三角箭头时首次向服务请求子目录
  （listHostDirectories(path)），子项缓存，折叠再展开不会重复请求。
  组件不持有「当前位置」的概念：selectedPath 由父级维护，点击节点选中，
  双击确认。

  <script setup> 里组件按文件名自引用（Vue 3.3+），无需额外 import。
-->
<template>
  <li class="tree-item" role="none" :aria-expanded="node.expandable ? expanded : undefined">
    <div class="tree-row" :class="{ selected: node.path === selectedPath }">
      <button
        type="button"
        class="twisty"
        :class="{ placeholder: !node.expandable }"
        :disabled="!node.expandable || loading"
        :aria-label="node.expandable ? (expanded ? `折叠 ${node.name}` : `展开 ${node.name}`) : undefined"
        :aria-expanded="node.expandable ? expanded : undefined"
        @click="toggleExpand"
      >
        <span v-if="expanding" class="spinner" aria-hidden="true" />
        <span v-else-if="node.expandable" aria-hidden="true">{{ expanded ? '▾' : '▸' }}</span>
      </button>
      <button
        type="button"
        class="node-label"
        :title="node.path"
        @click="select"
        @dblclick="choose"
      >
        <span class="folder-icon" aria-hidden="true">📁</span>
        <span class="node-name">{{ node.name }}</span>
      </button>
    </div>

    <p v-if="nodeError" class="node-error" role="alert">{{ nodeError }}</p>
    <ul v-else-if="expanded && children.length" class="tree-children" role="group">
      <DirectoryTreeNode
        v-for="child in children"
        :key="child.path"
        :node="child"
        :selected-path="selectedPath"
        @select="emit('select', $event)"
        @confirm="emit('confirm', $event)"
      />
    </ul>
    <p v-else-if="expanded && !loading && loaded && children.length === 0" class="node-empty">
      没有子文件夹
    </p>
  </li>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import {
  listHostDirectories,
  type HostDirectoryEntry,
} from '@/services/hostDirectories'

export interface TreeNode {
  name: string
  path: string
  expandable: boolean
}

const props = withDefaults(defineProps<{
  node: TreeNode
  selectedPath?: string
}>(), {
  selectedPath: '',
})

const emit = defineEmits<{
  (e: 'select', path: string): void
  (e: 'confirm', path: string): void
}>()

const expanded = ref(false)
const loaded = ref(false)
const loading = ref(false)
const expanding = ref(false)
const children = ref<TreeNode[]>([])
const nodeError = ref('')

async function toggleExpand(): Promise<void> {
  if (!props.node.expandable || loading.value) return
  if (expanded.value) {
    expanded.value = false
    return
  }
  if (loaded.value) {
    expanded.value = true
    return
  }
  loading.value = true
  expanding.value = true
  nodeError.value = ''
  try {
    const listing = await listHostDirectories(props.node.path)
    const toNode = (entry: HostDirectoryEntry): TreeNode => ({
      name: entry.name,
      path: entry.path,
      expandable: true,
    })
    // 点击展开后，孩子按 listing 的 entries 填充；加载过就缓存
    children.value = listing.entries.map(toNode)
    loaded.value = true
    expanded.value = true
  } catch (cause) {
    nodeError.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    loading.value = false
    expanding.value = false
  }
}

function select(): void {
  emit('select', props.node.path)
}
function choose(): void {
  emit('confirm', props.node.path)
}
</script>

<style scoped>
.tree-item {
  display: flex;
  flex-direction: column;
}
.tree-row {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  border-radius: var(--radius-md);
  transition: background var(--motion-fast) var(--motion-ease);
}
.tree-row:hover { background: var(--surface-hover); }
.tree-row.selected {
  background: var(--accent-subtle-bg);
  box-shadow: inset 2px 0 0 var(--accent);
}
.twisty {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.5rem;
  height: 1.75rem;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}
.twisty.placeholder { cursor: default; }
.twisty:disabled { opacity: 0.4; }
.node-label {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-2);
  border: none;
  background: transparent;
  color: var(--text-primary);
  text-align: left;
  cursor: pointer;
}
.folder-icon { flex-shrink: 0; font-size: 0.9rem; }
.node-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tree-row.selected .node-name {
  font-weight: var(--font-weight-semibold);
  color: var(--accent, var(--text-primary));
}
.tree-children {
  list-style: none;
  margin: 0 0 0 0.95rem;
  padding: 0 0 0 0.75rem;
  border-left: 1px solid var(--border-default);
}
.node-error,
.node-empty {
  margin: 0 0 0 2.5rem;
  color: var(--text-muted);
  font-size: var(--font-size-xs);
}
.node-error { color: var(--danger-fg); }
.spinner {
  display: inline-block;
  width: 0.7rem;
  height: 0.7rem;
  border: 2px solid var(--border-default);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: tree-spin 0.7s linear infinite;
}
@keyframes tree-spin { to { transform: rotate(360deg); } }
.twisty:focus-visible,
.node-label:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
</style>

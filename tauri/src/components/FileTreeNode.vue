<template>
  <div class="tree-node">
    <div
      class="node-content"
      :style="{ paddingLeft: (level * 12 + 8) + 'px' }"
      :class="{
        'is-dir': item.is_dir,
        'is-selected': selectedPath === item.path,
        'is-expanded': isExpanded
      }"
      @click="handleClick"
    >
      <span
        v-if="item.is_dir"
        class="expand-icon"
        :class="{ 'is-loading': isLoading }"
        @click.stop="handleExpandClick"
      >
        <span v-if="isLoading" class="mini-spinner" />
        <span v-else>{{ isExpanded ? '▼' : '▶' }}</span>
      </span>
      <span v-else class="expand-icon placeholder" />

      <span class="item-icon">
        {{ item.is_dir ? '📁' : getFileIcon(item.name) }}
      </span>

      <span class="item-name">{{ item.name }}</span>

      <span v-if="!item.is_dir && item.size" class="item-size">
        {{ formatSize(item.size) }}
      </span>
    </div>

    <!-- 子目录 -->
    <div v-if="item.is_dir && isExpanded" class="node-children">
      <FileTreeNode
        v-for="child in childItems"
        :key="child.path"
        :item="child"
        :level="level + 1"
        :selected-path="selectedPath"
        @select="$emit('select', $event)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 文件树节点（递归）
 *
 * - 展开状态由 explorer store 维护，跨组件重建能保留
 * - 第一次展开时自动触发懒加载（store.toggleExpand 内部处理）
 * - 子目录加载中显示小 spinner，不再把整棵树 v-if 切走
 */
import { computed } from 'vue'
import { type FileItem } from '@/services/protocol'
import { useExplorerStore } from '../stores/explorer'

const props = defineProps<{
  item: FileItem
  level: number
  selectedPath: string | null
  children?: FileItem[]
}>()

const emit = defineEmits<{
  (e: 'select', path: string): void
}>()

const store = useExplorerStore()

// 展开状态从 store 读取
const isExpanded = computed(() => store.isExpanded(props.item.path))
const isLoading = computed(() => store.isDirLoading(props.item.path))

// 子项：优先使用 props.children（父组件传过来的），否则从 store 查
const childItems = computed(() => {
  if (props.children && props.children.length > 0) {
    return props.children
  }
  if (props.item.is_dir) {
    return store.getChildren(props.item.path)
  }
  return []
})

// 点击行：选中 + 切换展开（仅目录）
function handleClick() {
  emit('select', props.item.path)
  if (props.item.is_dir) {
    store.toggleExpand(props.item.path)
  }
}

// 点击展开图标：仅切换展开
function handleExpandClick(e: MouseEvent) {
  e.stopPropagation()
  if (props.item.is_dir) {
    store.toggleExpand(props.item.path)
  }
}

function formatSize(size?: number): string {
  if (size === undefined) return ''
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / (1024 * 1024)).toFixed(1)} MB`
}

function getFileIcon(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase()
  const icons: Record<string, string> = {
    'md': '📝',
    'txt': '📄',
    'js': '📜',
    'ts': '📘',
    'vue': '💚',
    'json': '📋',
    'yaml': '📝',
    'yml': '📝',
    'html': '🌐',
    'css': '🎨',
    'png': '🖼️',
    'jpg': '🖼️',
    'jpeg': '🖼️',
    'gif': '🖼️',
    'svg': '🖼️',
    'pdf': '📕',
    'zip': '📦',
    'tar': '📦',
    'gz': '📦',
    'rs': '🦀',
    'py': '🐍',
    'go': '🔹',
    'java': '☕',
    'c': '⚙️',
    'cpp': '⚙️',
    'h': '⚙️',
    'hpp': '⚙️',
    'sh': '📜',
    'bash': '📜',
  }
  return icons[ext || ''] || '📄'
}
</script>

<style scoped>
.tree-node {
  display: flex;
  flex-direction: column;
}

.node-content {
  display: flex;
  align-items: center;
  padding: var(--space-1) var(--space-2);
  cursor: pointer;
  border-radius: 0.25rem;
  transition: background 0.15s;
  user-select: none;
}

.node-content:hover {
  background: #f0f0f0;
}

.node-content.is-selected {
  background: #e8e8f0;
}

.node-content.is-dir {
  font-weight: 500;
}

.expand-icon {
  width: 1rem;
  height: 1rem;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.625rem;
  color: var(--color-text-muted);
  margin-right: var(--space-05);
  flex-shrink: 0;
}

.expand-icon.placeholder {
  /* 文件节点：保留位置让对齐 */
}

.expand-icon:hover:not(.is-loading) {
  color: var(--color-text);
}

.expand-icon.is-loading {
  cursor: default;
}

.mini-spinner {
  width: 0.5rem;
  height: 0.5rem;
  border: 0.0938rem solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
  display: inline-block;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.item-icon {
  font-size: 1rem;
  margin-right: 0.5rem;
}

.item-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.875rem;
}

.item-size {
  font-size: 0.75rem;
  color: var(--color-text-muted);
  margin-left: 0.5rem;
}

.node-children {
  display: flex;
  flex-direction: column;
}
</style>

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
        @click.stop="toggleExpand"
      >
        {{ isExpanded ? '▼' : '▶' }}
      </span>
      <span v-else class="expand-icon"></span>
      
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
        @expand="$emit('expand', $event)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { type FileItem } from '../stores/explorer'
import { useExplorerStore } from '../stores/explorer'

const props = defineProps<{
  item: FileItem
  level: number
  selectedPath: string | null
  children?: FileItem[]
}>()

const emit = defineEmits<{
  (e: 'select', path: string): void
  (e: 'expand', path: string): void
}>()

const store = useExplorerStore()

// 子项：优先使用 props.children，否则从 store 获取
const childItems = computed(() => {
  if (props.children && props.children.length > 0) {
    return props.children
  }
  if (props.item.is_dir) {
    return store.getChildren(props.item.path)
  }
  return []
})

const isExpanded = ref(false)

// 点击处理
function handleClick() {
  emit('select', props.item.path)

  // 如果是目录，切换展开状态
  if (props.item.is_dir) {
    toggleExpand()
  }
}

// 切换展开状态
function toggleExpand() {
  isExpanded.value = !isExpanded.value
  if (isExpanded.value) {
    emit('expand', props.item.path)
  }
}

// 格式化大小
function formatSize(size?: number): string {
  if (size === undefined) return ''
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / (1024 * 1024)).toFixed(1)} MB`
}

// 获取文件图标
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
  padding: 4px 8px;
  cursor: pointer;
  border-radius: 4px;
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
  width: 16px;
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.625rem;
  color: var(--color-text-muted);
  margin-right: 2px;
}

.expand-icon:hover {
  color: var(--color-text);
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

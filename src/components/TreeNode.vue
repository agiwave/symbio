<template>
  <div
    class="tree-node"
    :style="{ paddingLeft: `${level * 16 + 8}px` }"
    :class="{ 
      active: document.id === activeId,
      'drag-over': isDragOver,
      'dragging': isDragging
    }"
    draggable="true"
    @dragstart="onDragStart"
    @dragend="onDragEnd"
    @dragover.prevent="onDragOver"
    @dragleave="onDragLeave"
    @drop="onDrop"
  >
    <div class="node-content" @click="$emit('select', document.id)">
      <span class="expand-icon" @click.stop="toggleExpand">
        {{ hasChildren ? (expanded ? '▼' : '▶') : '•' }}
      </span>
      <span class="node-title">{{ document.title }}</span>
      <span class="node-actions">
        <button class="action-btn" @click.stop="$emit('create-child', document.id)" title="添加子文档">
          +
        </button>
        <button class="action-btn delete" @click.stop="$emit('delete', document.id)" title="删除">
          ×
        </button>
      </span>
    </div>
    
    <div v-if="expanded && hasChildren" class="children">
      <TreeNode
        v-for="childId in document.children"
        :key="childId"
        :document="documents.get(childId)!"
        :level="level + 1"
        :active-id="activeId"
        :documents="documents"
        @select="(id) => $emit('select', id)"
        @create-child="(id) => $emit('create-child', id)"
        @delete="(id) => $emit('delete', id)"
        @move="(payload) => $emit('move', payload)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import type { Document } from '../stores/workspace'

const props = defineProps<{
  document: Document
  level: number
  activeId: string | null
  documents: Map<string, Document>
}>()

const emit = defineEmits<{
  select: [id: string]
  'create-child': [id: string]
  delete: [id: string]
  move: [payload: { id: string; targetParentId: string | null }]
}>()

const expanded = ref(true)
const isDragOver = ref(false)
const isDragging = ref(false)

const hasChildren = computed(() => props.document.children.length > 0)

function toggleExpand() {
  expanded.value = !expanded.value
}

// 拖拽相关
let draggedId: string | null = null

function onDragStart(e: DragEvent) {
  draggedId = props.document.id
  isDragging.value = true
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', props.document.id)
  }
}

function onDragEnd() {
  isDragging.value = false
  draggedId = null
}

function onDragOver(e: DragEvent) {
  if (draggedId && draggedId !== props.document.id) {
    isDragOver.value = true
  }
}

function onDragLeave() {
  isDragOver.value = false
}

function onDrop(e: DragEvent) {
  isDragOver.value = false
  
  if (e.dataTransfer) {
    const sourceId = e.dataTransfer.getData('text/plain')
    if (sourceId && sourceId !== props.document.id) {
      // 不能拖拽到自己的子文档中
      if (!isDescendant(sourceId, props.document.id)) {
        emit('move', { 
          id: sourceId, 
          targetParentId: props.document.id 
        })
      }
    }
  }
}

// 检查是否是后代节点
function isDescendant(parentId: string, childId: string): boolean {
  const parent = props.documents.get(parentId)
  if (!parent) return false
  
  for (const child of parent.children) {
    if (child === childId) return true
    if (isDescendant(child, childId)) return true
  }
  
  return false
}
</script>

<style scoped>
.tree-node {
  user-select: none;
}

.node-content {
  display: flex;
  align-items: center;
  padding: 4px 8px;
  border-radius: 4px;
  cursor: pointer;
  transition: background 0.2s;
}

.node-content:hover {
  background: #f0f0f0;
}

.tree-node.active > .node-content {
  background: #e8f4fd;
  color: var(--color-primary);
}

.tree-node.drag-over > .node-content {
  background: #e8f8e8;
  border: 2px dashed var(--color-primary);
}

.tree-node.dragging > .node-content {
  opacity: 0.5;
}

.expand-icon {
  width: 16px;
  font-size: 10px;
  color: var(--color-text-muted);
  cursor: pointer;
}

.expand-icon:hover {
  color: var(--color-text-secondary);
}

.node-title {
  flex: 1;
  margin-left: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
}

.node-actions {
  opacity: 0;
  transition: opacity 0.2s;
}

.node-content:hover .node-actions {
  opacity: 1;
}

.action-btn {
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 12px;
  color: var(--color-text-secondary);
  padding: 2px 4px;
  margin-left: 4px;
}

.action-btn:hover {
  color: var(--color-primary);
}

.action-btn.delete:hover {
  color: #dc3545;
}

.children {
  /* 子节点容器 */
}
</style>
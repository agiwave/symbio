<template>
  <div
    class="tree-node"
    :style="{ paddingLeft: `${level * 16 + 8}px` }"
    :class="{ active: document.id === activeId }"
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
      </span>
    </div>
    
    <div v-if="expanded && hasChildren" class="children">
      <TreeNode
        v-for="childId in document.children"
        :key="childId"
        :document="documents.get(childId)!"
        :level="level + 1"
        :active-id="activeId"
        @select="(id) => $emit('select', id)"
        @create-child="(id) => $emit('create-child', id)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, inject } from 'vue'
import type { Document } from '../stores/workspace'

const props = defineProps<{
  document: Document
  level: number
  activeId: string | null
}>()

defineEmits<{
  select: [id: string]
  'create-child': [id: string]
}>()

const documents = inject<Map<string, Document>>('documents')!

const expanded = ref(true)

const hasChildren = computed(() => props.document.children.length > 0)

function toggleExpand() {
  expanded.value = !expanded.value
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

.expand-icon {
  width: 16px;
  font-size: 10px;
  color: var(--color-text-muted);
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
}

.action-btn:hover {
  color: var(--color-primary);
}

.children {
  /* 子节点容器 */
}
</style>
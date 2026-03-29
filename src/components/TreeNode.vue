<template>
  <div class="tree-node" :style="{ paddingLeft: `${depth * 1.5}rem` }">
    <div 
      class="node-content"
      :class="{ selected }"
      @click="handleClick"
    >
      <span class="node-icon">{{ isExpandable ? (expanded ? '📂' : '📁') : '📄' }}</span>
      <span class="node-name">{{ node.name }}</span>
      <span v-if="isExpandable" class="expand-indicator">
        {{ expanded ? '▼' : '▶' }}
      </span>
    </div>
    
    <div v-if="expanded && isExpandable" class="node-children">
      <TreeNode 
        v-for="child in node.children"
        :key="child.name"
        :node="child"
        :path="[...path, child.name]"
        :selected="isSelected(child.name)"
        @select="handleChildSelect"
        :depth="depth + 1"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import type { PluginMeta } from '../types'

interface PluginHierarchy {
  name: string
  meta: PluginMeta
  children: PluginHierarchy[]
}

const props = defineProps<{
  node: PluginHierarchy
  path: string[]
  selected: boolean
  depth: number
}>()

const emit = defineEmits<{
  'select': [path: string[]]
}>()

const expanded = ref(false)

const isExpandable = computed(() => {
  return props.node.children && props.node.children.length > 0
})

const isSelected = (name: string) => {
  return props.selected || props.path.includes(name)
}

const handleClick = () => {
  if (isExpandable.value) {
    expanded.value = !expanded.value
  }
  emit('select', props.path)
}

const handleChildSelect = (path: string[]) => {
  emit('select', path)
}
</script>

<style scoped>
.tree-node {
  user-select: none;
}

.node-content {
  display: flex;
  align-items: center;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  cursor: pointer;
  transition: background-color 0.2s;
  margin: 0.25rem 0;
}

.node-content:hover {
  background-color: #f0f0f0;
}

.node-content.selected {
  background-color: #e3e7fc;
  color: #667eea;
  font-weight: 500;
}

.node-icon {
  margin-right: 0.5rem;
  font-size: 1.1rem;
}

.node-name {
  flex: 1;
  font-size: 0.9rem;
}

.expand-indicator {
  font-size: 0.7rem;
  color: #999;
  margin-left: 0.5rem;
}

.node-children {
  margin-top: 0.25rem;
}
</style>

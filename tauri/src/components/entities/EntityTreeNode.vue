<!--
  EntityTreeNode — 树视图节点（EntityTree 的递归子件，机制内置）

  纯结构渲染：展开箭头（expandable !== false）/ 图标（entity 注册表按
  kind 映射）/ 名称 / 选中态 / 子层级递归。不含任何场景语义。
-->
<template>
  <div class="tree-node">
    <div
      class="node-row"
      :class="{ active: node.id === selectedId }"
      :style="{ paddingLeft: `${depth * 0.875 + 0.375}rem` }"
      role="treeitem"
      :aria-selected="node.id === selectedId"
      :aria-expanded="node.expandable === false ? undefined : isExpanded(node.id)"
      @click="onClick"
    >
      <span class="chevron" :class="{ open: isExpanded(node.id), hidden: node.expandable === false }">
        ▸
      </span>
      <span class="node-icon" :title="node.name">
        <component :is="nodeIcon" v-if="nodeIcon" />
        <span v-else class="icon-dot" />
      </span>
      <span class="node-name" :title="node.id">{{ node.name }}</span>
      <span v-if="isLoading(node.id)" class="node-loading">…</span>
    </div>
    <template v-if="node.expandable !== false && isExpanded(node.id)">
      <EntityTreeNode
        v-for="child in childrenOf(node.id)"
        :key="child.id"
        :node="child"
        :depth="depth + 1"
        :children-of="childrenOf"
        :is-expanded="isExpanded"
        :is-loading="isLoading"
        :selected-id="selectedId"
        @toggle="(...args) => $emit('toggle', ...args)"
        @select="(...args) => $emit('select', ...args)"
      />
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { EntitySummary } from '@/schemas/entities'
import { getEntityIconFor } from '@/registry/entityTypes'

const props = defineProps<{
  node: EntitySummary
  depth: number
  /** 取某父路径下已加载的直接子节点（EntityTree 提供的响应式 getter） */
  childrenOf: (parentPath: string) => EntitySummary[]
  isExpanded: (path: string) => boolean
  isLoading: (path: string) => boolean
  selectedId?: string | null
}>()

const emit = defineEmits<{
  (e: 'toggle', node: EntitySummary): void
  (e: 'select', node: EntitySummary): void
}>()

const nodeIcon = computed(() => getEntityIconFor({ kind: props.node.kind }))

function onClick() {
  if (props.node.expandable !== false) emit('toggle', props.node)
  emit('select', props.node)
}

function isLoading(path: string): boolean {
  return props.isLoading(path)
}
</script>

<style scoped>
.node-row {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.2rem 0.375rem;
  border-radius: var(--radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--motion-fast) var(--motion-ease);
}
.node-row:hover { background: var(--surface-hover); }
.node-row.active { background: var(--accent-subtle-bg); }

.chevron {
  flex-shrink: 0;
  font-size: 0.65rem;
  color: var(--text-muted);
  transition: transform var(--motion-fast) var(--motion-ease);
  width: 0.75rem;
  text-align: center;
}
.chevron.open { transform: rotate(90deg); }
.chevron.hidden { visibility: hidden; }

.node-icon {
  flex-shrink: 0;
  display: inline-flex;
  color: var(--text-secondary);
}
.icon-dot {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: var(--radius-full);
  background: var(--border-strong);
  display: inline-block;
}
.node-name {
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-primary);
}
.node-loading {
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: 0.7rem;
}
</style>

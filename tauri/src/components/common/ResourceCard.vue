<!--
  ResourceCard — 通用"列表项"卡片

  适用于所有 ResourceShell 的左侧列表：Model Provider / MCP Server / Skill / Agent。

  视觉风格对齐 SessionCard：
  - 状态点 + 标题行
  - 副标题（描述预览）
  - 元信息行（来源/类型/其他标签）
-->
<template>
  <div
    class="resource-card"
    :class="{ active: isActive, disabled }"
    @click="$emit('click')"
  >
    <!-- 状态条 + 标题 -->
    <div class="card-header">
      <div class="status-area">
        <span :class="['status-dot', statusClass]" :title="statusTitle" />
      </div>
      <div class="title-row">
        <span class="title-text" :title="title">{{ title }}</span>
        <span v-if="badge" class="activity-text" :class="badgeClass">{{ badge }}</span>
      </div>
    </div>

    <!-- 副标题（描述） -->
    <div v-if="subtitle" class="card-preview">
      <span class="preview-text">{{ subtitle }}</span>
    </div>

    <!-- 元信息行 -->
    <div v-if="$slots.meta || tags.length" class="card-meta">
      <slot name="meta" />
      <template v-for="(tag, i) in tags" :key="i">
        <span class="tag" :class="`tag-${tag.kind || 'default'}`">{{ tag.label }}</span>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

interface Tag {
  label: string
  kind?: 'default' | 'primary' | 'success' | 'warn' | 'info' | 'muted'
}

interface ResourceCardProps {
  title: string
  subtitle?: string
  status?: 'active' | 'disabled' | 'warning' | 'error' | 'muted'
  statusTitle?: string
  badge?: string
  badgeKind?: 'default' | 'primary' | 'success' | 'warn' | 'info'
  isActive?: boolean
  disabled?: boolean
  tags?: Tag[]
}

const props = withDefaults(defineProps<ResourceCardProps>(), {
  status: 'active',
  statusTitle: '',
  badgeKind: 'default',
  isActive: false,
  disabled: false,
  tags: () => [],
})

defineEmits<{
  (e: 'click'): void
}>()

const statusClass = computed(() => `status-${props.status}`)
const badgeClass = computed(() => `kind-${props.badgeKind}`)
</script>

<style scoped>
.resource-card {
  margin: 0.25rem 0.5rem;
  padding: 0.55rem 0.65rem;
  background: var(--color-surface, #fff);
  border: 1px solid var(--color-border, #e5e7eb);
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.15s ease;
  user-select: none;
}

.resource-card:hover {
  border-color: var(--color-primary, #667eea);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.05);
}

.resource-card.active {
  border-color: var(--color-primary, #667eea);
  background: rgba(102, 126, 234, 0.05);
}

.resource-card.disabled {
  opacity: 0.55;
}

/* 状态条 + 标题 */
.card-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.25rem;
}

.status-area {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.status-dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #9ca3af;
  flex-shrink: 0;
}
.status-dot.status-active { background: #22c55e; }
.status-dot.status-disabled { background: #9ca3af; }
.status-dot.status-warning { background: #f59e0b; }
.status-dot.status-error { background: #ef4444; }
.status-dot.status-muted { background: #d1d5db; }

.title-row {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  min-width: 0;
}

.title-text {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--color-text, #1f2937);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}

.activity-text {
  font-size: 0.65rem;
  color: var(--color-text-muted, #6b7280);
  flex-shrink: 0;
}
.activity-text.kind-primary { color: var(--color-primary, #667eea); }
.activity-text.kind-success { color: #22c55e; }
.activity-text.kind-warn { color: #f59e0b; }
.activity-text.kind-info { color: #3b82f6; }

/* 副标题 */
.card-preview {
  margin: 0.2rem 0;
  font-size: 0.75rem;
  color: var(--color-text-muted, #6b7280);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 元信息 */
.card-meta {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  flex-wrap: wrap;
  margin-top: 0.3rem;
  font-size: 0.7rem;
  color: var(--color-text-muted, #6b7280);
}

.tag {
  padding: 0.1rem 0.4rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 4px;
  font-size: 0.65rem;
}
.tag-primary { background: rgba(102, 126, 234, 0.1); color: var(--color-primary, #667eea); }
.tag-success { background: rgba(34, 197, 94, 0.1); color: #22c55e; }
.tag-warn    { background: rgba(245, 158, 11, 0.1); color: #f59e0b; }
.tag-info    { background: rgba(59, 130, 246, 0.1); color: #3b82f6; }
.tag-muted   { background: rgba(0, 0, 0, 0.04); color: #6b7280; }
</style>

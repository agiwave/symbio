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
    role="option"
    :tabindex="disabled ? -1 : 0"
    :aria-selected="isActive"
    :aria-disabled="disabled"
    :aria-label="title"
    @click="$emit('click')"
    @keydown.enter="$emit('click')"
    @keydown.space.prevent="$emit('click')"
  >
    <!-- 状态条 + 类型图标 + 标题 -->
    <div class="card-header">
      <div class="status-area">
        <span :class="['status-dot', statusClass]" :title="statusTitle" />
      </div>
      <component v-if="icon" :is="icon" class="card-icon" />
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
import { computed, type Component } from 'vue'

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
  /** 类型图标组件（资源注册表下发，如设置分区图标） */
  icon?: Component
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
  margin: var(--space-1) var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background-color var(--motion-fast) var(--motion-ease),
    border-color var(--motion-fast) var(--motion-ease),
    box-shadow var(--motion-fast) var(--motion-ease);
  user-select: none;
}

.resource-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow-1);
}

.resource-card.active {
  border-color: var(--accent);
  background: var(--surface-selected);
}

.resource-card.disabled {
  opacity: 0.55;
}

/* 状态条 + 标题 */
.card-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-1);
}

.status-area {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.status-dot {
  display: inline-block;
  width: 0.4375rem;
  height: 0.4375rem;
  border-radius: var(--radius-full);
  background: var(--text-disabled);
  flex-shrink: 0;
}
.status-dot.status-active { background: var(--success-solid); }
.status-dot.status-disabled { background: var(--border-strong); }
.status-dot.status-warning { background: var(--warning-solid); }
.status-dot.status-error { background: var(--danger-solid); }
.status-dot.status-muted { background: var(--text-disabled); }

/* 类型图标 */
.card-icon {
  flex-shrink: 0;
  color: var(--text-secondary);
}

.title-row {
  flex: 1;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}

.title-text {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}

.activity-text {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  flex-shrink: 0;
}
.activity-text.kind-primary { color: var(--accent); }
.activity-text.kind-success { color: var(--success-solid); }
.activity-text.kind-warn { color: var(--warning-solid); }
.activity-text.kind-info { color: var(--info-solid); }

/* 副标题 */
.card-preview {
  margin: var(--space-1) 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 元信息 */
.card-meta {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
  margin-top: var(--space-1);
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

.tag {
  padding: 0.1rem 0.4rem;
  background: var(--surface-sunken);
  border-radius: var(--radius-sm);
  font-size: 0.65rem;
}
.tag-primary { background: var(--accent-subtle-bg); color: var(--accent); }
.tag-success { background: var(--success-bg); color: var(--success-fg); }
.tag-warn    { background: var(--warning-bg); color: var(--warning-fg); }
.tag-info    { background: var(--info-bg); color: var(--info-fg); }
.tag-muted   { background: var(--surface-sunken); color: var(--text-muted); }
</style>

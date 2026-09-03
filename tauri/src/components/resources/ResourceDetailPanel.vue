<!--
  ResourceDetailPanel — 统一资源详情（通用展示）

  展示 ResourceSummary 的公共字段 + extra 扩展字段。
  作为泛型 ResourceManagerView 的默认详情实现；对需要复杂表单/编辑的类型，
  泛型视图会优先渲染注册的专属 detail 组件，此处仅作通用回退。
-->
<template>
  <div v-if="item" class="resource-detail">
    <header class="detail-header">
      <h2 class="detail-title">{{ displayName }}</h2>
      <span class="detail-status" :class="`status-${item.status}`" :title="item.status_detail">
        {{ statusLabel }}
      </span>
    </header>

    <p v-if="item.description" class="detail-description">{{ item.description }}</p>
    <p v-else-if="item.summary" class="detail-description muted">{{ item.summary }}</p>

    <div class="detail-section">
      <label>名称（ID）</label>
      <code class="detail-code">{{ item.id }}</code>
    </div>

    <div v-if="item.updated_at" class="detail-section">
      <label>更新时间</label>
      <code class="detail-code">{{ formatTime(item.updated_at) }}</code>
    </div>

    <template v-if="extraEntries.length">
      <div v-for="[k, v] in extraEntries" :key="k" class="detail-section">
        <label>{{ k }}</label>
        <code class="detail-code mono">{{ formatValue(v) }}</code>
      </div>
    </template>
  </div>
  <div v-else class="no-selection">
    <p>← 选择一个资源查看详情</p>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ResourceSummary } from '@/schemas/resources'

const props = defineProps<{ item: ResourceSummary | null }>()

const STATUS_LABELS: Record<string, string> = {
  active: '可用',
  disabled: '已停用',
  working: '工作中',
  error: '异常',
  unknown: '未知',
}

const displayName = computed(() => props.item?.name || props.item?.id || '')

const statusLabel = computed(() => STATUS_LABELS[props.item?.status || 'unknown'] || props.item?.status)

const EXTRA_SKIP = new Set([
  'kind', 'name', 'id', 'description', 'summary', 'updated_at', 'status', 'status_detail',
])

const extraEntries = computed<Array<[string, unknown]>>(() => {
  const it = props.item
  if (!it) return []
  const out: Array<[string, unknown]> = []
  for (const [k, v] of Object.entries(it)) {
    if (EXTRA_SKIP.has(k)) continue
    if (v === null || v === undefined || v === '') continue
    out.push([k, v])
  }
  return out
})

function formatTime(sec: number): string {
  try {
    return new Date(sec * 1000).toLocaleString()
  } catch {
    return String(sec)
  }
}

function formatValue(v: unknown): string {
  if (typeof v === 'object') return JSON.stringify(v, null, 2)
  return String(v)
}
</script>

<style scoped>
.resource-detail {
  flex: 1;
  padding: 1.5rem 2rem;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.detail-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--color-border, #e5e7eb);
}
.detail-title {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0;
  color: var(--color-text, #1f2937);
}
.detail-status {
  font-size: 0.7rem;
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
  background: var(--color-bg-subtle, #f3f4f6);
  color: var(--color-text-muted, #6b7280);
}
.detail-status.status-active { color: #16a34a; }
.detail-status.status-disabled { color: #6b7280; }
.detail-status.status-working { color: #2563eb; }
.detail-status.status-error { color: #dc2626; }
.detail-description {
  font-size: 0.95rem;
  color: var(--color-text, #1f2937);
  margin: 0;
  line-height: 1.5;
  white-space: pre-wrap;
}
.detail-description.muted { color: var(--color-text-muted, #6b7280); }
.detail-section { display: flex; flex-direction: column; gap: 0.4rem; }
.detail-section label {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--color-text-muted, #6b7280);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.detail-code {
  display: block;
  padding: 0.5rem 0.75rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 0.375rem;
  font-size: 0.8rem;
  font-family: 'Menlo', 'Monaco', monospace;
  word-break: break-all;
  color: var(--color-text, #1f2937);
  white-space: pre-wrap;
}
.no-selection {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted, #6b7280);
  font-size: 0.9rem;
}
</style>
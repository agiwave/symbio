<!--
  ResourceDetailPanel — 统一资源详情（通用展示兜底）

  展示 ResourceSummary 的公共字段 + extra 扩展字段。
  作为 ResourceManagerView 的通用详情兜底；专属表单类型（如 model）
  由视图的 FORM_COMPONENTS 注册表接管，不经过本面板。
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
  connected: '已连接',
  disabled: '已停用',
  working: '工作中',
  error: '异常',
  failed: '异常',
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
  border-bottom: 1px solid var(--border-default);
}
.detail-title {
  font-size: var(--font-size-xl);
  font-weight: var(--font-weight-semibold);
  margin: 0;
  color: var(--text-primary);
}
.detail-status {
  font-size: 0.7rem;
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-full);
  background: var(--surface-sunken);
  color: var(--text-muted);
}
.detail-status.status-active,
.detail-status.status-connected { color: var(--success-fg); }
.detail-status.status-disabled { color: var(--text-muted); }
.detail-status.status-working { color: var(--info-fg); }
.detail-status.status-error,
.detail-status.status-failed { color: var(--danger-fg); }
.detail-description {
  font-size: 0.95rem;
  color: var(--text-primary);
  margin: 0;
  line-height: var(--line-height-normal);
  white-space: pre-wrap;
}
.detail-description.muted { color: var(--text-muted); }
.detail-section { display: flex; flex-direction: column; gap: 0.4rem; }
.detail-section label {
  font-size: var(--font-size-xs);
  font-weight: var(--font-weight-semibold);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.detail-code {
  display: block;
  padding: 0.5rem 0.75rem;
  background: var(--surface-sunken);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  font-size: 0.8rem;
  font-family: var(--font-mono);
  word-break: break-all;
  color: var(--text-primary);
  white-space: pre-wrap;
}
.no-selection {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 0.9rem;
}
</style>

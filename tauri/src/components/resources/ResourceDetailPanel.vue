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

    <div class="detail-section">
      <label>资源路径</label>
      <div class="path-row">
        <code class="detail-code path-code">{{ resourcePathLabel }}</code>
        <button class="copy-btn" title="复制路径" @click="copyPath">复制</button>
      </div>
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
import { resourcePath } from '@/registry/resourceTypes'
import { useToast } from '@/composables/useToast'

const props = defineProps<{ item: ResourceSummary | null }>()

const toast = useToast()

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
  'kind', 'provider', 'name', 'id', 'description', 'summary', 'updated_at', 'status', 'status_detail',
])

/** 资源路径唯一标识：[provider]/[id].[kind]（provider 缺省回退 kind） */
const resourcePathLabel = computed(() => {
  const it = props.item
  if (!it) return ''
  return resourcePath(it.provider || it.kind, it.id, it.kind)
})

async function copyPath() {
  if (!resourcePathLabel.value) return
  try {
    await navigator.clipboard.writeText(resourcePathLabel.value)
    toast.showToast('success', '已复制资源路径')
  } catch {
    // clipboard 不可用时展示路径文本供手抄
    toast.showToast('info', `路径：${resourcePathLabel.value}`)
  }
}

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
.path-row {
  display: flex;
  align-items: stretch;
  gap: 0.4rem;
}
.path-row .path-code {
  flex: 1;
}
.copy-btn {
  flex-shrink: 0;
  align-self: center;
  padding: 0.3rem 0.75rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  cursor: pointer;
  transition: all var(--motion-fast) var(--motion-ease);
}
.copy-btn:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
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

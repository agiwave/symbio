<!--
  Agent — Agent（OAB bundle）专属详情

  统一资源协议下的"详情差异化"组件，经项级 editor 注册表分发
  （registerResourceEditor('agent:bundle', ...)，按 item.config_type 命中；
  kind 级不注册，agent 的 zip 上传新建流程不受影响）。

  职责：bundle 概览（版本/规格/来源/目录）+ 三类内部资源计数 +
  「管理资源」入口按钮——点击后**全局路由推入**统一 WorkbenchView
  （/container/:kind/:id/resources，与主界面同构的侧边栏+列表+详情整页），
  资源管理逻辑全部在页面侧（useWorkbenchView 组合式），本组件只读概览
  （useContainerOverview：类别 + 计数）。
-->
<template>
  <div v-if="item" class="agent-detail">
    <header class="form-header">
      <div class="title-area">
        <h2 class="title-text">{{ item.name || item.id }}</h2>
        <span v-if="version" class="badge">v{{ version }}</span>
        <span class="badge scope">{{ scope === 'workspace' ? '工作区级' : '全局级' }}</span>
      </div>
      <div class="header-actions">
        <button type="button" class="action-btn" @click="openResources">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
          </svg>
          管理资源（提示词 / 技能 / MCP）
        </button>
        <button
          type="button"
          class="icon-btn danger"
          title="删除该 Agent"
          :disabled="deleting"
          @click="$emit('delete')"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="3 6 5 6 21 6" />
            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          </svg>
        </button>
      </div>
    </header>

    <div class="detail-body">
      <p v-if="item.description" class="detail-description">{{ item.description }}</p>

      <div class="overview-grid">
        <div class="detail-section">
          <label>名称（ID）</label>
          <code class="detail-code">{{ item.id }}</code>
        </div>
        <div v-if="version" class="detail-section">
          <label>版本</label>
          <code class="detail-code">{{ version }}</code>
        </div>
        <div class="detail-section">
          <label>来源层级</label>
          <code class="detail-code">{{ scope === 'workspace' ? '工作区级（.symbio/plugins/agent）' : '全局级（系统目录）' }}</code>
        </div>
        <div v-if="dir" class="detail-section">
          <label>安装目录</label>
          <code class="detail-code mono">{{ dir }}</code>
        </div>
      </div>

      <div class="detail-section">
        <label>内部资源概览</label>
        <div class="count-row">
          <button
            v-for="k in containerKinds"
            :key="k.kind"
            type="button"
            class="count-chip"
            :title="`管理${k.label}`"
            @click="openResources"
          >
            <span class="count-num">{{ counts[k.kind] ?? 0 }}</span>
            <span class="count-label">{{ k.label }}</span>
          </button>
        </div>
        <p v-if="entriesError" class="load-error">{{ entriesError }}</p>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import type { ResourceCapabilities, ResourceSummary } from '@/schemas/resources'
import { useContainerOverview } from '@/composables/useWorkbenchView'

const props = defineProps<{
  item: ResourceSummary | null
  capabilities: ResourceCapabilities
  saving?: boolean
  testing?: boolean
  deleting?: boolean
}>()

defineEmits<{
  (e: 'delete'): void
}>()

const router = useRouter()

// === bundle 概览字段（extra flatten 下发） ===
const version = computed(() => (props.item?.version as string) || '')
const scope = computed(() => (props.item?.scope as string) || 'global')
const dir = computed(() => (props.item?.dir as string) || '')
const bundleId = computed(() => props.item?.id || '')

// === 资源概览：只读（类别 + 计数来自 useContainerOverview；管理逻辑在 WorkbenchView 页面侧） ===
const { containerKinds, counts, entriesError, loadEntries } = useContainerOverview(
  'agent',
  () => bundleId.value
)

/** 全局路由推入容器资源管理页（整页替换，非详情内切换） */
function openResources() {
  router.push({
    name: 'container-resources',
    params: { kind: 'agent', id: bundleId.value },
  })
}

onMounted(() => {
  loadEntries()
})
</script>

<style scoped>
.agent-detail {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

.form-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
}
.title-area {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}
.title-text {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.badge {
  font-size: 0.65rem;
  padding: 0.1rem 0.45rem;
  border-radius: var(--radius-full);
  background: var(--accent-subtle-bg);
  color: var(--accent);
  flex-shrink: 0;
}
.badge.scope {
  background: var(--surface-sunken);
  color: var(--text-muted);
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-shrink: 0;
}

.detail-body {
  flex: 1;
  overflow-y: auto;
  padding: 1.25rem 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.detail-description {
  margin: 0;
  font-size: 0.92rem;
  color: var(--text-primary);
  line-height: var(--line-height-normal);
  white-space: pre-wrap;
}
.overview-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
  gap: 0.75rem;
}
.detail-section {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}
.detail-section label {
  font-size: var(--font-size-xs);
  font-weight: var(--font-weight-semibold);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.detail-code {
  padding: 0.4rem 0.6rem;
  background: var(--surface-sunken);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  font-size: 0.78rem;
  font-family: var(--font-mono);
  word-break: break-all;
  color: var(--text-primary);
}
.mono { font-family: var(--font-mono); }

.count-row {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
}
.count-chip {
  display: flex;
  align-items: baseline;
  gap: 0.4rem;
  padding: 0.5rem 0.9rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-overlay);
  cursor: pointer;
  transition: border-color var(--motion-fast) var(--motion-ease),
    background var(--motion-fast) var(--motion-ease);
}
.count-chip:hover {
  border-color: var(--accent);
  background: var(--surface-hover);
}
.count-num {
  font-size: 1.1rem;
  font-weight: var(--font-weight-semibold);
  color: var(--accent);
}
.count-label {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}
.load-error {
  margin: 0;
  color: var(--danger-fg);
  font-size: 0.8rem;
}
</style>

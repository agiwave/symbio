<!--
  McpServerCard — 左侧 MCP Server 列表中的"卡片"项

  视觉风格对齐 ModelProviderCard / SessionCard：
  - 状态点（启用 / 停用）+ 标题行
  - 预览行：显示 command 或 URL
  - 元信息行：参数数量 + 环境变量数量

  说明：删除入口已上移到详情页头部，本卡片只承担浏览 / 选中交互。
-->
<template>
  <div
    class="server-card"
    :class="{
      active: isActive,
      disabled: !server.enabled
    }"
    @click="onClick"
  >
    <!-- 状态条 + 标题 -->
    <div class="card-header">
      <div class="status-area">
        <span
          :class="['status-dot', dotClass]"
          :title="dotTitle"
        />
      </div>
      <div class="title-row">
        <span class="title-text" :title="name">
          {{ name }}
        </span>
        <span class="transport-badge" :class="`transport-${transportType}`">
          {{ transportType }}
        </span>
        <span v-if="!server.enabled" class="activity-text failed">已停用</span>
        <span v-else class="activity-text">点击查看 / 编辑</span>
      </div>
    </div>

    <!-- 预览行：显示 command 或 URL -->
    <div class="card-preview" v-if="previewText">
      <span class="preview-text">{{ previewText }}</span>
    </div>

    <!-- 元信息 -->
    <div class="card-meta" v-if="transportType === 'stdio'">
      <span class="meta-pill">
        <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2">
          <polyline points="4 17 10 11 4 5" />
          <line x1="12" y1="19" x2="20" y2="19" />
        </svg>
        {{ argsCount }} 参数
      </span>
      <span class="dot" v-if="envCount > 0">·</span>
      <span class="meta-pill" v-if="envCount > 0">
        <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3" />
        </svg>
        {{ envCount }} 环境变量
      </span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { McpServerConfig } from '@/schemas/mcp_config'

const props = defineProps<{
  name: string
  server: McpServerConfig
  isActive: boolean
}>()

const emit = defineEmits<{
  click: []
}>()

/** 当前 transport 类型，缺省 stdio */
const transportType = computed(() => props.server.type ?? 'stdio')

/** 预览行展示的简要标识：stdio 用 command；http/sse 用 url */
const previewText = computed(() => {
  if (transportType.value === 'stdio') {
    return props.server.command || ''
  }
  return props.server.url || ''
})

const argsCount = computed(() => props.server.args?.length ?? 0)
const envCount = computed(() => Object.keys(props.server.env ?? {}).length)

const dotClass = computed(() => {
  return props.server.enabled ? 'running' : 'failed'
})

const dotTitle = computed(() => {
  return props.server.enabled ? '可用' : '已停用'
})

function onClick() {
  emit('click')
}
</script>

<style scoped>
.server-card {
  position: relative;
  padding: 0.6rem 0.75rem 0.55rem;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
  border-left: 2px solid transparent;
  border-bottom: 1px solid var(--color-border-subtle, rgba(0, 0, 0, 0.04));
  user-select: none;
}

.server-card:hover {
  background: rgba(0, 0, 0, 0.03);
}

.server-card.active {
  background: rgba(102, 126, 234, 0.08);
  border-left-color: var(--color-primary, #667eea);
}

.server-card.disabled {
  opacity: 0.55;
}

/* 状态条 + 标题 */
.card-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}

.status-area {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.status-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #94a3b8;
  flex-shrink: 0;
}

.status-dot.running {
  background: #22c55e;
  box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.4);
  animation: pulse 1.6s ease-in-out infinite;
}

.status-dot.failed {
  background: #94a3b8;
}

@keyframes pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.4); }
  50% { box-shadow: 0 0 0 4px rgba(34, 197, 94, 0); }
}

.title-row {
  display: flex;
  align-items: baseline;
  gap: 0.4rem;
  min-width: 0;
  flex: 1;
}

.title-text {
  font-size: 0.85rem;
  font-weight: 500;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
  max-width: 60%;
}

.activity-text {
  font-size: 0.7rem;
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.activity-text.failed {
  color: var(--color-text-muted);
}

/* 预览行 */
.card-preview {
  margin-top: 0.3rem;
  padding-left: 1rem;
}

.preview-text {
  font-size: 0.75rem;
  color: var(--color-text-secondary);
  display: -webkit-box;
  -webkit-line-clamp: 1;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-all;
  font-family: var(--font-mono, 'JetBrains Mono', Consolas, monospace);
}

/* 元信息 */
.card-meta {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  margin-top: 0.35rem;
  padding-left: 1rem;
  font-size: 0.7rem;
  color: var(--color-text-muted);
  min-width: 0;
  overflow: hidden;
}

.meta-pill {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0 0.35rem;
  background: rgba(0, 0, 0, 0.05);
  border-radius: 3px;
  font-size: 0.65rem;
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.dot {
  opacity: 0.5;
}

/* Transport 徽章 */
.transport-badge {
  display: inline-block;
  padding: 0 0.4rem;
  font-size: 0.6rem;
  font-weight: 500;
  border-radius: 3px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  background: rgba(102, 126, 234, 0.12);
  color: #667eea;
  flex-shrink: 0;
}

.transport-badge.transport-http {
  background: rgba(34, 197, 94, 0.12);
  color: #16a34a;
}

.transport-badge.transport-sse {
  background: rgba(245, 158, 11, 0.12);
  color: #d97706;
}
</style>

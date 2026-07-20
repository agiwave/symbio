<!--
  ModelProviderCard — 左侧 Provider 列表中的"卡片"项

  视觉风格对齐 SessionCard：
  - 状态点（默认 / 启用 / 停用 / 错误）+ 标题行
  - 模型预览行
  - 元信息行（provider · 协议）

  说明：删除入口已上移到详情页头部，本卡片只承担浏览 / 选中交互。
-->
<template>
  <div
    class="provider-card"
    :class="{
      active: isActive,
      disabled: !provider.enabled,
      default: isDefault
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
        <span class="title-text" :title="title">
          {{ title }}
        </span>
        <span v-if="isDefault" class="activity-text">默认 Provider</span>
        <span v-else-if="!provider.enabled" class="activity-text failed">已停用</span>
        <span v-else class="activity-text">点击查看 / 编辑</span>
      </div>
    </div>

    <!-- 预览行：显示 provider + model -->
    <div class="card-preview" v-if="hasPreview">
      <span class="preview-text">{{ preview }}</span>
    </div>

    <!-- 元信息 -->
    <div class="card-meta">
      <span class="provider-pill">{{ provider.provider }}</span>
      <span class="dot">·</span>
      <span class="protocol">{{ protocolLabel }}</span>
      <span v-if="(provider.rate_limit_ms ?? 0) > 0" class="dot">·</span>
      <span v-if="(provider.rate_limit_ms ?? 0) > 0" class="rate" :title="`最小间隔 ${provider.rate_limit_ms}ms`">
        ⏱ {{ provider.rate_limit_ms }}ms
      </span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ModelProviderConfig } from '@/schemas/model_providers'
import { protocolLabels } from '@/constants/modelProviders'

const props = defineProps<{
  provider: ModelProviderConfig
  isActive: boolean
  isDefault: boolean
}>()

const emit = defineEmits<{
  click: []
}>()

const title = computed(() => props.provider.name || props.provider.id || '未命名')
const hasPreview = computed(() => !!props.provider.model)
const preview = computed(() => props.provider.model || '')

const protocolLabel = computed(
  () => protocolLabels[(props.provider as any).api_protocol] || props.provider.api_protocol || 'openai_responses'
)

const dotClass = computed(() => {
  if (!props.provider.enabled) return 'failed'
  if (props.isDefault) return 'running'
  return 'idle'
})

const dotTitle = computed(() => {
  if (!props.provider.enabled) return '已停用'
  if (props.isDefault) return '默认 Provider'
  return '可用'
})

function onClick() {
  emit('click')
}
</script>

<style scoped>
.provider-card {
  position: relative;
  padding: 0.6rem 0.75rem 0.55rem;
  cursor: pointer;
  transition: background 0.12s ease, border-color 0.12s ease;
  border-left: 2px solid transparent;
  border-bottom: 1px solid var(--color-border-subtle, rgba(0, 0, 0, 0.04));
  user-select: none;
}

.provider-card:hover {
  background: rgba(0, 0, 0, 0.03);
}

.provider-card.active {
  background: rgba(34, 197, 94, 0.06);
  border-left-color: #22c55e;
}

.provider-card.disabled {
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

.status-dot.idle {
  background: #94a3b8;
}

.status-dot.failed {
  background: #ef4444;
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
  color: #ef4444;
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

.provider-pill {
  display: inline-block;
  padding: 0 0.35rem;
  background: rgba(0, 0, 0, 0.05);
  border-radius: 3px;
  font-size: 0.65rem;
  color: var(--color-text-secondary);
  font-family: var(--font-mono, 'JetBrains Mono', Consolas, monospace);
  white-space: nowrap;
}

.protocol,
.rate {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dot {
  opacity: 0.5;
}
</style>
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
    role="option"
    tabindex="0"
    :aria-selected="isActive"
    :aria-disabled="!provider.enabled"
    :aria-label="title"
    @click="onClick"
    @keydown.enter="onClick"
    @keydown.space.prevent="onClick"
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
  transition: background-color var(--motion-fast) var(--motion-ease),
    border-color var(--motion-fast) var(--motion-ease);
  border-left: 0.125rem solid transparent;
  border-bottom: 1px solid var(--border-subtle);
  user-select: none;
}

.provider-card:hover {
  background: var(--surface-hover);
}

.provider-card.active {
  background: var(--surface-selected);
  border-left-color: var(--accent);
}

.provider-card.disabled {
  opacity: 0.55;
}

/* 状态条 + 标题 */
.card-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
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
  width: 0.5rem;
  height: 0.5rem;
  border-radius: var(--radius-full);
  background: var(--text-disabled);
  flex-shrink: 0;
}

.status-dot.running {
  background: var(--success-solid);
  animation: pulse 1.6s ease-in-out infinite;
}

.status-dot.idle { background: var(--text-disabled); }
.status-dot.disabled { background: var(--border-strong); }
.status-dot.failed { background: var(--danger-solid); }

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.title-row {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  min-width: 0;
  flex: 1;
}

.title-text {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
  max-width: 60%;
}

.activity-text {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.activity-text.failed {
  color: var(--danger-solid);
}

/* 预览行 */
.card-preview {
  margin-top: var(--space-1);
  padding-left: 1rem;
}

.preview-text {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
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
  gap: var(--space-1);
  margin-top: var(--space-1);
  padding-left: 1rem;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  min-width: 0;
  overflow: hidden;
}

.provider-pill {
  display: inline-block;
  padding: 0 var(--space-1);
  background: var(--surface-sunken);
  border-radius: var(--radius-sm);
  font-size: 0.65rem;
  color: var(--text-secondary);
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
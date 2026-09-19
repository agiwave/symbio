<!--
  失败 / 中止交代条 —— 「这一轮（或这个工具）没跑完」的统一呈现

  三处消费：Turn 组级错误条、工具级错误条、工具结果的失败体。同一套观感与
  「重试」入口，因此只有一份实现——原先这三处各写了一遍 `.error-box` 结构。

  中止（`aborted`）**不是故障**：图标与配色都与失败区分开（中性色而非故障红），
  但**重试入口照给**——这一轮只跑了一半，用户中止后通常就想重跑它。
-->
<template>
  <div class="error-box" :class="{ aborted, 'turn-error': variant === 'turn' }">
    <span class="err-icon">{{ aborted ? '⏹' : '⚠' }}</span>
    <span class="err-text">{{ aborted ? MESSAGE_ABORTED_NOTE : text }}</span>
    <button v-if="retryable" class="retry" @click="emit('retry')">重试</button>
  </div>
</template>

<script setup lang="ts">
import { MESSAGE_ABORTED_NOTE } from '@/registry/messageTypes'

withDefaults(
  defineProps<{
    /** 失败原因（已由 `messageErrorTextOf` 兜底过，不会为空） */
    text: string
    /** 用户主动终止（非故障） */
    aborted?: boolean
    /** 是否给出「重试」入口 */
    retryable?: boolean
    /** 组级错误条（Turn）：需要额外的上下间距 */
    variant?: 'plain' | 'turn'
  }>(),
  { aborted: false, retryable: false, variant: 'plain' },
)

const emit = defineEmits<{ retry: [] }>()
</script>

<style scoped>
.error-box {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--color-error-bg);
  border: 1px solid var(--color-error-border);
  border-radius: 0.5rem;
  padding: 0.45rem 0.6rem;
  font-size: 0.8rem;
  color: var(--color-error-fg);
}
.err-icon {
  flex-shrink: 0;
}
.err-text {
  flex: 1;
  word-break: break-word;
}
/* 失败 Turn 的错误条（error-box + turn-error 组合）：与上下文留出间距 */
.turn-error {
  margin: 0.25rem 0 0.375rem;
}
/* 中止不是失败：中性配色，只交代"这一轮没跑完"，不渲染成故障红。 */
.error-box.aborted {
  background: var(--color-option-bg);
  border-color: var(--border-default);
  color: var(--text-secondary);
}
.error-box.aborted .retry {
  background: transparent;
  border-color: var(--border-default);
  color: var(--text-secondary);
}
.retry {
  flex-shrink: 0;
  background: var(--color-option-bg);
  border: 1px solid var(--color-error-border);
  color: var(--color-error-fg);
  border-radius: 0.375rem;
  padding: 0.2rem 0.6rem;
  font-size: 0.76rem;
  cursor: pointer;
}
</style>

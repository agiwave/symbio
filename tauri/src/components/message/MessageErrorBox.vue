<!--
  失败 / 中止交代条 —— 「这一轮（或这个工具）没跑完」的统一呈现

  四处消费：Turn 组级错误条、工具级错误条、工具结果的失败体、**会话级错误条**
  （`ModelChatPanel` 的流尾兜底）。同一套观感与「重试」入口，因此只有一份实现——
  原先这几处各写了一遍 `.error-box` 结构。

  会话级那处是**唯一带关闭按钮的**（`dismissible`）：节点级错误由节点状态承载，
  「关掉它」只是把状态藏起来，下一帧重渲染又回来；而会话级兜底错误没有节点可依，
  用户需要一个方式清掉它（它本身也会在下一轮开始时被节点状态覆盖，见
  `sessions.ts::applySessionNode`）。

  中止（`aborted`）**不是故障**：图标与配色都与失败区分开（中性色而非故障红），
  但**重试入口照给**——这一轮只跑了一半，用户中止后通常就想重跑它。
-->
<template>
  <div class="error-box" :class="{ aborted, 'turn-error': variant === 'turn' }">
    <span class="err-icon">{{ aborted ? '⏹' : '⚠' }}</span>
    <span class="err-text">{{ aborted ? MESSAGE_ABORTED_NOTE : text }}</span>
    <button v-if="retryable" class="retry" @click="emit('retry')">重试</button>
    <button v-if="dismissible" class="dismiss" title="关闭" @click="emit('dismiss')">×</button>
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
    /** 是否给出「关闭」入口（仅会话级兜底错误需要，见文件头注） */
    dismissible?: boolean
  }>(),
  { aborted: false, retryable: false, variant: 'plain', dismissible: false },
)

const emit = defineEmits<{ retry: []; dismiss: [] }>()
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
/* 关闭（×）：与「重试」同排，但视觉上更低一级——它不是主操作。 */
.dismiss {
  flex-shrink: 0;
  background: transparent;
  border: none;
  color: var(--color-error-fg);
  font-size: 1.1rem;
  line-height: 1;
  cursor: pointer;
  padding: 0 0.25rem;
  border-radius: 0.25rem;
  opacity: 0.7;
}
.dismiss:hover {
  opacity: 1;
}
</style>

<!--
  Toast.vue — 全局浮动消息浮层

  全站唯一实例，挂在 MainLayout 中。消费 useToast() 的单例状态，
  取代原先散落在各资源页的 4 套本地 toast 实现。

  视觉全部令牌化（浅色/深色均正确）：
  - 浮层底色：--surface-overlay
  - 描边：--border-default，左侧语义强调条按 type 取 solid 色
  - 阴影：--shadow-2（深色下自动转为深底描边感）
  点击浮层可提前关闭；aria-live 便于读屏播报。
-->
<template>
  <Teleport to="body">
    <div class="toast-layer" role="region" aria-label="通知" aria-live="polite">
      <TransitionGroup name="toast">
        <div
          v-for="t in toasts"
          :key="t.id"
          class="toast"
          :class="`toast--${t.type}`"
          role="status"
          @click="dismiss(t.id)"
        >
          <span class="toast__icon" aria-hidden="true">
            <!-- success -->
            <svg
              v-if="t.type === 'success'"
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2.4"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <polyline points="20 6 9 17 4 12" />
            </svg>
            <!-- error -->
            <svg
              v-else-if="t.type === 'error'"
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2.4"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
            <!-- info -->
            <svg
              v-else
              viewBox="0 0 24 24"
              width="16"
              height="16"
              fill="none"
              stroke="currentColor"
              stroke-width="2.4"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="12" cy="12" r="9" />
              <line x1="12" y1="11" x2="12" y2="16" />
              <line x1="12" y1="8" x2="12" y2="8" />
            </svg>
          </span>
          <span class="toast__text">{{ t.text }}</span>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { useToast } from '@/composables/useToast'

const { toasts, dismiss } = useToast()
</script>

<style scoped>
.toast-layer {
  position: fixed;
  top: var(--space-4);
  right: var(--space-4);
  z-index: var(--z-toast);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  pointer-events: none;
  max-width: min(22.5rem, calc(100vw - 2 * var(--space-4)));
}

.toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-left: 0.1875rem solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-2);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  line-height: var(--line-height-normal);
  cursor: pointer;
  user-select: none;
}

.toast--success {
  border-left-color: var(--success-solid);
}
.toast--error {
  border-left-color: var(--danger-solid);
}
.toast--info {
  border-left-color: var(--info-solid);
}

.toast__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 1rem;
  height: 1rem;
}
.toast--success .toast__icon {
  color: var(--success-solid);
}
.toast--error .toast__icon {
  color: var(--danger-solid);
}
.toast--info .toast__icon {
  color: var(--info-solid);
}

.toast__text {
  flex: 1;
  min-width: 0;
  word-break: break-word;
}

.toast-enter-active,
.toast-leave-active {
  transition:
    opacity var(--motion-base) var(--motion-ease),
    transform var(--motion-base) var(--motion-ease);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(calc(-1 * var(--space-2)));
}
</style>

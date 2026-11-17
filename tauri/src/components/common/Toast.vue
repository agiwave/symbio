<!--
  Toast.vue — 全局浮动消息浮层

  全站唯一实例，挂在 MainLayout 中。消费 useToast() 的单例状态，
  取代原先散落在各资源页的 4 套本地 toast 实现。

  视觉全部令牌化（浅色/深色均正确）：
  - 浮层底色：--surface-overlay
  - 描边：--border-default，左侧语义强调条按 type 取 solid 色
  - 阴影：--shadow-2（深色下自动转为深底描边感）
  点 × 按钮关闭（整块不再可点，错误信息因此能选中复制）；aria-live 便于读屏播报，
  错误另走 assertive 通道以便立刻告知。
-->
<template>
  <Teleport to="body">
    <div class="toast-layer" role="region" aria-label="通知" aria-live="polite">
      <!-- 错误用 assertive 通道：读屏用户必须被立刻告知失败（切换失败等），
           polite 会排到当前朗读之后，常被错过。 -->
      <div class="toast-assertive" aria-live="assertive" aria-atomic="true">
        <template v-for="t in toasts" :key="'a-' + t.id">
          <span v-if="t.type === 'error'" class="sr-only">{{ t.text }}</span>
        </template>
      </div>
      <TransitionGroup name="toast">
        <div
          v-for="t in toasts"
          :key="t.id"
          class="toast"
          :class="`toast--${t.type}`"
          role="status"
          @mouseenter="pause(t.id)"
          @mouseleave="resume(t.id)"
          @focusin="pause(t.id)"
          @focusout="resume(t.id)"
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
          <!-- 内联动作（如「重试」）：提示说得出下一步，就要给得出那个按钮 -->
          <button
            v-if="t.action"
            type="button"
            class="toast__action"
            @click.stop="runAction(t)"
          >
            {{ t.action.label }}
          </button>
          <!-- 关闭必须有可聚焦的语义控件：整块 @click 对键盘用户不存在 -->
          <button
            type="button"
            class="toast__close"
            aria-label="关闭通知"
            @click.stop="dismiss(t.id)"
          >×</button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { useToast, type ToastItem } from '@/composables/useToast'

const { toasts, dismiss, pause, resume } = useToast()

/** 先移除本条再执行：动作往往改变页面状态，留着一条已失效的提示只会误导 */
function runAction(t: ToastItem) {
  const action = t.action
  dismiss(t.id)
  action?.run()
}
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
  /* 整块不再 @click 关闭：错误信息（失败原因 / 路径）需要能选中复制排查。
     关闭交给右上角的 × 按钮。 */
  cursor: default;
  user-select: text;
}

.toast-assertive {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
}
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
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

.toast__action {
  flex-shrink: 0;
  padding: 0.1rem 0.5rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-sm);
  background: var(--surface-panel);
  color: var(--accent);
  font-size: var(--font-size-xs);
  cursor: pointer;
}
.toast__action:hover {
  border-color: var(--accent);
}
.toast__action:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}

.toast__close {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.25rem;
  height: 1.25rem;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.95rem;
  line-height: 1;
  cursor: pointer;
}
.toast__close:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}
.toast__close:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
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

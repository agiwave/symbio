<!--
  ConfirmDialog — 二次确认对话框（自定义 modal，替代浏览器原生 confirm）

  设计原则：
  - 主题一致：使用项目 CSS variables，与系统其它 modal 风格统一
  - 不阻塞 UI：可关闭、可点遮罩取消
  - 异步：通过 resolve 回调返回用户选择（不阻塞调用栈）
  - 可访问：focus trap、ESC 关闭、aria 属性
-->
<template>
  <Transition name="confirm-fade">
    <div
      v-if="visible"
      class="confirm-overlay"
      @click.self="onCancel"
      @keydown="onKeydown"
    >
      <div
        ref="dialogRef"
        class="confirm-dialog"
        role="alertdialog"
        :aria-labelledby="titleId"
        :aria-describedby="messageId"
        tabindex="-1"
      >
        <header v-if="title" class="confirm-header">
          <span v-if="icon" class="confirm-icon" :class="iconClass">{{ icon }}</span>
          <h3 :id="titleId" class="confirm-title">{{ title }}</h3>
        </header>
        <div :id="messageId" class="confirm-message">
          <slot>{{ message }}</slot>
        </div>
        <footer class="confirm-footer">
          <button
            type="button"
            class="confirm-btn cancel"
            :disabled="loading"
            @click="onCancel"
          >
            {{ cancelText }}
          </button>
          <button
            type="button"
            :class="['confirm-btn', 'primary', danger ? 'danger' : '']"
            :disabled="loading"
            @click="onConfirm"
          >
            {{ loading ? '处理中…' : confirmText }}
          </button>
        </footer>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'

interface Props {
  /** 是否可见 */
  visible: boolean
  /** 标题（可省略） */
  title?: string
  /** 消息文本（也可使用 slot 覆盖） */
  message?: string
  /** 确认按钮文本 */
  confirmText?: string
  /** 取消按钮文本 */
  cancelText?: string
  /** 是否为危险操作（变红按钮） */
  danger?: boolean
  /** 图标 emoji 或字符 */
  icon?: string
  /** 图标变体（info / warning / danger） */
  iconKind?: 'info' | 'warning' | 'danger'
  /** 异步处理 loading 状态 */
  loading?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  title: '',
  message: '',
  confirmText: '确认',
  cancelText: '取消',
  danger: false,
  icon: '',
  iconKind: 'info',
  loading: false,
})

const emit = defineEmits<{
  (e: 'confirm'): void
  (e: 'cancel'): void
  (e: 'update:visible', v: boolean): void
}>()

const titleId = computed(
  () => `confirm-title-${Math.random().toString(36).slice(2, 9)}`
)
const messageId = computed(
  () => `confirm-msg-${Math.random().toString(36).slice(2, 9)}`
)
const dialogRef = ref<HTMLDivElement | null>(null)

const iconClass = computed(() => `kind-${props.iconKind}`)

function onCancel() {
  if (props.loading) return
  emit('cancel')
  emit('update:visible', false)
}

function onConfirm() {
  if (props.loading) return
  emit('confirm')
}

/**
 * 焦点陷阱（focus trap）：在对话框内循环 Tab / Shift+Tab，
 * 保证键盘用户不会被焦点"逃出"遮罩层。ESC 走 onCancel。
 */
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    onCancel()
    return
  }
  if (e.key !== 'Tab' || !dialogRef.value) return
  const focusables = Array.from(
    dialogRef.value.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    )
  )
  if (focusables.length === 0) {
    e.preventDefault()
    return
  }
  const first = focusables[0]
  const last = focusables[focusables.length - 1]
  const active = document.activeElement as HTMLElement | null
  if (e.shiftKey && (active === first || active === dialogRef.value)) {
    e.preventDefault()
    last.focus()
  } else if (!e.shiftKey && active === last) {
    e.preventDefault()
    first.focus()
  }
}

// 打开时自动聚焦首个可交互元素（焦点陷阱入口）
watch(
  () => props.visible,
  async (v) => {
    if (v) {
      await nextTick()
      const first = dialogRef.value?.querySelector<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), [tabindex]:not([tabindex="-1"])'
      )
      if (first) {
        first.focus()
      } else {
        dialogRef.value?.focus()
      }
    }
  }
)
</script>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--z-dialog);
  padding: var(--space-4);
}

.confirm-dialog {
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  min-width: 20rem;
  max-width: 30rem;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  outline: none;
}

.confirm-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-4) var(--space-5) var(--space-2);
}

.confirm-icon {
  font-size: 1.4rem;
  width: 2rem;
  height: 2rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-full);
  background: var(--surface-hover);
  flex-shrink: 0;
}
.confirm-icon.kind-info { background: var(--info-bg); }
.confirm-icon.kind-warning { background: var(--warning-bg); }
.confirm-icon.kind-danger { background: var(--danger-bg); }

.confirm-title {
  margin: 0;
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}

.confirm-message {
  padding: var(--space-2) var(--space-5) var(--space-4);
  font-size: var(--font-size-base);
  line-height: var(--line-height-normal);
  color: var(--text-secondary);
  white-space: pre-line;
}

.confirm-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-5) var(--space-4);
  border-top: 1px solid var(--border-default);
  background: var(--surface-hover);
}

.confirm-btn {
  padding: var(--space-2) var(--space-4);
  font-size: var(--font-size-base);
  border: 1px solid var(--border-default);
  background: var(--surface-overlay);
  color: var(--text-primary);
  border-radius: var(--radius-md);
  cursor: pointer;
  font-weight: var(--font-weight-medium);
  transition: background-color var(--motion-fast) var(--motion-ease),
    border-color var(--motion-fast) var(--motion-ease);
  min-width: 5rem;
}
.confirm-btn:hover:not(:disabled) {
  background: var(--surface-hover);
}
.confirm-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.confirm-btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}
.confirm-btn.primary:hover:not(:disabled) {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
}
.confirm-btn.primary.danger {
  background: var(--danger-solid);
  border-color: var(--danger-solid);
}
.confirm-btn.primary.danger:hover:not(:disabled) {
  background: var(--danger-solid);
  filter: brightness(0.92);
  border-color: var(--danger-solid);
}

.confirm-fade-enter-active,
.confirm-fade-leave-active {
  transition: opacity 0.18s ease;
}
.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}
</style>

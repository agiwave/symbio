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
      @keydown.esc="onCancel"
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

// 打开时自动 focus 到 dialog（便于 ESC 关闭）
watch(
  () => props.visible,
  async (v) => {
    if (v) {
      await nextTick()
      dialogRef.value?.focus()
    }
  }
)
</script>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
  padding: 1rem;
}

.confirm-dialog {
  background: var(--color-bg, #fff);
  border: 1px solid var(--color-border, #e5e7eb);
  border-radius: 10px;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.25);
  min-width: 320px;
  max-width: 480px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  outline: none;
}

.confirm-header {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 1rem 1.25rem 0.5rem;
}

.confirm-icon {
  font-size: 1.4rem;
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.04);
  flex-shrink: 0;
}
.confirm-icon.kind-info { background: rgba(59, 130, 246, 0.12); }
.confirm-icon.kind-warning { background: rgba(245, 158, 11, 0.15); }
.confirm-icon.kind-danger { background: rgba(239, 68, 68, 0.12); }

.confirm-title {
  margin: 0;
  font-size: 1rem;
  font-weight: 600;
  color: var(--color-text, #1f2937);
}

.confirm-message {
  padding: 0.5rem 1.25rem 1rem;
  font-size: 0.9rem;
  line-height: 1.5;
  color: var(--color-text-secondary, #4b5563);
  white-space: pre-line;
}

.confirm-footer {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  padding: 0.75rem 1.25rem 1rem;
  border-top: 1px solid var(--color-border, #e5e7eb);
  background: rgba(0, 0, 0, 0.02);
}

.confirm-btn {
  padding: 0.45rem 1rem;
  font-size: 0.85rem;
  border: 1px solid var(--color-border, #d1d5db);
  background: var(--color-bg, #fff);
  color: var(--color-text, #1f2937);
  border-radius: 6px;
  cursor: pointer;
  font-weight: 500;
  transition: all 0.15s;
  min-width: 80px;
}
.confirm-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.04);
}
.confirm-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.confirm-btn.primary {
  background: var(--color-primary, #667eea);
  border-color: var(--color-primary, #667eea);
  color: #fff;
}
.confirm-btn.primary:hover:not(:disabled) {
  background: var(--color-primary-dark, #5568d3);
  border-color: var(--color-primary-dark, #5568d3);
}
.confirm-btn.primary.danger {
  background: #ef4444;
  border-color: #ef4444;
}
.confirm-btn.primary.danger:hover:not(:disabled) {
  background: #dc2626;
  border-color: #dc2626;
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

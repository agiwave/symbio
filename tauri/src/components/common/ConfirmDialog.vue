<!--
  ConfirmDialog — 二次确认对话框（自定义 modal，替代浏览器原生 confirm）

  设计原则：
  - 主题一致：使用项目 CSS variables，与系统其它 modal 风格统一
  - 不阻塞 UI：可关闭、可点遮罩取消
  - 异步：通过 resolve 回调返回用户选择（不阻塞调用栈）
  - 可访问：focus trap、ESC 关闭、aria 属性

  外壳（遮罩 / ESC / 焦点陷阱 / 自动聚焦）已收进 `BaseModal`——本组件只保留
  **确认框自己的语义**：标题 / 图标 / 消息 / 两个按钮，以及「`loading` 期间
  拒绝取消」这条业务规则。面板类名 `confirm-dialog` 继续传给 BaseModal，故
  特化样式与测试的定位方式不变；遮罩改由机制类 `modal-mask` 定位。
-->
<template>
  <BaseModal
    :visible="visible"
    panel-class="confirm-dialog"
    role="alertdialog"
    :labelledby="titleId"
    :describedby="messageId"
    @close="onCancel"
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
  </BaseModal>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import BaseModal from './BaseModal.vue'

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
</script>

<style scoped>
/* 遮罩与面板的底色 / 圆角 / 阴影 / 层级由 `BaseModal` 统一提供（`.modal-mask` /
   `.modal-panel`）；这里只写确认框自己的尺寸与边框。 */
.confirm-dialog {
  border: 1px solid var(--border-default);
  min-width: 20rem;
  max-width: 30rem;
  display: flex;
  flex-direction: column;
  overflow: hidden;
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
/* 取消按钮：在基础中性样式上再降一级视觉权重 */
.confirm-btn.cancel {
  color: var(--text-secondary);
  background: transparent;
}
.confirm-btn.cancel:hover:not(:disabled) {
  background: var(--surface-hover);
}
</style>

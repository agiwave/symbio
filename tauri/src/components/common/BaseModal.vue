<!--
  BaseModal — 弹窗外壳（**唯一实现**）

  职责：遮罩 + 面板容器 + **键盘可访问性**（ESC 关闭、Tab 焦点陷阱、打开时
  自动聚焦首个可交互元素）。内容一律由插槽提供——本组件不含任何业务、文案
  与按钮语义。

  ## 为什么要有它

  此前 4 处弹窗各自手写遮罩（`ConfirmDialog` / `OptionFormDialog` /
  `HomedirSwitcher` / `ModelChatPanel` 的编辑浮层），其中**只有 `ConfirmDialog`
  实现了 ESC 与焦点陷阱**。另外三处键盘用户按 ESC 关不掉、Tab 会跑到遮罩后面
  的页面上——这是可访问性缺陷，不是样式偏好，所以收敛它是修 bug 而非整理代码。

  顺带修掉两处硬编码（原 `ModelChatPanel` 的编辑浮层）：遮罩 `z-index: 100`
  低于 `--z-overlay`(1000) / `--z-dialog`(1500)，会被其它浮层盖住；底色写成
  `rgba(0,0,0,0.45)` 而不走 `--overlay`，于是**不跟随主题**（浅色主题下比别的
  弹窗更暗、暗色主题下反而更浅）。现在两者都由 token 统一给出。

  ## 类名契约

  遮罩恒带 `modal-mask`（机制类，负责定位 / 遮罩 / 层级），面板恒带
  `modal-panel`，调用方可用 `panelClass` 追加**语义类**（如 `confirm-dialog`）
  供特化样式与测试定位。

  刻意**不提供 `maskClass`**：遮罩的样式在四个使用方之间没有任何差异，多一个
  可传类名只会让"某个弹窗的遮罩长什么样"变成两处可写（本文件 + 调用方）。
  面板类名则有真实差异（宽度 / 边框 / 内边距）。

  ## 不内建 Teleport

  由调用方按需自行包 `<Teleport to="body">`。内建会让所有使用方的 DOM 位置
  一并改变（测试里 `wrapper.find` 就找不到内容了），收益不抵代价。
-->
<template>
  <Transition name="modal-fade">
    <div v-if="visible" class="modal-mask" @click.self="onMaskClick" @keydown="onKeydown">
      <div
        ref="panelRef"
        class="modal-panel"
        :class="panelClass"
        :role="role"
        aria-modal="true"
        :aria-labelledby="labelledby"
        :aria-describedby="describedby"
        tabindex="-1"
      >
        <slot />
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'

interface Props {
  /** 是否可见 */
  visible: boolean
  /** ESC / 点遮罩是否关闭（默认 true）。需要拦截时（如确认框 loading 中）传 false */
  dismissable?: boolean
  /** 面板追加语义类（宽度 / 边框 / 内边距这类各弹窗自己的语义） */
  panelClass?: string
  /** ARIA 角色：确认类用 `alertdialog`，普通表单/编辑用 `dialog` */
  role?: 'dialog' | 'alertdialog'
  /** 面板 `aria-labelledby` 指向的标题 id */
  labelledby?: string
  /** 面板 `aria-describedby` 指向的描述 id */
  describedby?: string
}

const props = withDefaults(defineProps<Props>(), {
  dismissable: true,
  panelClass: '',
  role: 'dialog',
  labelledby: undefined,
  describedby: undefined,
})

const emit = defineEmits<{ (e: 'close'): void }>()

const panelRef = ref<HTMLDivElement | null>(null)

/** 可聚焦元素选择器（焦点陷阱与自动聚焦**共用一份**，避免两处口径漂移） */
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

function onMaskClick() {
  if (props.dismissable) emit('close')
}

/**
 * 焦点陷阱：在面板内循环 Tab / Shift+Tab，保证键盘用户不会被焦点"逃出"遮罩。
 * ESC 走 `close`，由调用方决定语义（取消 / 直接关）。
 */
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    if (props.dismissable) emit('close')
    return
  }
  if (e.key !== 'Tab' || !panelRef.value) return
  const focusables = Array.from(panelRef.value.querySelectorAll<HTMLElement>(FOCUSABLE))
  if (focusables.length === 0) {
    e.preventDefault()
    return
  }
  const first = focusables[0]
  const last = focusables[focusables.length - 1]
  const active = document.activeElement as HTMLElement | null
  if (e.shiftKey && (active === first || active === panelRef.value)) {
    e.preventDefault()
    last.focus()
  } else if (!e.shiftKey && active === last) {
    e.preventDefault()
    first.focus()
  }
}

/**
 * 打开时自动聚焦首个可交互元素（焦点陷阱的入口）。
 *
 * `immediate: true` 不能省：弹窗有两种打开方式——`visible` 由 false 翻成 true
 * （`ConfirmDialog` / `HomedirSwitcher`），以及**挂载时就是可见**（
 * `OptionFormDialog` 恒传 `:visible="true"`，显隐由父级的条件渲染决定）。
 * 后者没有"变化"可触发 watch，只有 immediate 能覆盖。
 */
watch(
  () => props.visible,
  async (v) => {
    if (!v) return
    await nextTick()
    const first = panelRef.value?.querySelector<HTMLElement>(FOCUSABLE)
    if (first) {
      first.focus()
    } else {
      panelRef.value?.focus()
    }
  },
  { immediate: true }
)
</script>

<style scoped>
/* 遮罩：定位 / 底色 / 层级三者由 token 统一——不写死数值，否则不跟随主题 */
.modal-mask {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--z-dialog);
  padding: var(--space-4);
}

/* 面板：只给"所有弹窗都该有"的三样（底色 / 圆角 / 阴影）；
   宽度、边框、内边距属于各弹窗自己的语义，由 panelClass 追加。 */
.modal-panel {
  background: var(--surface-overlay);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  outline: none;
}

.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity var(--motion-fast) var(--motion-ease);
}
.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}
</style>

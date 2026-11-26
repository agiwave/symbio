<!--
  SettingsFormShell — 设置分区表单的共享布局壳

  统一各设置 editor（会话 / 本地工具 / 网络工具等）的视觉结构：
  - 顶部：分区标题 + 描述
  - 中部：setting 行（slot 默认，可用类：setting-item / setting-info / setting-desc /
    toggle / segmented / seg-btn 等，经 :deep 生效于 slot 内容）
  - 底部：操作区（slot footer，通常放保存按钮；按钮类由调用方自持——
    本壳不提供按钮样式，`.action-btn` 已随工作台提示态改造删除）
-->
<template>
  <div class="settings-form">
    <header class="form-header">
      <h2 class="form-title">{{ title }}</h2>
      <p v-if="description" class="form-desc">{{ description }}</p>
    </header>
    <div v-overlay-scrollbar class="form-body">
      <slot />
    </div>
    <footer v-if="$slots.footer" class="form-footer">
      <slot name="footer" />
    </footer>
  </div>
</template>

<script setup lang="ts">
import { vOverlayScrollbar } from '@/directives/overlayScrollbar'

defineProps<{
  title: string
  description?: string
}>()
</script>

<style scoped>
.settings-form {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  container: settings / inline-size;
}

.form-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-height: var(--workbench-bar-height);
  height: var(--workbench-bar-height);
  padding: 0.4375rem 1rem;
  border-bottom: 1px solid var(--border-default);
  background: var(--surface-panel);
  flex-shrink: 0;
}

.form-title {
  margin: 0;
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  white-space: nowrap;
  flex-shrink: 0;
}
.form-desc {
  margin: 0;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
  color: var(--text-muted);
}

.form-body {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 1rem clamp(1rem, 4vw, 2.5rem) 1.5rem;
  max-width: 40rem;
  width: 100%;
}

.form-footer {
  display: flex;
  gap: 0.75rem;
  padding: 1rem clamp(1rem, 4vw, 2.5rem);
  border-top: 1px solid var(--border-default);
  background: var(--surface-panel);
  flex-shrink: 0;
  max-width: 40rem;
  width: 100%;
}

/* ============ slot 内容（经 :deep 生效） ============ */

/* setting 行 */
.form-body :deep(.setting-item) {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.9rem 1rem;
  border-radius: var(--radius-md);
  transition: background-color var(--motion-fast) var(--motion-ease);
}
.form-body :deep(.setting-item:hover) {
  background: var(--surface-hover);
}
.form-body :deep(.setting-info) {
  flex: 1;
  min-width: 0;
}
/* 行标题：有对应控件的用 `<label for>`，没有的用 `.setting-label`——
   没有可关联控件的 `<label>` 是悬空语义，读屏与点击行为都不对。 */
.form-body :deep(.setting-info label),
.form-body :deep(.setting-info .setting-label) {
  display: block;
  font-weight: var(--font-weight-medium);
  font-size: var(--font-size-base);
  color: var(--text-primary);
  margin-bottom: 0.2rem;
}
.form-body :deep(.setting-desc) {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

/* 输入框 */
.form-body :deep(.setting-item input[type='number']),
.form-body :deep(.setting-item input[type='text']),
.form-body :deep(.setting-item input[type='password']) {
  min-height: 2rem;
  padding: 0.45rem 0.7rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  background: var(--surface-sunken);
  color: var(--text-primary);
}
.form-body :deep(.setting-item input[type='number']) {
  width: 6.5rem;
}
.form-body :deep(.setting-item input[type='text']),
.form-body :deep(.setting-item input[type='password']) {
  min-width: 14rem;
}
.form-body :deep(.setting-item input:focus) {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}

/* 开关 */
.form-body :deep(.toggle) {
  position: relative;
  display: inline-block;
  width: 3rem;
  height: 1.5rem;
  flex-shrink: 0;
}
.form-body :deep(.toggle input) {
  opacity: 0;
  width: 0;
  height: 0;
}
.form-body :deep(.toggle-slider) {
  position: absolute;
  inset: 0;
  cursor: pointer;
  background: var(--border-strong);
  border-radius: 1.5rem;
  transition: background var(--motion-fast) var(--motion-ease);
}
.form-body :deep(.toggle-slider::before) {
  position: absolute;
  content: '';
  height: 1.125rem;
  width: 1.125rem;
  left: 0.1875rem;
  bottom: 0.1875rem;
  background: var(--surface-panel);
  border-radius: 50%;
  transition: transform var(--motion-fast) var(--motion-ease);
}
.form-body :deep(.toggle input:checked + .toggle-slider) {
  background: var(--accent);
}
.form-body :deep(.toggle input:checked + .toggle-slider::before) {
  transform: translateX(1.5rem);
}

/* 分段选择 */
.form-body :deep(.segmented) {
  display: inline-flex;
  flex-wrap: wrap;
  max-width: 100%;
  background: var(--surface-sunken);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: 0.125rem;
}
.form-body :deep(.seg-btn) {
  min-height: 2rem;
  padding: 0.35rem 0.9rem;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-base);
  transition: background var(--motion-fast) var(--motion-ease), color var(--motion-fast) var(--motion-ease);
}
.form-body :deep(.seg-btn:hover) {
  color: var(--text-primary);
}
.form-body :deep(.seg-btn.active) {
  background: var(--surface-panel);
  color: var(--accent);
  font-weight: var(--font-weight-medium);
}

/* 详情栏可能只有 400–600px；按容器而非窗口折行，宽屏并排、窄栏堆叠。 */
@container settings (max-width: 40rem) {
  .form-body :deep(.setting-item) {
    flex-direction: column;
    align-items: stretch;
    gap: 0.65rem;
  }
  .form-body :deep(.segmented) {
    width: 100%;
  }
  .form-body :deep(.seg-btn) {
    flex: 1 1 4rem;
    min-height: 2.25rem;
  }
  .form-body :deep(.kind-controls),
  .form-body :deep(.volume-control) {
    width: 100%;
  }
  .form-body :deep(.setting-item input[type='text']),
  .form-body :deep(.setting-item input[type='password']) {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
  }
  .form-footer {
    flex-wrap: wrap;
  }
}
</style>

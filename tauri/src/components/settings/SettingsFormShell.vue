<!--
  SettingsFormShell — 设置分区表单的共享布局壳

  统一各设置 editor（会话 / 本地工具 / 网络工具等）的视觉结构：
  - 顶部：分区标题 + 描述
  - 中部：setting 行（slot 默认，可用类：setting-item / setting-info / setting-desc /
    toggle / segmented / seg-btn 等，经 :deep 生效于 slot 内容）
  - 底部：操作区（slot footer，通常放保存按钮，可用类：action-btn）
-->
<template>
  <div class="settings-form">
    <header class="form-header">
      <h2 class="form-title">{{ title }}</h2>
      <p v-if="description" class="form-desc">{{ description }}</p>
    </header>
    <div class="form-body">
      <slot />
    </div>
    <footer v-if="$slots.footer" class="form-footer">
      <slot name="footer" />
    </footer>
  </div>
</template>

<script setup lang="ts">
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
  overflow-y: auto;
  padding: 1.5rem 2.5rem;
}

.form-header {
  margin-bottom: 1.25rem;
}
.form-title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}
.form-desc {
  margin: 0.3rem 0 0;
  font-size: var(--font-size-sm);
  color: var(--text-muted);
}

.form-body {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  max-width: 40rem;
}

.form-footer {
  display: flex;
  gap: 0.75rem;
  padding-top: 1rem;
  margin-top: 0.75rem;
  border-top: 1px solid var(--border-default);
  max-width: 40rem;
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
.form-body :deep(.setting-info label) {
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
  background: var(--surface-sunken);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: 0.125rem;
}
.form-body :deep(.seg-btn) {
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

/* 按钮（footer 内） */
.form-footer :deep(.action-btn) {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.5rem 1rem;
  border: none;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: var(--text-on-accent);
  font-size: var(--font-size-base);
  cursor: pointer;
  white-space: nowrap;
  transition: background var(--motion-fast) var(--motion-ease), opacity var(--motion-fast) var(--motion-ease);
}
.form-footer :deep(.action-btn:hover:not(:disabled)) {
  background: var(--accent-hover);
}
.form-footer :deep(.action-btn:disabled) {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>

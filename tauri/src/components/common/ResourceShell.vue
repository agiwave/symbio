<!--
  ResourceShell — 通用"左列表 + 右详情"两栏布局

  适用场景：管理类页面（Model Provider、MCP Server、Skill、Agent、Channel 等）
  这些页面共享：
  - 顶部 panel-header（标题 + 操作按钮）
  - 状态条（list-meta）
  - 卡片列表
  - 空状态
  - 加载态
  - 右侧详情区
  - 浮动 Toast

  避免在每个 view 里重复实现（当前 ModelProvidersView / McpView 90% 重复）。

  Slot 设计：
  - `header-actions`：右上角按钮组（默认带 + 新建按钮）
  - `meta`：状态条（list-meta）
  - `list`：左侧列表内容
  - `empty`：空状态
  - `loading`：加载态
  - `detail`：右侧详情
  - `toast`：浮动消息（不传则用 ResourceShell 内置）
-->
<template>
  <div class="resource-shell">
    <!-- 左栏 -->
    <aside class="shell-list" :style="{ width: `${listWidth}px` }">
      <header class="panel-header">
        <h3 class="panel-title">{{ title }}</h3>
        <div class="header-actions">
          <slot
            name="header-actions"
            :loading="loading"
            :on-new="emitNew"
          >
            <button
              v-if="!hideDefaultNew"
              class="icon-btn"
              :title="`新建 ${title}`"
              :disabled="loading"
              @click="emitNew"
            >
              <svg
                viewBox="0 0 24 24"
                width="16"
                height="16"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
              >
                <line x1="12" y1="5" x2="12" y2="19" />
                <line x1="5" y1="12" x2="19" y2="12" />
              </svg>
            </button>
          </slot>
        </div>
      </header>

      <div v-if="$slots.meta" class="list-meta">
        <slot name="meta" />
      </div>

      <!-- 列表内容 -->
      <slot name="list" />

      <!-- 默认空状态 -->
      <div v-if="!hasListContent && $slots.empty" class="empty-state">
        <slot name="empty" />
      </div>

      <!-- 默认加载态 -->
      <div v-else-if="!hasListContent && loading" class="loading-state">
        加载中…
      </div>
    </aside>

    <!-- 右栏 -->
    <section class="shell-detail">
      <slot name="detail" />
    </section>

    <!-- 浮动消息（外部传入时优先） -->
    <slot name="toast" />
  </div>
</template>

<script setup lang="ts">


interface ResourceShellProps {
  title: string
  /** 左侧栏宽度（px），默认 260 */
  listWidth?: number
  /** 是否隐藏默认的 + 新建按钮（外部完全自定义 header-actions） */
  hideDefaultNew?: boolean
  /** 列表区是否有内容（用于控制空/加载态的渲染） */
  hasListContent?: boolean
  /** 全局 loading 标志 */
  loading?: boolean
}

const props = withDefaults(defineProps<ResourceShellProps>(), {
  listWidth: 260,
  hideDefaultNew: false,
  hasListContent: false,
  loading: false,
})

const emit = defineEmits<{
  (e: 'new'): void
}>()

function emitNew() {
  emit('new')
}
</script>

<style scoped>
.resource-shell {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  background: var(--color-bg);
  position: relative;
}

/* 左侧栏 */
.shell-list {
  flex: 0 0 auto;
  min-width: 220px;
  max-width: 360px;
  display: flex;
  flex-direction: column;
  background: var(--color-bg);
  border-right: 1px solid var(--color-border);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.panel-title {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--color-text-secondary);
  margin: 0;
}

.header-actions {
  display: flex;
  gap: 0.25rem;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.15s;
}

.icon-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.06);
  color: var(--color-text);
}

.icon-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.list-meta {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.7rem;
  color: var(--color-text-muted);
  padding: 0.4rem 0.75rem;
  border-bottom: 1px solid var(--color-border);
  background: rgba(102, 126, 234, 0.04);
  flex-shrink: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.running-pulse {
  display: inline-block;
  width: 7px;
  height: 7px;
  background: #22c55e;
  border-radius: 50%;
  animation: pulse 1.4s ease-in-out infinite;
  flex-shrink: 0;
}

@keyframes pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.5; transform: scale(0.85); }
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
  font-size: 0.85rem;
  gap: 0.3rem;
  padding: 1rem;
  text-align: center;
}

.empty-state :deep(.hint) {
  font-size: 0.75rem;
  opacity: 0.7;
}

.loading-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
  font-size: 0.8rem;
}

/* 右侧详情栏 */
.shell-detail {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--color-bg);
}
</style>

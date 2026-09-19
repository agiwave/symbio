<!--
  Workbench — 「侧边栏 + 列表 + 详情」三栏工作台容器（全项目统一命名/唯一实现）

  页面容器，只做插槽装配，自身无数据逻辑。三栏自上而下相接：

  - **侧边栏**（窄条图标导航，可选——不传 railItems 即无侧边栏的两栏形态）：
    类别集合一律由后端下发（VDFS 目录节点），前端只做 UI 映射；`rail-header` /
    `rail-footer` 由宿主注入（返回键 / logo / 系统目录入口等宿主件）；
  - **中栏**：panel-header（标题 + header-actions）+ list + 空态 / 加载态；
  - **右栏**：detail。

  插槽只有**消费方真的会用**的那些：`rail-header` / `rail-footer` / `header-actions`
  / `list` / `empty` / `detail`。此前还转发过 `content` / `meta` 两个无人使用的插槽，
  以及 `back` / `backTitle` 两个恒为缺省的 prop（连同它们的默认返回键），已删除——
  不可达的分支不是灵活性，是维护税。

  侧栏的 `.nav-btn` / `.nav-count` / `.nav-label` 是**全局**样式
  （styles/controls.css）：`rail-header` 插槽内容属宿主作用域，只有全局类才能命中。

  VDFS 的三栏页面 = VdfsWorkbench 控件（components/vdfs）+ useVdfs 数据逻辑
  （绑定数据地址，自取左栏/中栏/详情）。规范：docs/design/vdfs.md。
-->
<template>
  <div class="workbench">
    <!-- 第一栏：侧边栏 -->
    <nav v-if="railItems" class="side-nav">
      <slot name="rail-header" />

      <div class="nav-items">
        <button
          v-for="it in railItems"
          :key="it.key"
          class="nav-btn"
          :class="{ active: it.active }"
          :aria-label="it.label"
          :title="it.description || it.label"
          @click="$emit('rail-select', it.key)"
        >
          <component :is="it.icon" v-if="it.icon" />
          <svg
            v-else
            viewBox="0 0 24 24"
            width="20"
            height="20"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
            <polyline points="13 2 13 9 20 9" />
          </svg>
          <span v-if="it.count" class="nav-count">{{ it.count }}</span>
          <span class="nav-label">{{ it.label }}</span>
        </button>
      </div>

      <slot name="rail-footer" />
    </nav>

    <!-- 第二栏：列表 -->
    <aside class="workbench-list" :style="{ width: `${listWidth}px` }">
      <header class="panel-header">
        <h3 class="panel-title">{{ title }}</h3>
        <div class="header-actions">
          <slot name="header-actions" />
        </div>
      </header>

      <slot name="list" />

      <div v-if="!hasListContent && $slots.empty" class="empty-state">
        <slot name="empty" />
      </div>
      <div v-else-if="!hasListContent && loading" class="loading-state">加载中…</div>
    </aside>

    <!-- 第三栏：详情 -->
    <section class="workbench-detail">
      <slot name="detail" />
    </section>
  </div>
</template>

<script setup lang="ts">
import type { Component } from 'vue'

/**
 * 侧栏导航项（宿主与容器之间的纯 UI 契约）。
 *
 * 它是**呈现投影**而非协议形状：由数据层（如 useVdfs 的 navItems）产出，
 * 容器只负责画。`icon` 是已解析的组件（图标映射属 registry 的纯 UI 映射）。
 */
export interface WorkbenchRailItem {
  /** 类别键（provider kind / 容器子类别 kind / VDFS 挂载名） */
  key: string
  label: string
  /** 类别图标（registry/vdfsIcons 注册表映射；缺省回退通用文件图标） */
  icon?: Component | null
  /** 语义说明（作 tooltip；VDFS 导航由后端下发 description） */
  description?: string
  /** 计数角标（0/undefined 不显示） */
  count?: number
  active?: boolean
}

withDefaults(
  defineProps<{
    /** 侧边栏类别项（后端下发的 VDFS 挂载点清单；不传 = 本页无侧边栏） */
    railItems?: WorkbenchRailItem[]
    /** 中栏标题（列表加载后即为当前目录的自述） */
    title?: string
    /** 中栏宽度（px） */
    listWidth?: number
    /** 中栏列表是否已有内容（控制空/加载态是否渲染） */
    hasListContent?: boolean
    /** 中栏加载中 */
    loading?: boolean
  }>(),
  {
    railItems: undefined,
    title: '',
    listWidth: 260,
    hasListContent: false,
    loading: false,
  }
)

defineEmits<{
  (e: 'rail-select', key: string): void
}>()
</script>

<style scoped>
.workbench {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: var(--surface-page);
}

/* ============== 第一栏：侧边栏 ============== */
.side-nav {
  width: var(--sidebar-width);
  background: var(--surface-panel);
  border-right: 1px solid var(--border-default);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  z-index: 10;
}

.nav-items {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.25rem;
  flex: 1;
  padding: 0.5rem 0.375rem;
  overflow-y: auto;
}

/* ============== 第二栏：列表 ============== */
.workbench-list {
  flex: 0 0 auto;
  min-width: 13.75rem;
  max-width: 22.5rem;
  display: flex;
  flex-direction: column;
  background: var(--surface-panel);
  border-right: 1px solid var(--border-default);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
}

.panel-title {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-secondary);
  margin: 0;
}

.header-actions {
  display: flex;
  gap: 0.25rem;
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
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
  color: var(--text-muted);
  font-size: 0.8rem;
}

/* ============== 第三栏：详情 ============== */
.workbench-detail {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--surface-panel);
}
</style>

<!--
  NavRail — 「侧边栏 + 列表 + 详情」三栏工作台的侧边栏唯一实现

  窄条图标导航，全项目所有三栏页面共用（此前每类页面各持一份重复的 nav CSS，
  现收敛于此）：
  - MainLayout：items = `.vdfs` 挂载点清单（资源类别导航，后端 order 排列）。

  类别集合全部由后端下发，前端只做 UI 映射（icon 由调用方传入，
  未传时回退通用文件图标）。头部/尾部内容经插槽注入（logo、返回键、系统工具）。
-->
<template>
  <nav class="side-nav">
    <slot name="header">
      <button
        v-if="back"
        class="nav-btn back"
        :title="backTitle"
        :aria-label="backTitle"
        @click="$emit('back')"
      >
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="19" y1="12" x2="5" y2="12" />
          <polyline points="12 19 5 12 12 5" />
        </svg>
      </button>
    </slot>

    <div class="nav-items">
      <button
        v-for="it in items"
        :key="it.key"
        class="nav-btn"
        :class="{ active: it.active }"
        :aria-label="it.label"
        :title="it.description || it.label"
        @click="$emit('select', it.key)"
      >
        <component :is="it.icon" v-if="it.icon" />
        <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
          <polyline points="13 2 13 9 20 9" />
        </svg>
        <span v-if="it.count" class="nav-count">{{ it.count }}</span>
        <span class="nav-label">{{ it.label }}</span>
      </button>
    </div>

    <slot name="footer" />
  </nav>
</template>

<script setup lang="ts">
import type { Component } from 'vue'

export interface NavRailItem {
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
    items: NavRailItem[]
    /** 头部渲染返回键（默认插槽未接管 header 时生效） */
    back?: boolean
    backTitle?: string
  }>(),
  { back: false, backTitle: '返回' }
)

defineEmits<{
  (e: 'select', key: string): void
  (e: 'back'): void
}>()
</script>

<style scoped>
/* 布局壳专属样式；.nav-btn/.nav-count/.nav-label 为共享控件样式，
   收敛在 styles/controls.css（插槽内容属父作用域，须全局类才能命中） */
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
</style>

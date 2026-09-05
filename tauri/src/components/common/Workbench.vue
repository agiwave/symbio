<!--
  Workbench — 「侧边栏 + 列表 + 详情」三栏工作台容器（全项目统一命名/唯一实现）

  整个 App 是一台三栏工作台，本组件是其页面容器，两种用法：

  1. 应用外壳模式（MainLayout）：传 railItems + #content 插槽 ——
     侧边栏 = 后端 providers 注册表导航，工作区 = RouterView；
  2. 资源页模式（WorkbenchView，全 App 唯一资源页面）：传 railItems
     （容器页）或不传（顶层页，侧边栏由 MainLayout 承担）+ list/detail 等插槽 ——
     工作区 = 内置 ResourceShell（列表 + 详情）。

  状态机在 useWorkbench.ts（同名配套组合式）；类别集合一律由后端注册表下发
  （providers / container_kinds），前端零硬编码。
-->
<template>
  <div class="workbench">
    <!-- 第一栏：侧边栏（可选——顶层资源页的侧边栏由应用外壳承担） -->
    <NavRail
      v-if="railItems"
      :items="railItems"
      :back="back"
      :back-title="backTitle"
      @select="(k) => $emit('rail-select', k)"
      @back="$emit('rail-back')"
    >
      <template v-if="$slots['rail-header']" #header><slot name="rail-header" /></template>
      <template v-if="$slots['rail-footer']" #footer><slot name="rail-footer" /></template>
    </NavRail>

    <!-- 工作区（第二/三栏）：content 插槽完全接管（应用外壳模式），
         否则内置 ResourceShell（列表 + 详情，资源页模式），插槽逐一定向转发 -->
    <main class="workbench-content">
      <slot v-if="$slots.content" name="content" />
      <ResourceShell
        v-else
        :title="title ?? ''"
        :list-width="listWidth"
        :hide-default-new="hideDefaultNew"
        :has-list-content="hasListContent"
        :loading="loading"
        @new="$emit('new')"
      >
        <template v-if="$slots['header-actions']" #header-actions="sa">
          <slot name="header-actions" v-bind="sa" />
        </template>
        <template v-if="$slots.meta" #meta><slot name="meta" /></template>
        <template v-if="$slots.list" #list><slot name="list" /></template>
        <template v-if="$slots.empty" #empty><slot name="empty" /></template>
        <template v-if="$slots.detail" #detail><slot name="detail" /></template>
      </ResourceShell>
    </main>
  </div>
</template>

<script setup lang="ts">
import NavRail from '@/components/common/NavRail.vue'
import type { NavRailItem } from '@/components/common/NavRail.vue'
import ResourceShell from '@/components/common/ResourceShell.vue'

withDefaults(
  defineProps<{
    /** 侧边栏类别项（后端注册表下发；不传 = 本页无侧边栏） */
    railItems?: NavRailItem[]
    /** 侧边栏返回键（容器页用） */
    back?: boolean
    backTitle?: string
    /** —— 以下透传 ResourceShell（资源页模式）—— */
    title?: string
    listWidth?: number
    hideDefaultNew?: boolean
    hasListContent?: boolean
    loading?: boolean
  }>(),
  {
    railItems: undefined,
    back: false,
    backTitle: '返回',
    title: '',
    listWidth: 260,
    hideDefaultNew: false,
    hasListContent: false,
    loading: false,
  }
)

defineEmits<{
  (e: 'rail-select', key: string): void
  (e: 'rail-back'): void
  (e: 'new'): void
}>()
</script>

<style scoped>
.workbench {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.workbench-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  display: flex;
}
</style>

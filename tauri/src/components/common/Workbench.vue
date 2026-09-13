<!--
  Workbench — 「侧边栏 + 列表 + 详情」三栏工作台容器（全项目统一命名/唯一实现）

  整个 App 是一台三栏工作台，本组件是其页面容器，两种用法：

  1. 应用外壳模式（MainLayout）：传 railItems + #content 插槽 ——
     侧边栏 = `.vdfs` 挂载点导航，工作区 = RouterView；
  2. 页面模式（VdfsView）：传 list/detail 等插槽（嵌入形态不传 railItems，
     侧边栏由 MainLayout 承担）——工作区 = 内置 EntityShell（列表 + 详情）。

  类别集合一律由后端下发（VDFS 挂载点 / 目录节点），前端零硬编码。
  （原「统一实体页」WorkbenchView 与其状态机 useWorkbench 已于 S5 下线。）
-->
<template>
  <div class="workbench">
    <!-- 第一栏：侧边栏（可选——顶层实体页的侧边栏由应用外壳承担） -->
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
         否则内置 EntityShell（列表 + 详情，实体页模式），插槽逐一定向转发 -->
    <main class="workbench-content">
      <slot v-if="$slots.content" name="content" />
      <EntityShell
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
      </EntityShell>
    </main>
  </div>
</template>

<script setup lang="ts">
import NavRail from '@/components/common/NavRail.vue'
import type { NavRailItem } from '@/components/common/NavRail.vue'
import EntityShell from '@/components/common/EntityShell.vue'

withDefaults(
  defineProps<{
    /** 侧边栏类别项（后端注册表下发；不传 = 本页无侧边栏） */
    railItems?: NavRailItem[]
    /** 侧边栏返回键（容器页用） */
    back?: boolean
    backTitle?: string
    /** —— 以下透传 EntityShell（实体页模式）—— */
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

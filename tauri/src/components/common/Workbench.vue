<!--
  Workbench — 「侧边栏 + 列表 + 详情」三栏工作台容器（全项目统一命名/唯一实现）

  页面容器，只做插槽装配，自身无数据逻辑：

  - 传 railItems → 渲染第一栏（NavRail），rail-header / rail-footer 由宿主注入
    （返回键 / logo / 系统目录入口等宿主件）；
  - 工作区 = 内置 VdfsShell（列表 + 详情），list / detail / empty / meta /
    header-actions 插槽逐一定向转发。

  VDFS 的三栏页面 = VdfsWorkbench 控件（components/vdfs）+ useVdfs 数据逻辑
  （绑定数据地址，自取左栏/中栏/详情）+ 本容器。类别集合一律由后端下发
  （VDFS 目录节点），前端零硬编码（规范：docs/design/vdfs.md）。
-->
<template>
  <div class="workbench">
    <!-- 第一栏：侧边栏（可选——不传 railItems 即无侧边栏的两栏形态） -->
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
         否则内置 VdfsShell（列表 + 详情，资源页模式），插槽逐一定向转发 -->
    <main class="workbench-content">
      <slot v-if="$slots.content" name="content" />
      <VdfsShell
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
      </VdfsShell>
    </main>
  </div>
</template>

<script setup lang="ts">
import NavRail from '@/components/common/NavRail.vue'
import type { NavRailItem } from '@/components/common/NavRail.vue'
import VdfsShell from '@/components/common/VdfsShell.vue'

withDefaults(
  defineProps<{
    /** 侧边栏类别项（后端下发的 VDFS 挂载点清单；不传 = 本页无侧边栏） */
    railItems?: NavRailItem[]
    /** 侧边栏返回键（容器页用） */
    back?: boolean
    backTitle?: string
    /** —— 以下透传 VdfsShell（资源页模式）—— */
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

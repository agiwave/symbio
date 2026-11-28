<!--
  VdfsCardSkeleton — 列表骨架卡（形状对齐 `VdfsCard`）

  逐行复刻 `VdfsCard` 的盒模型（同 margin / padding / 边框 / 圆角）与内部
  三行结构（状态点+图标+标题 / 副标题 / 元信息），因此在加载态与内容态之间
  切换时**高度与左侧起点都不动**——这正是骨架屏相对「加载中…」的全部价值。

  形状变化由 `variant` 控制（列表里的卡片并不都一样高：会话卡有副标题，
  设置分区卡没有）。默认交给调用方按目录类型选择，本组件不猜。

  为什么与 `VdfsCard` 分文件而不是给它加 `loading` 开关：卡片是**数据**的
  呈现（它的每个 prop 都对应节点字段），骨架没有数据。混在一起会让卡片的
  prop 出现「有些只在 loading 时有效」的半可用组合。
-->
<template>
  <div class="vdfs-card-skeleton" aria-hidden="true">
    <div class="skeleton-head">
      <span v-if="withMeta" class="skeleton-dot" />
      <SkeletonBlock class="skeleton-icon" :width="'1rem'" :height="1" />
      <SkeletonBlock :width="titleWidth" />
    </div>
    <SkeletonBlock v-if="withMeta" :width="'72%'" :height="0.6" />
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import SkeletonBlock from './SkeletonBlock.vue'

const props = withDefaults(
  defineProps<{
    /** 标题占位宽度（语义化比例，避免每张卡看起来一样长） */
    titleWidth?: string
    /** 是否带第二行（副标题 / 元信息）；会话与多数资源卡为 true */
    withMeta?: boolean
  }>(),
  {
    titleWidth: '58%',
    withMeta: true,
  }
)

/** 骨架点只在有副标题的形态下出现（与 VdfsCard 的 status-area 对齐） */
const withMeta = computed(() => props.withMeta)
</script>

<style scoped>
/* 与 VdfsCard 的盒模型逐条对齐——复制的是"形状"，不是"视觉" */
.vdfs-card-skeleton {
  margin: var(--space-1) var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.skeleton-head {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

/* 与 VdfsCard 的 .status-dot 同尺寸（0.4375rem），保证标题左缘对齐 */
.skeleton-dot {
  display: inline-block;
  width: 0.4375rem;
  height: 0.4375rem;
  border-radius: var(--radius-full);
  background: var(--surface-sunken);
  flex-shrink: 0;
}

.skeleton-icon {
  flex-shrink: 0;
  width: 1rem;
}
</style>

<!--
  SkeletonBlock — 骨架屏基本块（机制内置，唯一实现）

  一个宽度可调的灰条，自带微光动画；`lines` 可堆成多行（末行自动收短，
  模拟真实文本的参差收尾，比等宽多行更像内容）。

  为什么要有它：原来的加载态是纯文字「加载中…」，在列表与详情两处都出现。
  纯文字的问题不是难看，是**布局跳动**——文字态与内容态的高度往往不等，
  内容一到整页就重新排版，用户的视线要重新找位置。骨架屏按内容形状占位，
  数据到达时是「填充」而不是「重排」。

  动画遵守 `prefers-reduced-motion`：骨架子在不静止时反而更难读。

  颜色一律走 token（`--surface-sunken` 底 + `--surface-hover` 微光），
  故明暗两态自动成立，组件不持有任何硬编码色值。
-->
<template>
  <div class="skeleton" :class="{ 'is-inline': inline }" aria-hidden="true">
    <span
      v-for="i in lineCount"
      :key="i"
      class="skeleton-line"
      :style="{ width: widthOf(i), height: `${height}rem` }"
    />
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{
    /** 行数（>1 时末行自动收窄到 62%，模拟文本收尾） */
    lines?: number
    /** 行高（rem） */
    height?: number
    /** 每行宽度。传字符串则所有行同宽；不传则按「末行收窄」规则生成 */
    width?: string
    /** 行内形态：与相邻元素同行排列（不占满一行） */
    inline?: boolean
  }>(),
  {
    lines: 1,
    height: 0.75,
    width: '',
    inline: false,
  }
)

const lineCount = computed(() => Math.max(1, Math.floor(props.lines)))

/** 末行宽度：多行时收窄，读起来像一段真文本而不是一排等长色块 */
const LAST_LINE_RATIO = 0.62

function widthOf(i: number): string {
  if (props.width) return props.width
  if (lineCount.value > 1 && i === lineCount.value) return `${LAST_LINE_RATIO * 100}%`
  return '100%'
}
</script>

<style scoped>
.skeleton {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
  width: 100%;
}

.skeleton.is-inline {
  width: auto;
  flex-direction: row;
  align-items: center;
}

.skeleton-line {
  display: block;
  border-radius: var(--radius-sm);
  background: var(--surface-sunken);
  position: relative;
  overflow: hidden;
}

/*
 * 微光：一道 40% 宽的亮带从左扫到右。
 * 用 transform 而非 background-position——后者每帧重绘背景，长列表下更贵。
 */
.skeleton-line::after {
  content: '';
  position: absolute;
  inset: 0;
  transform: translateX(-100%);
  background: linear-gradient(
    90deg,
    transparent 0%,
    var(--surface-hover) 50%,
    transparent 100%
  );
  animation: skeleton-sweep 1.4s var(--motion-ease) infinite;
}

@keyframes skeleton-sweep {
  100% { transform: translateX(100%); }
}

@media (prefers-reduced-motion: reduce) {
  .skeleton-line::after { animation: none; }
}
</style>

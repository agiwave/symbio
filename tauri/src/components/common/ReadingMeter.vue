<!--
  ReadingMeter — 读数分区（`ReadingSection`）的**紧凑渲染器**：一行字 + 一条几何

  ## 它是谁

  与 `components/common/ReadingPanel.vue` 同一个方言的两个渲染器：面板把一份读数的
  **所有分区**铺开，本组件把**一个分区**压成一行——用在「地方只有一条，但必须在眼前」
  的位置（输入区上方）。两者都只是排版，都不认识数据说的是什么。

  ## 形状里没有业务词（机制化的判据）

  数据是宿主方言的一格 `ReadingSection`（后端在交付出口现算，`present/reading.rs` 是
  形状的 owner）：

  - **行**逐行排：标签、值的单位与精度、语气（`warn`）、口径提示（`hint`）都由产者判；
    本组件不认「已用」「上限」「阈值」任何一个词，后端多一行少一行这里零改动；
  - **几何**走 `bar`：`level` 是填充位置、`marks` 是参考线位置，全部是产者**已除好的
    0..1 比率**。这里的 `× 100` 是 ratio→% 的单位换算，不是第二次除法——除法一旦有
    两处，改系数就只有一半界面跟着变（表现为「界面上还没到线，后端已经动手」）；
  - **着色只由产者的 `tone` 决定**，不由任何阈值决定：组件里出现 `level > 0.7`
    就等于把判据搬进渲染器，而它住在拿着数的那一方；
  - 没有 `bar` 也照常排行（几何是呈现提示，不是新事实）；没有这一格就**整条不渲染**。

  ## 为什么参考线常显

  `marks` 说的是「到这儿产者会自己接手」，不是完成度。它越界也要留着——隐藏它会把
  「它已经动手了」这件最需要看见的事藏起来。
-->
<template>
  <div v-if="section" class="reading-meter" :title="summary">
    <div v-if="section.bar" class="meter-track" :aria-label="summary" role="img">
      <div
        class="meter-fill"
        :class="{ 'is-warn': barWarn }"
        :style="{ width: pct(section.bar.level) }"
      />
      <div
        v-for="(m, i) in section.bar.marks ?? []"
        :key="i"
        class="meter-tick"
        :style="{ left: pct(m) }"
      />
    </div>
    <div class="meter-rows">
      <span
        v-for="r in section.rows"
        :key="r.label"
        class="meter-row"
        :class="{ 'is-warn': r.tone === 'warn' }"
        :title="r.hint"
      >
        <span class="row-label">{{ r.label }}</span>
        <span class="row-value">{{ r.value }}</span>
      </span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ReadingSection } from '@/schemas/reading'

const props = defineProps<{
  /** 后端下发的一格读数；`undefined` = 这里没有数（整条不渲染，而不是画一条空条） */
  section?: ReadingSection
}>()

/** ratio → %：唯一的「换算」，不含任何判据（缺 ratio 时条不表态，画成 0）。 */
function pct(ratio?: number): string {
  return `${(ratio ?? 0) * 100}%`
}

/** 条的语气只看产者给的那一位（行与条由产者同时转色，见 `context_reading_of`）。 */
const barWarn = computed(() => props.section?.bar?.tone === 'warn')

/** 无障碍摘要 = 逐行「标签 值」（与面板里的 `stats` 行同一个读法，不另造一句话）。 */
const summary = computed(() =>
  (props.section?.rows ?? [])
    .map((r) => (r.value ? `${r.label} ${r.value}` : r.label))
    .join(' · ')
)
</script>

<style scoped>
.reading-meter {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 0 var(--space-1) var(--space-1);
}

.meter-track {
  position: relative;
  flex: 0 0 4.5rem;
  height: 3px;
  border-radius: var(--radius-full);
  background: var(--surface-hover);
  overflow: hidden;
}

.meter-fill {
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--accent);
  transition: width var(--motion-base) var(--motion-ease);
}

.meter-fill.is-warn {
  background: var(--warning-solid);
}

/* 参考线画在轨道内缘：1px 贴在裁切层里仍然可见，越界也不隐藏（见文件头的「为什么常显」）。 */
.meter-tick {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--border-default);
}

.meter-rows {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}

.meter-row {
  display: inline-flex;
  align-items: baseline;
  gap: var(--space-1);
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  white-space: nowrap;
}

.meter-row.is-warn {
  color: var(--warning-fg);
}

.row-label {
  font-weight: var(--font-weight-medium);
}

.row-value {
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
}
</style>

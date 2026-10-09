<!--
  ReadingPanel — 读数面板的通用渲染器（机制内置，唯一实现）

  消费后端下发的 ReadingPanel（`schemas/reading.ts`），**零业务知识**：
  本组件不认识「会话」「档位」「兜底率」中的任何一个，只按分区/行排版。
  产者换一个（成本、记忆、任何只读路由）本组件零改动。

  与 `vdfs/DetailForm` 的分工：那个渲染**可编辑**的详情表单（widget 分派、校验、
  保存），本组件渲染**只读**的指标读数。两者共用同一条哲学——后端下发定义，
  前端不把口径抄进第二处（否则读数一改，抄本不报错、只静静显示旧标签）。

  ## 前端不做的事

  - 不做数值格式化（值与单位由产者排成字符串——「P99 显示 ms 还是 s」
    是产者的口径，前端再定一次就是两处）；
  - 不解析值的语义定语气（`tone` 由产者判，它拿着数才知道 2 个未收束轮
    是不是问题；前端只按 tone 换样式）；
  - 不决定列哪些行（分区与行全由后端给——本组件不排序不过滤）。
-->
<template>
  <section class="reading-panel" :class="{ 'has-warn': warn }">
    <header v-if="panel.title || panel.subtitle" class="rp-head">
      <h3 class="rp-title">{{ panel.title }}</h3>
      <span v-if="panel.subtitle" class="rp-subtitle">{{ panel.subtitle }}</span>
      <span v-if="warn" class="rp-flag" title="有需留意读数">●</span>
    </header>

    <!-- 空态：没有可报的。**不是错误**——与后端 has_wal=false 时各列「有据的零」
         同一条判定：没有数与读不到是两件事。
         两种空分开说：一个分区都没有 vs 有分区但一行都没排——前者是「这个会话
         还没有任何读数」，后者是「后端给了骨架但没给行」，对用户是不同的话。 -->
    <template v-if="sections.length === 0">
      <p class="rp-empty">暂无可报读数</p>
    </template>
    <template v-else-if="rows === 0">
      <p class="rp-empty">有分区但没有可报的行</p>
    </template>

    <div v-for="(sec, i) in sections" :key="i" class="rp-section">
      <h4 v-if="sec.title" class="rp-section-title">{{ sec.title }}</h4>
      <dl class="rp-rows">
        <div v-for="(row, j) in sec.rows" :key="j" class="rp-row" :class="toneClass(row.tone)">
          <dt class="rp-label">{{ row.label }}</dt>
          <dd class="rp-value">{{ row.value }}</dd>
          <dd v-if="row.hint" class="rp-hint">{{ row.hint }}</dd>
        </div>
      </dl>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import {
  panelHasWarn,
  panelRowCount,
  type ReadingPanel,
  type ReadingTone,
} from '@/schemas/reading'

const props = defineProps<{
  /** 后端下发的面板定义（本渲染器的唯一形态来源） */
  panel: ReadingPanel
}>()

/** 分区直接取，空 sections 归一到空数组（模板里三处都要判，收在一处）。 */
const sections = computed(() => props.panel.sections ?? [])

/** 标题处的提示点：有需留意读数就亮。语气由产者判，这里只读它。 */
const warn = computed(() => panelHasWarn(props.panel))

/** 行数（含嵌套）：用来把「没有分区」与「有分区但没行」分开说——那对用户是
 *  两种不同的空，混成一句会把「后端给错了」显示成「本来就没数」。 */
const rows = computed(() => panelRowCount(props.panel))

/**
 * 语气 → 具名类。**两个字面量写死在这里**而不是模板里拼 tone 字符串：
 * style-audit 认的是字面类名，拼出来的 `${tone}` 它扫不到（实测会报
 * 「类名 .normal 使用了但未定义」），而那正是一种**静默**——类存在与否
 * 没有任何东西在验。写死之后漏一条就是守卫红。
 */
function toneClass(tone: ReadingTone | undefined): string {
  return tone === 'warn' ? 'is-warn' : 'is-normal'
}
</script>

<style scoped>
.reading-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.rp-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.rp-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
}

.rp-subtitle {
  font-size: 11px;
  color: var(--text-secondary, #999);
  font-family: ui-monospace, monospace;
}

/* 提示点：语气只在这里被翻译成视觉，语义不进来。 */
.rp-flag {
  color: var(--warning-fg);
  font-size: 10px;
}

.reading-panel.has-warn .rp-title {
  color: var(--warning-fg);
}

.rp-empty {
  margin: 0;
  font-size: 12px;
  color: var(--text-secondary, #999);
}

.rp-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.rp-section-title {
  margin: 0;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary, #999);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.rp-rows {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.rp-row {
  display: grid;
  grid-template-columns: minmax(72px, auto) 1fr;
  gap: 2px 10px;
  padding: 3px 0;
  border-bottom: 1px solid var(--border-subtle, rgba(128, 128, 128, 0.12));
}

.rp-row:last-child {
  border-bottom: none;
}

/* 平常语气：没有额外样式，但**必须有这条规则**——
   否则 toneClass 给出的 `is-normal` 成了未定义类名（style-audit 会红），
   而「未定义」与「不需要样式」在视觉上分不开。 */
.rp-row.is-normal {
  /* 平常：值用继承色 */
}

.rp-label {
  font-size: 12px;
  color: var(--text-secondary, #999);
}

.rp-value {
  margin: 0;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

/* 口径提示占整行、视觉次一级——它是补充说明，不是第二个值。 */
.rp-hint {
  grid-column: 2;
  margin: 0;
  font-size: 10px;
  color: var(--text-secondary, #999);
  opacity: 0.75;
}

.rp-row.is-warn .rp-value {
  color: var(--warning-fg);
}
</style>

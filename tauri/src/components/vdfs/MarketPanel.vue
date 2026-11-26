<!--
  MarketPanel — 插件市场（plugin_manager 的 `ext=market` 渲染器）

  候选由后端随「添加插件」新建类型的 `schema` 下发（`{ candidates: [...] }`），
  本组件只负责呈现与发起安装，不持有第二份插件清单。它不是 VDFS 清单节点，而是一张
  草稿页：点击安装向宿主抛 `save`，由通用写入机制落到当前目录（= 容器安装动词），
  成功后广播变更，列表与候选随之收敛。

  外部插件的形态仍由  暂缓；候选里的 `source` 字段是它与内置插件共用的呈现位，
  这里不据此分叉出任何恒空分支。
-->
<template>
  <section class="market">
    <header class="market-head">
      <h2 class="market-title">插件市场</h2>
      <p class="market-desc">添加尚未安装的可选插件。已安装的插件可在列表中卸载。</p>
    </header>

    <p v-if="props.error" class="market-error" role="alert">{{ props.error }}</p>

    <div v-if="visible.length" class="market-grid">
      <article v-for="c in visible" :key="c.name" class="market-card">
        <div class="card-head">
          <h3 class="card-name">{{ c.label || c.name }}</h3>
          <span class="card-source">{{ sourceLabel(c.source) }}</span>
        </div>
        <code class="card-id">{{ c.name }}</code>
        <button type="button" class="install-btn" @click="install(c)">安装</button>
      </article>
    </div>

    <p v-else class="market-empty">暂无可安装的可选插件。</p>
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { VdfsRendererProps } from './rendererContract'

const props = defineProps<VdfsRendererProps>()
const emit = defineEmits<{ (e: 'save', payload: Record<string, unknown>): void }>()

interface Candidate {
  name: string
  provider: string
  source: string
  label: string
}

/** schema 模型：候选 + 安装载荷的键名（后者由后端唯一定义，见 `VDFS_PLUGIN_PROVIDER_FIELD`）。 */
type MarketSchema = { candidates?: unknown; provider_field?: unknown }

const schema = computed<MarketSchema | undefined>(
  () => props.data as MarketSchema | undefined,
)

/** 候选来自 `new_type.schema`（不是正文）：机制把它作为 `data` 原样交给渲染器。 */
const candidates = computed<Candidate[]>(() => {
  const list = schema.value && typeof schema.value === 'object' ? schema.value.candidates : null
  return Array.isArray(list) ? (list as Candidate[]) : []
})

/**
 * 安装载荷的键名由后端下发，前端不硬编码——两侧各写一遍正是「保存失败：请选择插件工厂」
 * 的根因（前端曾写 `plugin_provider`，后端读 `provider`）。后端缺席时用一个不可能被
 * 容器识别的哨兵键，让失败原因如实暴露为「请选择一个插件工厂」，而不是悄悄装错。
 */
const providerField = computed<string>(() => {
  const f = schema.value && typeof schema.value === 'object' ? schema.value.provider_field : null
  return typeof f === 'string' && f ? f : '__provider_field_missing__'
})

const visible = candidates

function sourceLabel(source: string): string {
  return source === 'builtin' ? '内置可选' : source || '未知来源'
}

/**
 * 点安装 = 提交一张「新建插件」草稿：只抛载荷，写入由宿主统一处理。
 *
 * 成功后的列表刷新 / 关闭草稿都由 `useVdfs.write` 完成；失败原因走 `error` prop
 * 回到页面上，不在渲染器里再写一份 VDFS。
 */
function install(c: Candidate): void {
  emit('save', { [providerField.value]: c.provider })
}
</script>

<style scoped>
.market {
  padding: 20px;
  overflow-y: auto;
  height: 100%;
}

.market-head {
  margin-bottom: 18px;
}

.market-title {
  margin: 0 0 6px;
  font-size: var(--font-size-lg, 18px);
  color: var(--text-primary);
}

.market-desc {
  margin: 0;
  font-size: var(--font-size-sm, 13px);
  color: var(--text-secondary);
}

.market-error {
  padding: 10px 12px;
  margin: 0 0 16px;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md, 8px);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-size: var(--font-size-sm, 13px);
}

.market-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 12px;
}

.market-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md, 8px);
  background: var(--surface-panel);
}

.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.card-name {
  margin: 0;
  font-size: var(--font-size-base, 14px);
  font-weight: var(--font-weight-semibold, 600);
  color: var(--text-primary);
}

.card-source {
  flex-shrink: 0;
  padding: 2px 6px;
  border-radius: var(--radius-sm, 4px);
  background: var(--surface-sunken);
  color: var(--text-muted);
  font-size: var(--font-size-xs, 11px);
}

.card-id {
  color: var(--text-muted);
  font-family: var(--font-mono, monospace);
  font-size: var(--font-size-xs, 11px);
}

.install-btn {
  align-self: flex-start;
  padding: 6px 14px;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md, 8px);
  background: var(--accent);
  color: var(--surface-panel);
  font-size: var(--font-size-sm, 13px);
  cursor: pointer;
}

.install-btn:disabled {
  opacity: 0.6;
  cursor: progress;
}

.market-empty {
  color: var(--text-muted);
  font-size: var(--font-size-sm, 13px);
}
</style>

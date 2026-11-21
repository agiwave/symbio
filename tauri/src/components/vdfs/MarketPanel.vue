<!--
  MarketPanel — 插件市场（plugin_manager 的 `ext=market` 渲染器）

  节点正文是后端现场算出的候选 JSON（`{ candidates: [...] }`），本组件只负责呈现与
  发起安装，不持有第二份插件清单。安装写回市场节点：后端校验「可选 ∧ 未安装」后
  复用容器根的安装动词，成功后广播变更，列表与候选随之收敛。

  外部插件的形态仍由 D-407 暂缓；候选里的 `source` 字段是它与内置插件共用的呈现位，
  这里不据此分叉出任何恒空分支（D-520）。
-->
<template>
  <section class="market">
    <header class="market-head">
      <h2 class="market-title">插件市场</h2>
      <p class="market-desc">添加尚未安装的可选插件。已添加的插件在左侧列表中可以卸载。</p>
    </header>

    <p v-if="error" class="market-error" role="alert">{{ error }}</p>

    <div v-if="visible.length" class="market-grid">
      <article v-for="c in visible" :key="c.name" class="market-card">
        <div class="card-head">
          <h3 class="card-name">{{ c.label || c.name }}</h3>
          <span class="card-source">{{ sourceLabel(c.source) }}</span>
        </div>
        <code class="card-id">{{ c.name }}</code>
        <button
          type="button"
          class="install-btn"
          :disabled="busyId !== null"
          @click="install(c)"
        >
          {{ busyId === c.name ? '安装中…' : '安装' }}
        </button>
      </article>
    </div>

    <p v-else class="market-empty">暂无可安装的可选插件。</p>
  </section>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { writeVdfs } from '@/services/vdfs'
import type { VdfsRendererProps } from './rendererContract'

const props = defineProps<VdfsRendererProps>()

interface Candidate {
  name: string
  provider: string
  source: string
  label: string
}

const candidates = computed<Candidate[]>(() => {
  if (typeof props.data !== 'string') return []
  try {
    const model = JSON.parse(props.data) as { candidates?: Candidate[] }
    return Array.isArray(model.candidates) ? model.candidates : []
  } catch {
    return []
  }
})

/** 乐观收敛：安装成功后先把这张卡移走，广播到达后的重拉会给出同一份读数 */
const locallyInstalled = ref<Set<string>>(new Set())
const visible = computed(() => candidates.value.filter((c) => !locallyInstalled.value.has(c.name)))
const busyId = ref<string | null>(null)
const error = ref('')

function sourceLabel(source: string): string {
  return source === 'builtin' ? '内置可选' : source || '未知来源'
}

async function install(c: Candidate): Promise<void> {
  if (busyId.value !== null) return
  busyId.value = c.name
  error.value = ''
  try {
    await writeVdfs(props.node.path, JSON.stringify({ plugin_provider: c.provider }))
    locallyInstalled.value = new Set([...locallyInstalled.value, c.name])
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busyId.value = null
  }
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

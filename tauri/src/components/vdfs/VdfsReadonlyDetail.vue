<!--
  VdfsReadonlyDetail — VDFS 只读兜底渲染器（ext = dir / 未命中 / 无 ext）

  节点没有任何专属渲染器时，退化为「机制级只读视图」：展示节点自身的
  机制字段（路径 / 类型 / 状态 / 访问位 / 大小 / 更新时间）与场景扩展字段。
  这是资源管理器永不空白的原因，也是新增资源在补齐专属渲染器前的可用形态。

  本渲染器**没有自有动作**：能做的只有机制级默认动作（重命名 / 删除，判据在
  `useVdfs.mechanismActions`），故动作区完全由注入项构成。
-->
<template>
  <DetailShell
    :title="node.title || node.name"
    :mechanism-actions="mechanismActions"
    :mechanism-busy="mechanismBusy"
    :error="error"
    @run="onAction"
  >
    <template #meta>
      <span class="kind">{{ node.kind }}</span>
    </template>

    <p v-if="node.description" class="desc">{{ node.description }}</p>

    <dl class="meta">
      <div class="meta-row"><dt>路径</dt><dd><code>{{ node.path }}</code></dd></div>
      <div class="meta-row"><dt>状态</dt><dd>{{ node.status }}</dd></div>
      <div class="meta-row"><dt>访问位</dt><dd><code>{{ node.access || '—' }}</code> {{ accessText }}</dd></div>
      <div v-if="node.ext" class="meta-row"><dt>呈现扩展名</dt><dd>{{ node.ext }}</dd></div>
      <div v-if="typeof node.size === 'number'" class="meta-row"><dt>大小</dt><dd>{{ node.size }} 字节</dd></div>
      <div v-if="node.updated_at" class="meta-row"><dt>更新时间</dt><dd>{{ updatedText }}</dd></div>
    </dl>

    <div v-if="attributes.length">
      <p class="attrs-title">扩展字段</p>
      <dl class="meta">
        <div v-for="a in attributes" :key="a.key" class="meta-row">
          <dt>{{ a.key }}</dt>
          <dd>
            <!-- 对象 / 数组值此前被一行 JSON.stringify 平铺：大对象会把后面所有
                 字段推到很远。默认只显摘要，展开才看完整 JSON。 -->
            <details v-if="a.objectKind" class="attr-object">
              <summary>{{ a.objectKind }}</summary>
              <pre class="attr-json">{{ a.value }}</pre>
            </details>
            <template v-else>{{ a.value }}</template>
          </dd>
        </div>
      </dl>
    </div>
  </DetailShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import DetailShell from './DetailShell.vue'
import type { VdfsRendererProps } from './rendererContract'
import { vdfsAccessOf, type DetailAction } from '@/schemas/vdfs'
import { formatDateTime } from '@/utils/time'

const props = defineProps<VdfsRendererProps>()

const emit = defineEmits<{
  (e: 'save', payload: unknown): void
  (e: 'delete'): void
}>()

const access = computed(() => vdfsAccessOf(props.node))

/** 本渲染器无自有动作：机制动作原样上抛给页面执行 */
function onAction(a: DetailAction): void {
  if (a.id === 'delete') emit('delete')
}

const accessText = computed(() => {
  const a = access.value
  const parts: string[] = []
  if (a.read) parts.push('读')
  if (a.write) parts.push('写')
  if (a.list) parts.push('列目录')
  if (a.traverse) parts.push('深度遍历')
  return parts.length ? `（${parts.join(' / ')}）` : '（无）'
})

const updatedText = computed(() => formatDateTime(props.node.updated_at))

/** 场景扩展字段（协议保留字段之外的项，即 flatten 到顶层的 attributes） */
const RESERVED = new Set([
  'path', 'name', 'title', 'description', 'kind', 'status', 'access',
  'ext', 'size', 'updated_at', 'children', 'binary', 'schema', 'new_type',
])
const attributes = computed(() =>
  Object.entries(props.node)
    .filter(([k]) => !RESERVED.has(k))
    .map(([k, v]) => {
      const isObject = v !== null && typeof v === 'object'
      return {
        key: k,
        value: v == null ? '—' : isObject ? JSON.stringify(v, null, 2) : String(v),
        /** 对象值的摘要（展开前的可读规模）；非对象为空串 */
        objectKind: !isObject
          ? ''
          : Array.isArray(v)
            ? `数组（${v.length} 项）`
            : `对象（${Object.keys(v as Record<string, unknown>).length} 个字段）`,
      }
    })
)
</script>

<style scoped>
.kind {
  font-size: 0.65rem;
  padding: 0.1rem 0.45rem;
  border-radius: var(--radius-full);
  background: var(--accent-subtle-bg);
  color: var(--accent);
}

.desc {
  margin: 0;
  font-size: 0.8rem;
  color: var(--text-secondary);
  line-height: 1.5;
}

.meta {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.meta-row {
  display: flex;
  gap: 0.75rem;
  font-size: 0.78rem;
}

.meta-row dt {
  width: 6.5rem;
  flex-shrink: 0;
  color: var(--text-muted);
}

.meta-row dd {
  margin: 0;
  color: var(--text-primary);
  word-break: break-all;
}

.meta-row code {
  font-family: var(--font-mono);
}

.attrs-title {
  margin: 0 0 0.25rem;
  font-size: 0.75rem;
  color: var(--text-muted);
}

.attr-object summary {
  cursor: pointer;
  color: var(--accent);
  font-size: 0.78rem;
}
.attr-json {
  margin: 0.35rem 0 0;
  padding: 0.5rem 0.6rem;
  max-height: 18rem;
  overflow: auto;
  background: var(--color-code-bg);
  color: var(--color-code-fg);
  border-radius: var(--radius-md);
  font-family: var(--font-mono);
  font-size: 0.74rem;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>

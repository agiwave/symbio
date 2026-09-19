<!--
  VdfsTextDetail — VDFS 文本类渲染器（ext ∈ text | json | md）

  节点的内容经 `vdfs/read` 取回后由本组件承载编辑；保存 emit 纯文本，
  由页面写回 `vdfs/write`。本组件不含任何资源语义，渲染器对所有挂载点通用。

  动作区只有**本渲染器自有**的两项（保存 / 还原）；重命名、删除是机制级默认
  动作，由页面单点算好后经 `mechanism-actions` 注入，由 DetailShell 合并渲染
  （渲染器不再自己拼一遍——那曾是同一组动作的第 3、4 份实现）。
-->
<template>
  <DetailShell
    :title="node.title || node.name"
    :actions="actions"
    :busy="busy"
    :disabled="disabledFlags"
    :mechanism-actions="mechanismActions"
    :mechanism-busy="mechanismBusy"
    :error="error"
    @run="onAction"
  >
    <template #meta>
      <code class="path">{{ node.path }}</code>
    </template>

    <div class="editor-wrap">
      <CodeEditor
        :model-value="draft"
        :file-path="node.path"
        :readonly="readonly || saving"
        @update:model-value="draft = $event"
        @request-save="save"
      />
    </div>
  </DetailShell>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import CodeEditor from '@/components/CodeEditor.vue'
import DetailShell from './DetailShell.vue'
import type { VdfsRendererProps } from './rendererContract'
import { vdfsAccessOf, type DetailAction } from '@/schemas/vdfs'

const props = defineProps<VdfsRendererProps>()

const emit = defineEmits<{
  (e: 'save', text: string): void
  (e: 'delete'): void
  (e: 'rename'): void
}>()

const readonly = computed(() => !vdfsAccessOf(props.node).write)
const base = computed(() => (typeof props.data === 'string' ? props.data : ''))
const draft = ref(base.value)
const dirty = computed(() => draft.value !== base.value)

/** 本渲染器自有动作（机制动作由页面注入并合并，见 DetailShell） */
const actions = computed<DetailAction[]>(() => [
  { id: 'save', label: '保存', style: 'primary', busy_label: '保存中…' },
  { id: 'reset', label: '还原', style: 'secondary' },
])

/** 自有动作的进行中标记（按索引对齐；写操作在途时两者都锁） */
const busy = computed(() => actions.value.map(() => Boolean(props.saving)))

/** 自有动作的禁用条件（保存键在无改动或只读时不可用） */
const disabledFlags = computed(() =>
  actions.value.map((a) => (a.id === 'save' ? !dirty.value || readonly.value : false))
)

/** 自有动作就地处理；机制动作（rename / delete）原样上抛给页面 */
function onAction(a: DetailAction): void {
  if (a.id === 'save') save()
  else if (a.id === 'reset') reset()
  else if (a.id === 'rename') emit('rename')
  else if (a.id === 'delete') emit('delete')
}

// 节点身份或后端内容变化时重置草稿（编辑中不覆盖：仅当节点切换/保存回调后同步）
watch(
  () => [props.node.path, base.value] as const,
  () => {
    draft.value = base.value
  }
)

function save() {
  if (readonly.value) return
  emit('save', draft.value)
}
function reset() {
  draft.value = base.value
}
</script>

<style scoped>
.path {
  font-size: 0.68rem;
  font-family: var(--font-mono);
  color: var(--text-muted);
  background: var(--surface-sunken);
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-sm);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 编辑区吃掉剩余高度（CodeEditor 自带滚动条），外壳因此不会被撑出滚动 */
.editor-wrap {
  flex: 1;
  min-height: 14rem;
  display: flex;
}

.editor-wrap :deep(.code-editor) {
  flex: 1;
  height: 100%;
}
</style>

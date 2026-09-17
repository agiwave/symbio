<!--
  VdfsTextDetail — VDFS 文本类渲染器（ext ∈ text | json | md）

  节点的内容经 `vdfs/read` 取回后由本组件承载编辑；保存 emit 纯文本，
  由页面写回 `vdfs/write`。本组件不含任何资源语义，渲染器对所有挂载点通用。
-->
<template>
  <div class="vdfs-text">
    <header class="detail-head">
      <div class="head-title">
        <h3 class="title">{{ node.title || node.name }}</h3>
        <code class="path">{{ node.path }}</code>
      </div>
      <div class="head-actions">
        <button class="action-btn" :disabled="saving || !dirty || readonly" @click="save">
          {{ saving ? '保存中…' : '保存' }}
        </button>
        <button class="action-btn secondary" :disabled="saving" @click="reset">还原</button>
        <button v-if="!readonly" class="action-btn secondary" :disabled="saving" @click="$emit('rename')">重命名</button>
        <button v-if="!readonly" class="danger-btn" :disabled="saving" @click="$emit('delete')">删除</button>
      </div>
    </header>

    <p v-if="error" class="detail-error">{{ error }}</p>

    <div class="editor-wrap">
      <CodeEditor
        :model-value="draft"
        :file-path="node.path"
        :readonly="readonly || saving"
        @update:model-value="draft = $event"
        @request-save="save"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import CodeEditor from '@/components/CodeEditor.vue'
import { vdfsAccessOf, type VdfsFieldError, type VdfsNode } from '@/schemas/vdfs'

// 渲染器统一契约（页面按同一组 props/事件装配；未用到的项一并声明，
// 避免 Vue 把多余的 prop/监听器作为属性透传到根元素）
const props = defineProps<{
  node: VdfsNode
  /** vdfs/read 取回的文本内容 */
  data: unknown
  error?: string
  fieldErrors?: VdfsFieldError[]
  saving?: boolean
}>()

const emit = defineEmits<{
  (e: 'save', text: string): void
  (e: 'delete'): void
  (e: 'rename'): void
}>()

const readonly = computed(() => !vdfsAccessOf(props.node).write)
const base = computed(() => (typeof props.data === 'string' ? props.data : ''))
const draft = ref(base.value)
const dirty = computed(() => draft.value !== base.value)

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
.vdfs-text {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  padding: 0.75rem 1rem;
  gap: 0.5rem;
}
.detail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  flex-shrink: 0;
  flex-wrap: wrap;
}
.head-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}
.title {
  margin: 0;
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
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
.head-actions {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-shrink: 0;
}
.detail-error {
  margin: 0;
  font-size: 0.8rem;
  color: var(--danger-fg);
}
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

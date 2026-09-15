<!--
  OptionFormDialog — 级联选项机制中 `form` 类型选项的通用承载

  职责单一：把选项节点自带的 `form`（DetailDefinition，与资源详情表单同一套
  schema）交给唯一渲染器 DetailForm 渲染，绑定模式 `option`：

  - 预填：节点 `data` → DetailForm 的 `values`（表单模型对象）；
  - 保存：DetailForm 回吐纯字段值 → 本组件上抛 `save`，由父级（ChatOptionBar）
    经 `useSessionOptions.dispatch` 写入 `action.bind` 并调用后端服务。

  本组件不含任何业务字段；新增表单型选项 = 后端下发定义即可，前端零改动。
-->
<template>
  <Teleport to="body">
    <div class="ofd-overlay" @click.self="$emit('close')">
      <div class="ofd-dialog">
        <DetailForm
          v-if="definition"
          :definition="definition"
          :node="null"
          :values="node.data ?? {}"
          :capabilities="EMPTY_CAPABILITIES"
          :saving="saving"
          @save="onSave"
          @cancel="$emit('close')"
        />
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import DetailForm from '@/components/vdfs/DetailForm.vue'
import type { DetailDefinition } from '@/schemas/vdfs'
import type { OptionNode } from '@/schemas/options'

const props = defineProps<{
  /** 表单型选项节点（`form` / `data` / `action` 由后端下发） */
  node: OptionNode
  /** 保存进行中 */
  saving?: boolean
}>()

const emit = defineEmits<{
  close: []
  /** 表单字段值（已序列化）→ 由父级按 action.bind 落库并调用后端服务 */
  save: [values: Record<string, unknown>]
}>()

/** 选项表单无资源语义：能力全关（不渲染删除等机制动作） */
const EMPTY_CAPABILITIES: Record<string, boolean> = {
  mutable: false,
  test_connection: false,
}

/**
 * 注入机制动作「取消」：表单选项的关闭入口与保存同排渲染（复用统一动作区，
 * 避免叠加外框 header 与 DetailForm 既有 header 冲突）。
 */
const definition = computed<DetailDefinition | null>(() => {
  const def = props.node.form
  if (!def) return null
  if ((def.actions ?? []).some((a) => a.id === 'cancel')) return def
  return {
    ...def,
    actions: [...(def.actions ?? []), { id: 'cancel', label: '取消', style: 'default' }],
  }
})

function onSave(values: Record<string, unknown>) {
  emit('save', values)
}
</script>

<style scoped>
.ofd-overlay {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--z-dialog);
}

.ofd-dialog {
  width: 100%;
  max-width: 32rem;
  height: min(38rem, 86vh);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--surface-overlay);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
}
</style>

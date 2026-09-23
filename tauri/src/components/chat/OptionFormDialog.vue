<!--
  OptionFormDialog — 选项栏中 `widget = "form"` 字段的通用承载

  职责单一：把字段自带的**子定义**（`DetailField.form`，与资源详情表单同一套
  schema）交给唯一渲染器 DetailForm 渲染，绑定模式 `option`：

  - 预填：字段当前值（节点 `attributes.metadata[<字段 key>]`）→ DetailForm 的
    `values`（表单模型对象）；
  - 保存：DetailForm 回吐纯字段值 → 本组件上抛 `save`，由父级（ChatOptionBar）
    经 `useSessionOptionBar.save` 写入 `metadata[<字段 key>]`。

  本组件不含任何业务字段；新增表单型选项 = 后端下发定义即可，前端零改动。
-->
<template>
  <Teleport to="body">
    <BaseModal :visible="true" panel-class="ofd-dialog" @close="$emit('close')">
      <DetailForm
        :definition="definition"
        :node="null"
        :values="values"
        :capabilities="EMPTY_CAPABILITIES"
        :saving="saving"
        @save="onSave"
        @cancel="$emit('close')"
      />
    </BaseModal>
  </Teleport>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import BaseModal from '@/components/common/BaseModal.vue'
import DetailForm from '@/components/vdfs/DetailForm.vue'
import type { DetailDefinition } from '@/schemas/vdfs'

const props = defineProps<{
  /** 字段的结构化子定义（`DetailField.form`） */
  definition: DetailDefinition
  /** 子对象当前值（缺省 = 空对象，由 DetailForm 按子定义初始化） */
  values?: Record<string, unknown> | null
  /** 保存进行中 */
  saving?: boolean
}>()

const emit = defineEmits<{
  close: []
  /** 子表单字段值（已序列化）→ 由父级写入 `metadata[<字段 key>]` */
  save: [values: Record<string, unknown>]
}>()

/** 选项表单无资源语义：能力全关（不渲染删除等机制动作） */
const EMPTY_CAPABILITIES: Record<string, boolean> = {
  mutable: false,
  test_connection: false,
}

/**
 * 注入机制动作「取消」：表单字段的关闭入口与保存同排渲染（复用统一动作区，
 * 避免叠加外框 header 与 DetailForm 既有 header 冲突）。
 */
const definition = computed<DetailDefinition>(() => {
  const def = props.definition
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
/* 遮罩与面板底色 / 圆角 / 阴影 / 层级由 `BaseModal` 统一提供；这里只写尺寸与排布。
   顺带补上了原先缺的 ESC 关闭与焦点陷阱（此前只有点遮罩能关）。 */
.ofd-dialog {
  width: 100%;
  max-width: 32rem;
  height: min(38rem, 86vh);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
</style>

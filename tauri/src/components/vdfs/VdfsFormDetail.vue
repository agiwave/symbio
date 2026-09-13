<!--
  VdfsFormDetail — VDFS `form` 渲染器（定义驱动表单）

  职责：把节点的 `schema`（宿主方言的呈现描述，此处为 DetailDefinition）交给
  通用渲染器 DetailForm 动态生成表单，并把保存桥接到 VDFS 传输。

  ## 传输适配（本组件存在的原因）

  DetailForm 的 `config` 绑定自持 load/save_path（那是实体机制的通道）；
  VDFS 通道是 `vdfs/read` / `vdfs/write`。因此本组件把定义适配为
  `binding: 'option'` —— DetailForm 对它的语义恰好是「数据来自外部、保存
  只交回纯字段值」，与 VDFS 完全一致：预填来自 `optionData`（= vdfs/read 的
  解析结果），保存 emit 纯字段值，由页面写回 `vdfs/write`。

  因此 provider 无需为 VDFS 另写一份定义；同一份 DetailDefinition 同时服务
  实体页（config 通道）与 VDFS 页（vdfs 通道），校验仍由 provider 自持。
-->
<template>
  <div class="vdfs-form">
    <!-- 字段级校验横幅：provider 自持校验的产物，机制只负责展示 -->
    <div v-if="error || fieldErrors.length" class="vdfs-banner">
      <p v-if="error" class="banner-msg">{{ error }}</p>
      <ul v-if="fieldErrors.length" class="banner-fields">
        <li v-for="f in fieldErrors" :key="f.field">
          <code>{{ f.field }}</code>
          <span>{{ f.message }}</span>
        </li>
      </ul>
    </div>
    <DetailForm
      :definition="definition"
      :item="item"
      :option-data="optionData"
      :capabilities="capabilities"
      :saving="saving"
      @option-save="(v) => $emit('save', v)"
    />
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import DetailForm from '@/components/entities/DetailForm.vue'
import type { DetailDefinition, EntityCapabilities, EntitySummary } from '@/schemas/entities'
import { vdfsAccessOf, type VdfsFieldError, type VdfsNode } from '@/schemas/vdfs'

const props = withDefaults(
  defineProps<{
    node: VdfsNode
    /** vdfs/read 的文本按 JSON 解析后的字段值 */
    data: unknown
    error?: string
    fieldErrors?: VdfsFieldError[]
    saving?: boolean
  }>(),
  { error: '', fieldErrors: () => [], saving: false }
)

defineEmits<{
  (e: 'save', values: Record<string, unknown>): void
  (e: 'delete'): void
  /** 节点重命名由页面级内联栏承载；此处仅声明以对齐渲染器统一契约 */
  (e: 'rename'): void
}>()

const access = computed(() => vdfsAccessOf(props.node))

/**
 * 通道适配：清掉实体机制的 load/save_path，改由 VDFS 承载数据。
 * 不可写节点同时剥掉定义动作（避免渲染出无效的「保存」按钮）。
 */
const definition = computed<DetailDefinition>(() => {
  const raw = (props.node.schema ?? {}) as DetailDefinition
  return {
    ...raw,
    binding: 'option',
    load_path: undefined,
    save_path: undefined,
    actions: access.value.write ? raw.actions : [],
  }
})

const optionData = computed<Record<string, unknown> | null>(() =>
  props.data && typeof props.data === 'object' ? (props.data as Record<string, unknown>) : null
)

/** 节点 → 实体摘要（仅供 DetailForm 读取 id/name 参与标题回落链） */
const item = computed<EntitySummary>(() => ({
  kind: props.node.kind,
  id: props.node.name,
  name: props.node.title || props.node.name,
  status: props.node.status,
}))

/** 访问位 → 能力（VDFS 里能力就是访问位，不存在类型特判） */
const capabilities = computed<EntityCapabilities>(() => ({
  zip_upload: false,
  independent_form: true,
  realtime_status: false,
  refreshable: true,
  mutable: access.value.write,
  test_connection: false,
  read_only: !access.value.write,
}))

// 不注入机制动作（重命名 / 删除）：form 型节点是 provider 声明固定清单的项
// （如设置分区），增删本身没有语义，定义自带的动作（保存配置）即为全部能力。
</script>

<style scoped>
.vdfs-form {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}
.vdfs-banner {
  flex-shrink: 0;
  margin: 0.5rem 1rem 0;
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--danger-border, var(--border-default));
  border-radius: var(--radius-md);
  background: var(--danger-subtle-bg, var(--surface-sunken));
}
.banner-msg {
  margin: 0;
  font-size: 0.8rem;
  color: var(--danger-fg);
}
.banner-fields {
  margin: 0.35rem 0 0;
  padding-left: 1.1rem;
  font-size: 0.75rem;
  color: var(--text-secondary);
}
.banner-fields code {
  font-family: var(--font-mono);
  margin-right: 0.4rem;
  color: var(--text-primary);
}
</style>

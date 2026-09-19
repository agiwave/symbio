<!--
  VdfsFormDetail — VDFS `form` 渲染器（定义驱动表单）

  职责：把节点的 `schema`（宿主方言的呈现描述，此处为 DetailDefinition）交给
  通用渲染器 DetailForm 动态生成表单，并把保存桥接到 VDFS 传输。

  ## 两个入参各归各位

  - **定义** = `node.schema`（provider 下发，VDFS 只透传）；
  - **取值** = `data` —— 页面层 `vdfs/read` 拿到的 `VdfsContent.text` 经 JSON
    parse 后的**表单模型对象**，显式传给渲染器的 `values` 入参。
    节点自身不携带正文，取值只有这一条来源。

  ## 通道适配

  VDFS 的通道是 `vdfs/read` / `vdfs/write`。因此本组件把定义适配为
  `binding: 'option'` —— DetailForm 对它的语义恰好是「数据来自外部、保存
  只交回纯字段值」：预填来自 `values`，保存 emit 纯字段值，由页面写回 `vdfs/write`。

  校验仍由 provider 自持（`vdfs/write` 失败带回字段级错误）。

  ## 动作

  本渲染器**不计算机制动作**：重命名 / 删除由页面单点算好后经 `mechanism-actions`
  传入（DetailShell 负责与定义声明的动作合并、同 id 去重）。定义里声明的
  `delete` 因此不会和机制兜底的 `delete` 变成两个按钮——那条去重规则只有一份实现。
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
      :node="node"
      :values="values"
      :capabilities="capabilities"
      :mechanism-actions="mechanismActions"
      :mechanism-busy="mechanismBusy"
      :saving="saving"
      :testing="testing"
      @save="(v) => $emit('save', v)"
      @delete="$emit('delete')"
      @open-container="$emit('browse')"
      @test="$emit('action', 'test')"
      @action="(id) => $emit('action', id)"
    />
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import DetailForm from './DetailForm.vue'
import type { VdfsRendererProps } from './rendererContract'
import {
  VDFS_ACTION_EXPORT,
  VDFS_ACTION_TEST,
  isVdfsDraft,
  vdfsAccessOf,
  type DetailDefinition,
} from '@/schemas/vdfs'

const props = withDefaults(defineProps<VdfsRendererProps>(), {
  // `data` 刻意不给默认值：它的类型是 `unknown`（文本 / 字段对象都可能），
  // 而 `withDefaults` 只接受工厂函数形式的默认值——给 `null` 会让 `data` 的
  // 解析类型退化成 `null`。缺省即 `undefined`，`values` 计算属性已把
  // `undefined` 归一为 `null`（与其余三个渲染器的口径一致）。
  error: '',
  fieldErrors: () => [],
  saving: false,
  testing: false,
  mechanismActions: () => [],
  mechanismBusy: null,
})

defineEmits<{
  (e: 'save', values: Record<string, unknown>): void
  (e: 'delete'): void
  /**
   * 执行**节点动作**（详情定义声明的动作，如「测试连接」）。
   * 本组件只把动作标识上抛——执行与结果呈现归页面层（`vdfs/action`）。
   */
  (e: 'action', id: string): void
  /**
   * 进入节点内部（容器寻址：`<id>/<子类别>`）。由**详情定义**声明
   * （`open-container` 动作）触发——是否有内部结构是 provider 的知识，
   * 前端只负责把动作转发给页面层 `browseInto`。
   */
  (e: 'browse'): void
  /** 节点重命名由页面级内联栏承载；此处仅声明以对齐渲染器统一契约 */
  (e: 'rename'): void
}>()

const access = computed(() => vdfsAccessOf(props.node))

/**
 * 草稿节点（机制「新建」态）——判据与页面同源（`schemas/vdfs.isVdfsDraft`）。
 *
 * 草稿态**不成立**的定义动作：它们都以「这个资源已经存在」为前提。保存本身
 * （`save`）当然保留——草稿的保存就是「新建」。其余动作点下去只会打到一个
 * 不存在的地址上：删除没有目标、测试连接没有配置、浏览内部没有内部、导出没有内容。
 */
const DRAFT_UNAVAILABLE_ACTIONS = new Set<string>([
  'delete',
  VDFS_ACTION_TEST,
  VDFS_ACTION_EXPORT,
  'open-container',
])

/**
 * 通道适配：数据一律由 VDFS 承载（`vdfs/read` 取值、`vdfs/write` 保存），
 * 故把定义归一为 `binding: 'option'`。不可写节点同时剥掉写相关的定义动作
 * （避免渲染出无效的「保存」按钮）。
 */
const definition = computed<DetailDefinition>(() => {
  const raw = (props.node.schema ?? {}) as DetailDefinition
  const draft = isVdfsDraft(props.node)
  return {
    ...raw,
    binding: 'option',
    // 只读节点原先整表剥掉动作（避免渲染出无效的「保存」/「删除」），但
    // **与写无关**的动作必须保留：「浏览内部」是纯导航（agent 正是只读却
    // 最需要它的那类资源），「测试连接」是只读自检，「导出」是只读打包，
    // 三者都不依赖写权限。
    actions: (access.value.write
      ? raw.actions
      : (raw.actions ?? []).filter(
          (a) =>
            a.id === 'open-container' || a.id === VDFS_ACTION_TEST || a.id === VDFS_ACTION_EXPORT
        )
    )?.filter((a) => !draft || !DRAFT_UNAVAILABLE_ACTIONS.has(a.id)),
  }
})

/**
 * 详情定义**声明了**「测试连接」⇒ 能力位为真。
 *
 * 定义是后端下发（`detail_definition`），故「该资源能否自检」的知识仍在后端；
 * 前端只把它翻译成 DetailForm 的条件键（`cap.test_connection`），不自行猜测。
 */
const testable = computed(() =>
  (definition.value.actions ?? []).some((a) => a.id === VDFS_ACTION_TEST)
)

/** 表单模型对象：页面层 `vdfs/read` 的文本按 JSON 解析后的字段值 */
const values = computed<Record<string, unknown> | null>(() =>
  props.data && typeof props.data === 'object' ? (props.data as Record<string, unknown>) : null
)

/** 访问位 → 能力（VDFS 里能力就是访问位，不存在类型特判） */
const capabilities = computed<Record<string, boolean>>(() => ({
  mutable: access.value.write,
  test_connection: testable.value,
}))
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
  border: 1px solid var(--danger-border);
  border-radius: var(--radius-md);
  background: var(--danger-subtle-bg);
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

<!--
  DetailShell — 详情渲染器的公共外壳（标题行 + 动作区 + 错误行 + 内容）

  所有「有标题、有动作、可能有错误」的详情页共用这一份结构。此前它在
  VdfsTextDetail / VdfsReadonlyDetail / DetailForm 里各手抄了一遍
  （`.detail-head` / `.head-title` / `.head-actions` 三份 scoped 副本，
  第三份连类名都改叫 `.form-header`，注释里自认「同构」）。

  ## 动作区的合并规则

  动作来自两个来源，职责不同，合并规则只有一份实现
  （`schemas/vdfs-form.mergeDetailActions`）：

  - `actions` —— **渲染器自有**动作（save / reset / test / open-container…），
    随该资源的形态而变，由渲染器声明；
  - `mechanism-actions` —— **机制级**默认动作（重命名 / 删除），由页面单点算好
    （`useVdfs.mechanismActions`），对所有已落盘可写节点一致。

  规则两条：两段之间插一个 divider；**同 id 时渲染器声明的那一份胜出**
  （渲染器对「这是哪个动作」更具体，机制只负责保证它存在）。

  ## 忙态

  自有动作的忙态按索引对齐（`busy` / `disabled`）；机制动作同时最多一个在跑，
  故用单个 id（`mechanism-busy`）表达——比再开两个数组参数简单，也避免了
  「哪个数组对应哪一段」的错位。

  ## 形态

  默认形态：shell 自带内边距并随内容滚动（文本 / 只读 / 消息详情）。
  `panel` 形态：无内边距、不滚动、标题行自带下边框与背景，滚动交给内容区
  （定义驱动表单，它的表体自己滚）。
-->
<template>
  <div class="detail-shell" :class="{ 'is-panel': panel }">
    <header class="detail-head">
      <div class="head-title">
        <slot name="title">
          <h3 class="title">{{ title }}</h3>
        </slot>
        <slot name="meta" />
      </div>
      <div v-if="merged.actions.length" class="head-actions">
        <VdfsActions
          :actions="merged.actions"
          :busy="merged.busy"
          :disabled="merged.disabled"
          @run="$emit('run', $event)"
        />
      </div>
    </header>

    <p v-if="error" class="detail-error">{{ error }}</p>

    <slot />
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import VdfsActions from './VdfsActions.vue'
import { mergeDetailActions, type DetailAction } from '@/schemas/vdfs'

const props = withDefaults(
  defineProps<{
    /** 标题文本（需要更复杂的标题块时用 `#title` 插槽覆盖） */
    title?: string
    /** 渲染器自有动作 */
    actions?: DetailAction[]
    /** 自有动作的进行中标记（与 `actions` 等长，按索引对齐） */
    busy?: boolean[]
    /** 自有动作的禁用标记（与 `actions` 等长，按索引对齐） */
    disabled?: boolean[]
    /** 机制级默认动作（页面单点计算后注入） */
    mechanismActions?: DetailAction[]
    /** 正在执行的机制动作 id */
    mechanismBusy?: string | null
    /** 详情级错误（纯文本） */
    error?: string
    /** 面板形态（表单页：标题行自带内边距与下边框，滚动交给内容区） */
    panel?: boolean
  }>(),
  {
    title: '',
    actions: () => [],
    busy: () => [],
    disabled: () => [],
    mechanismActions: () => [],
    mechanismBusy: null,
    error: '',
    panel: false,
  }
)

defineEmits<{ (e: 'run', action: DetailAction): void }>()

/** 自有动作 + 机制动作的合并结果（三个数组等长、按索引对齐） */
const merged = computed(() =>
  mergeDetailActions(
    props.actions,
    props.mechanismActions,
    props.busy,
    props.disabled,
    props.mechanismBusy
  )
)
</script>

<style scoped>
.detail-shell {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0.75rem 1rem;
  gap: 0.5rem;
}

/* 面板形态：标题行是面板条，滚动交给内容区（内容区自带内边距） */
.detail-shell.is-panel {
  padding: 0;
  gap: 0;
  overflow: hidden;
}

.detail-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  flex-shrink: 0;
  flex-wrap: wrap;
}

.detail-shell.is-panel .detail-head {
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border-default);
  background: var(--surface-panel);
}

.head-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
  flex-wrap: wrap;
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

.head-actions {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-shrink: 0;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.detail-error {
  margin: 0;
  font-size: 0.8rem;
  color: var(--danger-fg);
}
</style>

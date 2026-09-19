<!--
  VdfsActions — 节点动作按钮统一渲染件（机制内置，唯一实现）

  机制约定（§2.3 统一动作区）：动作 = 图标优先（图标按钮 + tooltip 显示
  动作名 / 进行中文案），无图标映射的动作回落为文字按钮；同一行内图标与
  文字按钮共用同一套骨架样式，保持与自定义详情页动作区（如聊天头部
  .header-btn）同构的简洁形态。

  消费方（全部在详情页内部渲染，页面不得另加外框）：
  - DetailForm（定义驱动：定义动作 + 机制动作注入，同排）；
  - 自定义渲染器（经 mechanism-actions prop 接收，渲染在自己的动作区）；
  - 通用只读兜底面板（VDFS 下为 VdfsReadonlyDetail）。

  busy / disabled 为与 actions 等长的标记数组（按索引对齐，允许同 id
  多动作各自具备进行中/禁用状态）。
-->
<template>
  <template v-for="(a, i) in actions" :key="a.id + ':' + i">
    <span v-if="a.style === 'divider'" class="ea-divider" />
    <button
      v-else
      type="button"
      class="ea-btn"
      :class="[a.style, { 'is-text': !iconOf(a) }]"
      :title="tooltip(a, i)"
      :aria-label="tooltip(a, i)"
      :disabled="isDisabled(i)"
      @click="$emit('run', a)"
    >
      <!-- 图标按钮：tooltip = 动作名（进行中显示 busy_label） -->
      <svg
        v-if="iconOf(a)"
        viewBox="0 0 24 24"
        width="15"
        height="15"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        v-html="iconOf(a)"
      />
      <template v-else>{{ label(a, i) }}</template>
    </button>
  </template>
</template>

<script setup lang="ts">
import type { DetailAction } from '@/schemas/vdfs'
import { getActionIcon } from '@/registry/vdfsIcons'

const props = defineProps<{
  /** 动作清单（调用方完成条件求值 / 排序 / 注入合并） */
  actions: DetailAction[]
  /** 进行中标记（按索引对齐；进行中显示 busy_label 并禁用） */
  busy?: boolean[]
  /** 禁用标记（按索引对齐） */
  disabled?: boolean[]
}>()

defineEmits<{ (e: 'run', action: DetailAction): void }>()

function iconOf(a: DetailAction): string | undefined {
  return getActionIcon(a)
}

function isBusy(i: number): boolean {
  return Boolean(props.busy?.[i])
}

/** 文字回落按钮文案：进行中显示 busy_label */
function label(a: DetailAction, i: number): string {
  return isBusy(i) && a.busy_label ? a.busy_label : a.label
}

/** 图标按钮 tooltip：进行中显示 busy_label，否则动作名 */
function tooltip(a: DetailAction, i: number): string {
  return label(a, i)
}

function isDisabled(i: number): boolean {
  return isBusy(i) || Boolean(props.disabled?.[i])
}
</script>

<style scoped>
.ea-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.35rem;
  min-width: 1.75rem;
  height: 1.75rem;
  padding: 0 0.35rem;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  border-radius: var(--radius-md);
  cursor: pointer;
  font-size: 0.78rem;
  white-space: nowrap;
  transition: background var(--motion-fast) var(--motion-ease), color var(--motion-fast) var(--motion-ease);
}
.ea-btn:hover:not(:disabled) {
  background: var(--surface-hover);
  color: var(--text-primary);
}
.ea-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
/* 语义着色：主要动作 / 危险动作 */
.ea-btn.primary { color: var(--accent); }
.ea-btn.primary:hover:not(:disabled) { background: var(--accent-subtle-bg); color: var(--accent); }
.ea-btn.danger { color: var(--text-secondary); }
.ea-btn.danger:hover:not(:disabled) { background: var(--danger-bg); color: var(--danger-solid); }
/* 文字回落按钮（无图标映射） */
.ea-btn.is-text { padding: 0 0.55rem; }

.ea-divider {
  display: inline-block;
  width: 1px;
  height: 1.125rem;
  background: var(--border-default);
  margin: 0 0.15rem;
  flex-shrink: 0;
}
</style>

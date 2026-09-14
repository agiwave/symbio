<!--
  ChatOptionBar — 级联选项机制的唯一渲染件（机制内置）

  会话输入区下方的选项行：渲染后端下发的根选项节点（`OptionNode`），并按
  节点类型分派交互：

  - `invoke`：点击即执行（`pick` 原语先唤起原生对话框取值，再调用 action.endpoint）；
  - `sub`   ：展开级联菜单（菜单栈，支持任意深度；子项经 `children` 内联或懒加载）；
  - `form`  ：打开 OptionFormDialog（复用 DetailForm，绑定模式 option）。

  本组件与 `useSessionOptions` 组成机制的**全部前端实现**：不含任何具体业务
  选项（工作目录 / 智能体 / 模型 / 风险等级… 全部由后端声明）。
-->
<template>
  <div class="chat-option-bar">
    <button
      v-for="node in nodes"
      :key="node.id"
      type="button"
      class="option-btn"
      :class="[`st-${node.status}`, { 'is-disabled': !node.enabled, open: openRootId === node.id }]"
      :title="tooltip(node)"
      @click.stop="onNodeClick(node, $event)"
    >
      <span class="opt-icon">{{ optionIcon(node.icon) }}</span>
      <template v-if="showLabel(node)">
        <span class="opt-label">{{ node.label }}</span>
        <span v-if="hasValue(node)" class="opt-value">{{ node.value_label || node.value }}</span>
      </template>
      <span v-else class="opt-text">{{ node.value_label || node.value || node.label }}</span>
      <span v-if="node.option_type !== 'invoke'" class="opt-arrow" :class="{ open: openRootId === node.id }">▾</span>
      <span v-if="node.status === 'working'" class="opt-spin" />
      <span v-else-if="node.status === 'error'" class="opt-flag">!</span>
    </button>

    <!-- 级联菜单（菜单栈：sub → sub → … 任意深度） -->
    <Transition name="dropdown">
      <div
        v-if="stack.length"
        class="opt-menu"
        :style="{ left: `${menuLeft}px` }"
        @click.stop
      >
        <button v-if="stack.length > 1" type="button" class="menu-back" @click="popStack">
          <span class="back-arrow">←</span>
          <span>{{ parentLabel }}</span>
        </button>
        <div v-if="menuLoading" class="menu-hint">加载中…</div>
        <div v-else-if="!menuChildren.length" class="menu-hint">暂无可选项</div>
        <template v-else>
          <button
            v-for="child in menuChildren"
            :key="child.id"
            type="button"
            class="menu-item"
            :class="{ active: isSelected(child), 'is-disabled': !child.enabled }"
            :title="child.description || child.label"
            @click.stop="onChildClick(child)"
          >
            <span class="mi-icon">{{ optionIcon(child.icon) }}</span>
            <span class="mi-text">
              <span class="mi-label">{{ child.label }}</span>
              <span v-if="child.description" class="mi-desc">{{ child.description }}</span>
            </span>
            <span v-if="child.option_type !== 'invoke'" class="mi-arrow">›</span>
            <span v-else-if="isSelected(child)" class="mi-check">✓</span>
          </button>
        </template>
      </div>
    </Transition>

    <!-- 表单型选项（自动化表单；复用体系内唯一的详情渲染器） -->
    <OptionFormDialog
      v-if="formNode"
      :node="formNode"
      :saving="busy"
      @close="formNode = null"
      @save="onFormSave"
    />
  </div>
</template>

<script setup lang="ts">
/**
 * 级联选项栏（机制内置，唯一实现）
 *
 * 协议端点：选项宿主（session 插件）的 `worker/session/options/list`。
 * 根层与子层共用该端点（`parent` 参数区分）——与实体机制 `list_items`
 * 的树懒加载同构，机制只有一条通道。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import OptionFormDialog from './OptionFormDialog.vue'
import { useSessionOptions } from '@/composables/useSessionOptions'
import { optionIcon } from '@/registry/optionIcons'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import type { OptionNode } from '@/schemas/options'

const props = defineProps<{
  /** 当前会话 id；缺省 = 草稿态（新建会话前，选择缓冲后随创建写入） */
  sessionId?: string
}>()

const { nodes, loadChildren, dispatch, hasSession, draftMetadata } = useSessionOptions(() => props.sessionId)
const toast = useToast()

/**
 * 供新建会话（草稿态）流程取用：选项行在无会话时累积的 metadata 补丁。
 * 创建会话时作为参数透传给 `session/create`，与后端 `session/update` 同一浅合并语义。
 */
defineExpose({ getDraftMetadata: () => ({ ...draftMetadata.value }) })

// ── 级联菜单（菜单栈） ──────────────────────────────────
const openRootId = ref<string | null>(null)
const stack = ref<OptionNode[]>([])
const menuLeft = ref(0)
const menuLoading = ref(false)
const busy = ref(false)
const formNode = ref<OptionNode | null>(null)

const menuChildren = computed<OptionNode[]>(() => {
  const top = stack.value[stack.value.length - 1]
  return top?.children ?? []
})

const parentLabel = computed(() => {
  const parent = stack.value[stack.value.length - 2]
  return parent?.label ?? '返回'
})

/**
 * 是否在选项栏显示类别标签（label）。机制级开关：由后端在节点 `display.show_label`
 * 上声明（缺省 true = 现行行为）。false = 仅「图标 + 当前值」，类别名整体移入悬停提示。
 * 前端零写死——显示策略完全由后端控制。
 */
const showLabel = (node: OptionNode): boolean => node.display?.show_label !== false

/** 该节点是否已设置当前值（用于现行「标签 + 值胶囊」双段渲染） */
const hasValue = (node: OptionNode): boolean => Boolean(node.value_label || node.value)

function tooltip(node: OptionNode): string {
  const parts = [node.label]
  const val = node.value_label || node.value
  if (val && val !== node.label) parts.push(`当前：${val}`)
  if (node.description) parts.push(node.description)
  if (node.status_detail) parts.push(node.status_detail)
  return parts.join('\n')
}

/** 路径末段（原生取值原语的通用展示格式） */
function basename(p: string): string {
  return p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
}

/** 子项选中态：与所在父节点的当前值一致（机制约定，子项 value 对应父节点 value） */
function isSelected(child: OptionNode): boolean {
  const parent = stack.value[stack.value.length - 1]
  if (!parent || child.value === undefined) return false
  return String(child.value) === String(parent.value ?? '')
}

function closeMenu() {
  stack.value = []
  openRootId.value = null
}

function popStack() {
  if (stack.value.length <= 1) {
    closeMenu()
    return
  }
  stack.value = stack.value.slice(0, -1)
}

/** 子项懒加载（内联为空且为 sub 时按 parent 拉取） */
async function ensureChildren(node: OptionNode) {
  if (node.option_type !== 'sub') return
  if (node.children && node.children.length > 0) return
  menuLoading.value = true
  try {
    node.children = await loadChildren(node.id)
  } finally {
    menuLoading.value = false
  }
}

async function onNodeClick(node: OptionNode, ev: MouseEvent) {
  if (!node.enabled || busy.value) return

  if (node.option_type === 'sub') {
    if (openRootId.value === node.id) {
      closeMenu()
      return
    }
    openRootId.value = node.id
    menuLeft.value = (ev.currentTarget as HTMLElement | null)?.offsetLeft ?? 0
    stack.value = [node]
    await ensureChildren(node)
    return
  }

  if (node.option_type === 'form') {
    closeMenu()
    formNode.value = node
    return
  }

  // invoke：一次性调用（草稿态回显原生取值结果）
  const action = node.action
  if (!action) return
  busy.value = true
  try {
    const res = await dispatch(action)
    if (!hasSession() && action.pick && res.applied != null) {
      const p = String(res.applied)
      node.value = p
      node.value_label = basename(p)
    }
  } catch (e) {
    logger.error('ChatOptionBar', `选项「${node.label}」执行失败`, e)
    toast.showToast('error', `${node.label}失败：${e instanceof Error ? e.message : String(e)}`)
  } finally {
    busy.value = false
    closeMenu()
  }
}

async function onChildClick(child: OptionNode) {
  if (!child.enabled || busy.value) return

  if (child.option_type === 'sub') {
    stack.value = [...stack.value, child]
    await ensureChildren(child)
    return
  }
  if (child.option_type === 'form') {
    closeMenu()
    formNode.value = child
    return
  }

  const parent = stack.value[stack.value.length - 1]
  if (!child.action) return
  busy.value = true
  try {
    await dispatch(child.action)
    // 草稿态（无会话，未走后端回读）就地回显选中态
    if (!hasSession() && parent && child.value !== undefined) {
      parent.value = child.value
      parent.value_label = child.value_label ?? child.label
    }
  } catch (e) {
    logger.error('ChatOptionBar', `选项「${child.label}」执行失败`, e)
    toast.showToast('error', `${child.label}失败：${e instanceof Error ? e.message : String(e)}`)
  } finally {
    busy.value = false
    closeMenu()
  }
}

async function onFormSave(values: Record<string, unknown>) {
  const node = formNode.value
  if (!node?.action) {
    formNode.value = null
    return
  }
  busy.value = true
  try {
    await dispatch(node.action, { value: values })
    // 草稿态无权威回读，作通用「已配置」回显（会话态由后端回读权威值）
    if (!hasSession()) node.value_label = '已配置'
    formNode.value = null
  } catch (e) {
    logger.error('ChatOptionBar', `选项「${node.label}」保存失败`, e)
    toast.showToast('error', `${node.label}保存失败：${e instanceof Error ? e.message : String(e)}`)
  } finally {
    busy.value = false
  }
}

function onDocClick() {
  closeMenu()
}

onMounted(() => document.addEventListener('click', onDocClick))
onBeforeUnmount(() => document.removeEventListener('click', onDocClick))
</script>

<style scoped>
.chat-option-bar {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-1);
  flex-wrap: wrap;
  margin-top: 0.5rem;
}

/* ── 根选项按钮 ─────────────────────────────────────── */
.option-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.3rem 0.55rem;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  cursor: pointer;
  user-select: none;
  transition: background-color var(--motion-fast) var(--motion-ease),
    color var(--motion-fast) var(--motion-ease);
}

.option-btn:hover:not(.is-disabled) {
  background: var(--surface-hover);
  color: var(--text-secondary);
}

.option-btn.open {
  background: var(--surface-selected);
  color: var(--text-primary);
}

.option-btn.is-disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.option-btn.st-error {
  color: var(--danger-fg);
}

.opt-icon {
  font-size: 0.875rem;
  line-height: 1;
  flex-shrink: 0;
}

.opt-label {
  font-weight: var(--font-weight-medium);
  white-space: nowrap;
}

.opt-value {
  max-width: 14rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 0 0.3rem;
  border-radius: var(--radius-sm);
  background: var(--surface-sunken);
  color: var(--text-secondary);
  font-size: 0.68rem;
}

/* 仅显示「图标 + 当前值」模式（后端 display.show_label=false）下的主文本 */
.opt-text {
  font-weight: var(--font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 16rem;
}

.opt-arrow {
  font-size: 0.6rem;
  opacity: 0.6;
  transition: transform var(--motion-base) var(--motion-ease);
}

.opt-arrow.open {
  transform: rotate(180deg);
}

.opt-spin {
  width: 0.6rem;
  height: 0.6rem;
  border: 0.1rem solid var(--border-strong);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: opt-spin 0.7s linear infinite;
}

@keyframes opt-spin {
  to {
    transform: rotate(360deg);
  }
}

.opt-flag {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 0.85rem;
  height: 0.85rem;
  border-radius: 50%;
  background: var(--danger-bg);
  color: var(--danger-fg);
  font-size: 0.6rem;
  font-weight: var(--font-weight-semibold);
}

/* ── 级联菜单 ───────────────────────────────────────── */
.opt-menu {
  position: absolute;
  bottom: calc(100% + var(--space-2));
  min-width: 13.75rem;
  max-width: min(22rem, 88vw);
  max-height: 25rem;
  overflow-y: auto;
  padding: var(--space-2);
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  z-index: var(--z-overlay);
}

.menu-back {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: var(--space-2) var(--space-3);
  margin-bottom: var(--space-1);
  border: none;
  border-bottom: 1px solid var(--border-subtle);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  cursor: pointer;
  text-align: left;
}

.menu-back:hover {
  color: var(--accent);
}

.back-arrow {
  flex-shrink: 0;
}

.menu-hint {
  padding: var(--space-4);
  text-align: center;
  color: var(--text-muted);
  font-size: var(--font-size-sm);
}

.menu-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  padding: var(--space-3);
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: inherit;
  cursor: pointer;
  text-align: left;
  transition: background-color var(--motion-fast) var(--motion-ease);
}

.menu-item:hover:not(.is-disabled) {
  background: var(--surface-hover);
}

.menu-item.active {
  background: var(--surface-selected);
}

.menu-item.is-disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.mi-icon {
  font-size: 1.125rem;
  flex-shrink: 0;
}

.mi-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.mi-label {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
}

.mi-desc {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  line-height: var(--line-height-tight);
  overflow: hidden;
  text-overflow: ellipsis;
}

.mi-arrow,
.mi-check {
  flex-shrink: 0;
  color: var(--text-muted);
}

.mi-check {
  color: var(--accent);
}

.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity var(--motion-fast) var(--motion-ease),
    transform var(--motion-fast) var(--motion-ease);
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(0.4rem);
}
</style>

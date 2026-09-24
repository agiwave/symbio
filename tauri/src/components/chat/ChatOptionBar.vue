<!--
  ChatOptionBar — 会话选项栏的唯一渲染件（机制内置）

  会话输入区下方的一排紧凑按钮：渲染会话**配置表单**（`DetailDefinition`，
  `binding = "option"`）的字段，按 `DetailField.widget` 分派交互：

  - `select`：点击展开候选菜单（一层；候选 = `field.options`）；
  - `path`  （带 `pick`）：唤起原生选择对话框，取值即写入；
  - `form`  ：打开子表单（`OptionFormDialog` → `DetailForm`，同一套方言）；
  - `toggle`：点击翻转布尔值。

  未列出的 widget 在选项栏里是**惰性**的（只显示当前值）——选项栏是「一排紧凑
  按钮」，不是纵向表单；需要完整表单编辑的字段请用 `widget = "form"` 包一层，
  那正是本方言给出的逃生舱（见设计文档 §3）。

  本组件与 `useSessionOptionBar` 组成选项栏的**全部前端实现**：不含任何具体业务
  选项（工作目录 / 智能体 / Model / 风险等级… 全部由后端在定义里声明）。

  定义与值的来源（三条通路，见 `docs/archive/session-options-unification.md` §3.2）：
  - 定义：`<根>/session/<id>` 的 `node.schema`（已落盘）/ `new_type.schema`（草稿）；
  - 值：节点 `attributes.metadata`（键 = 字段 key）；
  - 落库：`vdfs/write(<根>/session/<id>, {"metadata": {<key>: <值>}})`。
-->
<template>
  <div class="chat-option-bar">
    <button
      v-for="f in visibleFields"
      :key="f.key"
      type="button"
      class="option-btn"
      :class="{ 'is-disabled': isDisabled(f), open: openKey === f.key }"
      :title="tooltip(f)"
      @click.stop="onFieldClick(f, $event)"
    >
      <span class="opt-icon">{{ fieldIcon(f.icon) }}</span>
      <span class="opt-text">{{ compactFieldText(f, rawValue(f)) }}</span>
      <span v-if="opensMenu(f)" class="opt-arrow" :class="{ open: openKey === f.key }">▾</span>
    </button>

    <!-- 候选菜单（一层；`select` 的扁平候选） -->
    <Transition name="dropdown">
      <div v-if="openField" class="opt-menu" :style="{ left: `${menuLeft}px` }" @click.stop>
        <div v-if="!menuOptions.length" class="menu-hint">暂无可选项</div>
        <template v-else>
          <button
            v-for="o in menuOptions"
            :key="o.value"
            type="button"
            class="menu-item"
            :class="{ active: isSelected(openField, o.value) }"
            :title="o.description || o.label"
            @click.stop="choose(openField, o.value)"
          >
            <span class="mi-icon">{{ fieldIcon() }}</span>
            <span class="mi-text">
              <span class="mi-label">{{ o.label }}</span>
              <span v-if="o.description" class="mi-desc">{{ o.description }}</span>
            </span>
            <span v-if="isSelected(openField, o.value)" class="mi-check">✓</span>
          </button>
        </template>
      </div>
    </Transition>

    <!-- 表单型字段（结构化子对象；复用体系内唯一的详情渲染器） -->
    <OptionFormDialog
      v-if="formField?.form"
      :definition="formField.form"
      :values="subValuesOf(formField)"
      :saving="busy"
      @close="formField = null"
      @save="onFormSave"
    />
  </div>
</template>

<script setup lang="ts">
/**
 * 会话选项栏（机制内置，唯一实现）
 *
 * 数据全部由调用方经 props 注入（定义 + 值 + 条件作用域），本组件**不出站**——
 * 出站只在 `useSessionOptionBar.save`（写会话 metadata）与 `services/nativePick`
 * （原生取值原语）两处。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import OptionFormDialog from './OptionFormDialog.vue'
import { useSessionOptionBar } from '@/composables/useSessionOptionBar'
import { fieldIcon } from '@/registry/vdfsIcons'
import { pickNative } from '@/services/nativePick'
import { useToast } from '@/composables/useToast'
import {
  compactFieldText,
  evalDetailCondition,
  type DetailDefinition,
  type DetailField,
  type DetailOption,
  type DetailPick,
} from '@/schemas/vdfs'

const props = withDefaults(
  defineProps<{
    /** 当前会话 id；缺省 = 草稿态（新建会话前，选择缓冲后随创建写入） */
    sessionId?: string
    /** 选项定义（`node.schema` / `new_type.schema`）；缺省 = 不渲染任何选项 */
    definition?: DetailDefinition | null
    /** 当前字段值（会话态 = 节点 `attributes.metadata`）；缺省 = 全部按定义缺省值显示 */
    values?: Record<string, unknown> | null
    /** 条件求值的额外键（节点 `attributes`，如 `message_count`） */
    scope?: Record<string, unknown> | null
  }>(),
  { sessionId: undefined, definition: null, values: null, scope: null },
)

const { fields, busy, draftMetadata, save } = useSessionOptionBar({
  sessionId: () => props.sessionId,
  definition: () => props.definition,
})
const { showToast } = useToast()

/**
 * 供新建会话（草稿态）流程取用：选项行在无会话时累积的 metadata 补丁。
 * 创建会话时作为参数透传给 `createSessionWithFirstMessage`，与后端浅合并语义一致。
 */
defineExpose({ getDraftMetadata: () => ({ ...draftMetadata.value }) })

// ── 取值 ──────────────────────────────────────────────
//
// 作用域规则（设计文档 §3.5）：条件求值模型 = `{ ...节点属性, ...字段值 }`。
// 前者提供「已有历史」这类节点级事实，后者提供字段自身的当前值。

/** 当前字段值 = 注入值 + 草稿缓冲（后者只在无会话时非空） */
const currentValues = computed<Record<string, unknown>>(() => ({
  ...(props.values ?? {}),
  ...draftMetadata.value,
}))

const rawValue = (f: DetailField): unknown => currentValues.value[f.key]

/** 生效值：未设置时按定义缺省（按钮文本与候选选中态共用，两者必须一致） */
const effectiveValue = (f: DetailField): unknown => rawValue(f) ?? f.default

function scopeOf(key: string): unknown {
  const merged = { ...(props.scope ?? {}), ...currentValues.value }
  return merged[key]
}

// ── 分派 ──────────────────────────────────────────────
const openKey = ref<string | null>(null)
const menuLeft = ref(0)
const formField = ref<DetailField | null>(null)

const visibleFields = computed(() => fields.value.filter((f) => isVisible(f)))
const openField = computed(() => fields.value.find((f) => f.key === openKey.value) ?? null)
const menuOptions = computed<DetailOption[]>(() => openField.value?.options ?? [])

/** 该字段的编辑入口是「展开菜单」还是「弹出子表单」（决定是否画下拉箭头） */
function opensMenu(f: DetailField): boolean {
  return f.widget === 'select' || f.widget === 'form'
}

function isVisible(f: DetailField): boolean {
  return f.visible_when ? evalDetailCondition(f.visible_when, scopeOf) : true
}

/** 禁用（`disabled_when` 成立才禁用；缺省 = 不禁用，故不可写成 `!eval…`） */
function isDisabled(f: DetailField): boolean {
  return f.disabled_when ? evalDetailCondition(f.disabled_when, scopeOf) : false
}

function isSelected(f: DetailField, value: string): boolean {
  return String(value) === String(effectiveValue(f) ?? '')
}

function tooltip(f: DetailField): string {
  const parts = [f.label]
  const val = compactFieldText(f, rawValue(f))
  if (val && val !== f.label) parts.push(`当前：${val}`)
  if (f.description) parts.push(f.description)
  return parts.join('\n')
}

function closeMenu() {
  openKey.value = null
}

/** 结构化子对象的当前值（缺省 = 字段 `default`，即子对象的缺省配置） */
function subValuesOf(f: DetailField): Record<string, unknown> {
  const v = rawValue(f) ?? f.default
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {}
}

/** 统一的提交出口：落库失败必须说出来，否则「点了没反应」 */
async function commit(f: DetailField, value: unknown) {
  try {
    await save(f.key, value)
    closeMenu()
  } catch (e) {
    // 诊断日志在写入口（`useSessionOptionBar`）已经记过，这里只负责让用户看见
    showToast('error', `${f.label}保存失败：${e instanceof Error ? e.message : String(e)}`)
  }
}

async function onFieldClick(f: DetailField, ev: MouseEvent) {
  if (isDisabled(f) || busy.value) return

  // ① 原生取值原语：后端唤不起原生对话框，由前端取值后直接写入
  if (f.pick) {
    const picked = await pickNative(f.pick as DetailPick)
    if (picked != null) await commit(f, picked)
    return
  }

  // ② 候选菜单：一层展开（再次点击收起）
  if (f.widget === 'select') {
    if (openKey.value === f.key) {
      closeMenu()
      return
    }
    formField.value = null
    openKey.value = f.key
    menuLeft.value = (ev.currentTarget as HTMLElement | null)?.offsetLeft ?? 0
    return
  }

  // ③ 结构化子对象：弹出子表单
  if (f.widget === 'form') {
    closeMenu()
    formField.value = f
    return
  }

  // ④ 开关：就地翻转
  if (f.widget === 'toggle') {
    await commit(f, !rawValue(f))
  }
}

async function choose(f: DetailField, value: string) {
  if (busy.value) return
  await commit(f, value)
}

async function onFormSave(values: Record<string, unknown>) {
  const f = formField.value
  if (!f) return
  formField.value = null
  await commit(f, values)
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

/* ── 字段按钮 ───────────────────────────────────────── */
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

.opt-icon {
  font-size: 0.875rem;
  line-height: 1;
  flex-shrink: 0;
}

/* 紧凑形态的主文本（「图标 + 当前值」） */
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

/* ── 候选菜单 ───────────────────────────────────────── */
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

.menu-item:hover {
  background: var(--surface-hover);
}

.menu-item.active {
  background: var(--surface-selected);
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

.mi-check {
  flex-shrink: 0;
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

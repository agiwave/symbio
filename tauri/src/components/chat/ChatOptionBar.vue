<!--
  ChatOptionBar — 会话选项栏的唯一渲染件（机制内置）

  会话输入区下方的一排紧凑按钮：渲染会话**配置表单**（`DetailDefinition`，
  `binding = "option"`）的字段，按 `DetailField.widget` 分派交互：

  - `select`：点击展开候选菜单（一层；候选 = `field.options`）；
  - `path`  （带 `pick`）：embedded 原生对话框 / browser Server 目录选择器，取值即写入；
  - `form`  ：打开子表单（`OptionFormDialog` → `DetailForm`，同一套方言）；
  - `toggle`：点击翻转布尔值。

  未列出的 widget 在选项栏里是**惰性**的（只显示当前值）——选项栏是「一排紧凑
  按钮」，不是纵向表单；需要完整表单编辑的字段请用 `widget = "form"` 包一层，
  那正是本方言给出的逃生舱（见设计文档 §3）。

  本组件与 `useSessionOptionBar` 组成选项栏的**全部前端实现**：不含任何具体业务
  选项（工作目录 / 智能体 / Model / 风险等级… 全部由后端在定义里声明）。

  定义与值的来源（三条通路，规范见 `crates/symbio-plugin-session/src/docs/107-session-options.md`）：
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
      :class="{
        'is-disabled': isDisabled(f) || busy,
        open: openKey === f.key,
        'is-busy': busy && busyKey === f.key,
      }"
      :title="busy && busyKey === f.key ? `保存中…` : tooltip(f)"
      :aria-label="fieldAriaLabel(f)"
      :aria-busy="busy && busyKey === f.key ? 'true' : undefined"
      :aria-expanded="opensMenu(f) ? openKey === f.key : undefined"
      :aria-haspopup="opensMenu(f) ? 'menu' : undefined"
      @click.stop="onFieldClick(f, $event)"
    >
      <!-- 保存进行中：当前字段原地转圈。
           此前只在逻辑里 `if (busy) return`：按钮看着完全正常却点不动，用户以为
           「点了没反应」。忙态必须看得见，且只标在那一格上——全部一起转圈就看不出
           是哪次写入在途。 -->
      <span v-if="busy && busyKey === f.key" class="opt-spinner" aria-hidden="true" />
      <span v-else class="opt-icon">{{ fieldIcon(f.icon) }}</span>
      <span class="opt-text">{{ compactFieldText(f, rawValue(f)) }}</span>
      <span v-if="opensMenu(f)" class="opt-arrow" :class="{ open: openKey === f.key }">▾</span>
    </button>

    <!-- 候选菜单（一层；`select` 的扁平候选） -->
    <Transition name="dropdown">
      <div
        v-if="openField"
        ref="menuRef"
        class="opt-menu"
        role="menu"
        :aria-label="openField.label"
        :style="{ left: `${menuLeft}px` }"
        @click.stop
        @keydown="onMenuKeydown"
      >
        <!-- 候选为空是**可达状态**（还没配模型 / 还没建技能），不是异常。
             只说「暂无可选项」用户会困惑于"那我该做什么"，故补一句去向。

             去向只能指到**配置总入口**，不能指到某个具体目录：字段定义里没有
             「本字段的候选来自哪个挂载点」这一信息（那是后端 provider 的知识，
             前端硬编码字段名 → 目录的映射就会在新增资源类型时悄悄失效）。
             指到根目录 = 把"该去哪一类里找"交还给用户，且永不失效。 -->
        <div v-if="!menuOptions.length" class="menu-hint">
          <p class="hint-title">暂无可选项</p>
          <p class="hint-desc">需要先在「资源」里配置，再来这里选择</p>
          <button class="hint-action" type="button" @click="goConfigure">
            前往资源管理
          </button>
        </div>
        <template v-else>
          <button
            v-for="o in menuOptions"
            :key="o.value"
            type="button"
            role="menuitem"
            class="menu-item"
            :class="{ active: isSelected(openField, o.value) }"
            :aria-current="isSelected(openField, o.value) ? 'true' : undefined"
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

    <DirectoryPicker
      v-if="pickerField"
      :visible="true"
      :title="`选择${pickerField.label}`"
      :initial-path="String(effectiveValue(pickerField) ?? '')"
      @close="pickerField = null"
      @select="onDirectorySelected"
    />
  </div>
</template>

<script setup lang="ts">
/**
 * 会话选项栏（机制内置，唯一实现）
 *
 * 数据全部由调用方经 props 注入（定义 + 值 + 条件作用域），本组件**不出站**——
 * 出站只在 `useSessionOptionBar.save`（写会话 metadata）与 `services/nativePick`
 * / `DirectoryPicker`（embedded 原生取值 / browser Server 目录取值）两处。
 */
import { computed, inject, nextTick, onBeforeUnmount, onMounted, ref, type Ref } from 'vue'
import { routerKey } from 'vue-router'
import OptionFormDialog from './OptionFormDialog.vue'
import DirectoryPicker from '@/components/common/DirectoryPicker.vue'
import { useSessionOptionBar } from '@/composables/useSessionOptionBar'
import { fieldIcon } from '@/registry/vdfsIcons'
import { pickPath } from '@/services/nativePick'
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
 * 路由**可选**取用。
 *
 * `useRouter()` 在无 router 时会 `console.warn`，而选项栏是通用机制件，会被单测
 * （无 router）与将来的嵌入式场景直接挂载。为一个「点了空候选才走到」的跳转，
 * 不值得给整个组件加挂载前置条件——故直接按 vue-router 的注入键取，取不到就
 * 静默降级为不跳转，其余功能照常。
 *
 * 键必须用 vue-router 导出的 `routerKey`（`InjectionKey<Router>`）。字符串
 * `'$router'` 是 Vue 2 时代的写法，在 vue-router 5 下取不到值；且因为 `inject`
 * 的泛型被显式断言，类型检查也发现不了——只有运行时才暴露。
 */
const router = inject(routerKey, null)

/**
 * 候选为空的去向：跳资源管理首页（VDFS 根）。
 *
 * 不深链到某个具体目录——字段定义里没有「候选来自哪个挂载点」这条信息，前端
 * 硬编码「模型字段 → /vdfs/model」会随后端新增资源类型而悄悄失效（而失效形态
 * 是"点了跳到空目录"，最难被发现）。指到根是永远成立的那一档。
 */
function goConfigure() {
  if (router) void router.push('/vdfs')
}

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
const pickerField = ref<DetailField | null>(null)
/** 打开菜单的触发按钮：关闭时焦点要回到它，而不是丢到 body 上 */
const menuTrigger: Ref<HTMLElement | null> = ref(null)
/** 候选菜单容器（键盘导航在它内部移动焦点） */
const menuRef: Ref<HTMLElement | null> = ref(null)
/** 正在保存的字段 key：忙态只标这一格（见模板注释） */
const busyKey = ref<string | null>(null)

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

/**
 * 可访问名称 = 字段名 + 当前值。
 *
 * 按钮的可见文字只有**值**（「GPT-4」），读屏听到的会是一个没有上下文的词；
 * 选项栏是紧凑控件，值必须可见，但名称不能只剩它。
 */
function fieldAriaLabel(f: DetailField): string {
  const val = compactFieldText(f, rawValue(f)) || '未设置'
  return `${f.label}：${val}`
}

function closeMenu(restoreFocus = false) {
  if (openKey.value && restoreFocus) menuTrigger.value?.focus()
  openKey.value = null
}

/** 结构化子对象的当前值（缺省 = 字段 `default`，即子对象的缺省配置） */
function subValuesOf(f: DetailField): Record<string, unknown> {
  const v = rawValue(f) ?? f.default
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {}
}

/** 统一的提交出口：落库失败必须说出来，否则「点了没反应」 */
async function commit(f: DetailField, value: unknown) {
  busyKey.value = f.key
  try {
    await save(f.key, value)
    closeMenu()
  } catch (e) {
    // 诊断日志在写入口（`useSessionOptionBar`）已经记过，这里只负责让用户看见
    showToast('error', `${f.label}保存失败：${e instanceof Error ? e.message : String(e)}`)
  } finally {
    busyKey.value = null
  }
}

async function onFieldClick(f: DetailField, ev: MouseEvent) {
  if (isDisabled(f) || busy.value) return

  // ① 统一路径取值：embedded 用原生 dialog，browser 打开 Server 目录选择器。
  //    取值本身也可能失败（权限 / 路径失效 / 接口异常），失败必须说出来。
  if (f.pick) {
    try {
      const picked = await pickPath(f.pick as DetailPick, {
        initialPath: String(effectiveValue(f) ?? ''),
        onBrowserPick: () => { pickerField.value = f },
      })
      if (picked != null) await commit(f, picked)
    } catch (e) {
      showToast('error', `${f.label}取值失败：${e instanceof Error ? e.message : String(e)}`)
    }
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
    menuTrigger.value = (ev.currentTarget as HTMLElement | null) ?? null
    menuLeft.value = menuTrigger.value?.offsetLeft ?? 0
    // 展开即把焦点送到候选上：`role=menu` 承诺的就是「用方向键选」。
    // 焦点留在触发按钮上时，方向键没有任何接收者——菜单看着能用，键盘用不了。
    void nextTick(() => {
      const items = menuItemsOf()
      const active = items.find((el) => el.classList.contains('active')) ?? items[0]
      active?.focus()
    })
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

async function onDirectorySelected(path: string) {
  const f = pickerField.value
  pickerField.value = null
  if (f) await commit(f, path)
}

function onDocClick() {
  closeMenu()
}

/** 菜单里可聚焦的候选项（按 DOM 顺序） */
function menuItemsOf(): HTMLElement[] {
  const root = menuRef.value
  if (!root) return []
  return Array.from(root.querySelectorAll<HTMLElement>('[role="menuitem"]'))
}

/**
 * 候选菜单的方向键导航（↑/↓/Home/End）。
 *
 * 只移动焦点，不代按回车：候选是原生 `<button>`，Enter / Space 的激活行为
 * 由浏览器给出，自己再处理一遍会双击。
 */
function onMenuKeydown(e: KeyboardEvent) {
  const items = menuItemsOf()
  if (!items.length) return
  const idx = items.indexOf(document.activeElement as HTMLElement)
  let next: number | null = null
  if (e.key === 'ArrowDown') next = idx < 0 ? 0 : (idx + 1) % items.length
  else if (e.key === 'ArrowUp') next = idx < 0 ? items.length - 1 : (idx - 1 + items.length) % items.length
  else if (e.key === 'Home') next = 0
  else if (e.key === 'End') next = items.length - 1
  if (next === null) return
  e.preventDefault()
  items[next].focus()
}

/** ESC 关闭候选菜单（页面交互规范承诺的关闭键）；只处理自己开着的那一层 */
function onDocKeydown(e: KeyboardEvent) {
  if (e.key !== 'Escape' || !openKey.value) return
  e.stopPropagation()
  closeMenu(true)
}

onMounted(() => {
  document.addEventListener('click', onDocClick)
  document.addEventListener('keydown', onDocKeydown)
})
onBeforeUnmount(() => {
  document.removeEventListener('click', onDocClick)
  document.removeEventListener('keydown', onDocKeydown)
})
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

/* 保存中：一个小转圈占住图标的位置（宽度不变，按钮不抽动） */
.opt-spinner {
  width: 0.75rem;
  height: 0.75rem;
  flex-shrink: 0;
  border: 2px solid var(--border-default);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: opt-spin 0.7s linear infinite;
}

@keyframes opt-spin {
  to {
    transform: rotate(360deg);
  }
}

.option-btn.is-busy {
  cursor: progress;
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
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-4);
  text-align: center;
  color: var(--text-muted);
  font-size: var(--font-size-sm);
}

.hint-title {
  margin: 0;
  color: var(--text-secondary);
}

.hint-desc {
  margin: 0;
  font-size: var(--font-size-xs);
}

.hint-action {
  padding: 0.3rem 0.85rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  cursor: pointer;
}
.hint-action:hover {
  background: var(--surface-hover);
  border-color: var(--accent);
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

/* 键盘导航时焦点必须看得见（展开即聚焦第一项，用户要知道当前在哪） */
.menu-item:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
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

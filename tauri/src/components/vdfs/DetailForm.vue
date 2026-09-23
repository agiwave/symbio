<!--
  DetailForm — 定义驱动的通用详情渲染器（机制内置，唯一实现）

  消费后端下发的 DetailDefinition（经 VDFS 节点的 `schema` 字段透传；预设联动 / 动态候选 /
  密码显隐 / 数字范围 / 折叠分区 / 条件徽标动作 / id·name 派生回落链），
  动态生成交互不复杂的详情页——新增此类详情 = 后端下发定义即可，
  前端零页面/零 ts 开发。

  两个入参分工明确（VDFS 口径）：
  - `node`    —— 被编辑的**资源节点**（VdfsNode）：提供 kind / name / title 与
                 场景扩展字段（flatten 到顶层），参与标题回落链与条件求值；
  - `values`  —— **表单模型对象**：字段当前值。它来自 `vdfs/read` 的
                 `VdfsContent.text`（JSON 文本由数据层 parse）或选项节点的 `data`。
                 节点自身不携带正文——正文只有 `vdfs/read` 这一条来源。

  绑定模式（definition.binding）：
  - upload  ：清单型资源。保存 emit `save`，载荷含派生/回显的 id 与全部字段值，
              由页面写回 `vdfs/write`（后端 validate_manifest 兜底）。
  - info    ：只读概览。无保存，static 字段取值优先来自 `values`，
              缺省时取节点顶层的扩展字段，动作仅限 open-container / delete 等
              机制通道动作（如 agent 目录概览）。
  - option  ：数据来自外部、保存只交回纯字段值——VDFS 的 `form` 节点
              （VdfsFormDetail 适配）与级联选项表单都走这一形态。

  结构化 widget 表单模型约定（与后端 validate_manifest 两侧一致）：
  list = 字符串数组（编辑态每行一项）；map = 键值对（编辑态每行 KEY=VALUE）。

  解析顺序（详情页分发）：按 ext 命中的专属渲染器 → 本渲染器（ext = form）
  → 通用兜底（registry/vdfsRenderers 登记）。
  会话聊天工作区 / appearance 即时生效 / about 信息展示等复杂详情
  不适用本渲染器，各自登记专属组件。
-->
<template>
  <DetailShell
    :actions="visibleActions"
    :busy="busyFlags"
    :disabled="disabledFlags"
    :mechanism-actions="mechanismActions"
    :mechanism-busy="mechanismBusy"
    panel
    @run="runAction"
  >
    <!-- 标题块：标题 + 徽标 + 副标题。比默认的「一行纯标题」复杂，故覆盖 #title 插槽 -->
    <template #title>
      <div class="title-block">
        <div class="title-line">
          <h2 class="title-text">{{ displayTitle }}</h2>
          <span v-for="(b, i) in visibleBadges" :key="i" class="badge" :class="b.style">
            {{ b.label }}
          </span>
        </div>
        <p v-if="subtitleParts.length" class="subtitle">
          <span v-for="(p, i) in subtitleParts" :key="i" class="subtitle-part">
            <span v-if="i > 0" class="dot">·</span>{{ p }}
          </span>
        </p>
      </div>
    </template>

    <!-- 表单主体：分区（可折叠）+ 字段行 -->
    <div class="form-body">
      <template v-for="(sec, si) in definition.sections" :key="si">
        <button
          v-if="sec.title"
          type="button"
          class="advanced-toggle"
          @click="toggleSection(si)"
        >
          <span class="chevron" :class="{ open: !collapsed[si] }">▸</span>
          {{ sec.title }}
        </button>
        <div v-show="!collapsed[si]" class="setting-group" :class="{ 'advanced-group': sec.title }">
          <template v-for="f in sec.fields" :key="f.key">
            <div v-if="fieldVisible(f)" class="setting-item" :class="{ column: widgetIsFullWidth(f) }">
            <div class="setting-info">
              <label>{{ f.label }}<span v-if="f.required" class="required">*</span></label>
              <p v-if="f.description" class="setting-desc">{{ f.description }}</p>
            </div>

            <!-- static（只读展示：info 绑定概览 / 只读字段；options 作值→标签映射） -->
            <div v-if="specOf(f).tag === 'static'" class="static-value">{{ staticDisplay(f) }}</div>

            <!-- form（结构化子对象：摘要 + 打开子表单） -->
            <div v-else-if="specOf(f).tag === 'form'" class="input-row">
              <span class="sub-summary">{{ subSummary(f) }}</span>
              <button
                type="button"
                class="icon-btn"
                :title="`配置「${f.label}」`"
                :disabled="fieldDisabled(f)"
                @click="openSubForm(f)"
              >
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M12 20h9" />
                  <path d="M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                </svg>
              </button>
            </div>

            <!-- toggle -->
            <label v-else-if="specOf(f).tag === 'toggle'" class="toggle">
              <input type="checkbox" v-model="form[f.key]" :disabled="fieldDisabled(f)" />
              <span class="toggle-slider" />
            </label>

            <!-- select（静态或预设动态选项） -->
            <select
              v-else-if="specOf(f).tag === 'select'"
              v-model="form[f.key]"
              :disabled="fieldDisabled(f)"
              @change="onPresetFieldChange(f)"
            >
              <option v-for="o in fieldOptions(f)" :key="o.value" :value="o.value">{{ o.label }}</option>
            </select>

            <!-- textarea / list（每行一项）/ map（每行 KEY=VALUE） -->
            <textarea
              v-else-if="specOf(f).tag === 'textarea'"
              v-model="form[f.key]"
              :rows="f.rows ?? 3"
              :placeholder="widgetPlaceholderOf(f)"
              :disabled="fieldDisabled(f)"
              spellcheck="false"
            />

            <!-- text / password / number / datalist / path（path 可带原生取值入口） -->
            <div v-else-if="specOf(f).revealable" class="input-row">
              <input
                v-model="form[f.key]"
                :type="reveal[f.key] ? 'text' : 'password'"
                :placeholder="f.placeholder"
                :disabled="fieldDisabled(f)"
              />
              <button
                type="button"
                class="icon-btn"
                :title="reveal[f.key] ? '隐藏' : '显示'"
                @click="reveal[f.key] = !reveal[f.key]"
              >
                <svg v-if="reveal[f.key]" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
                  <line x1="1" y1="1" x2="23" y2="23" />
                </svg>
                <svg v-else viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                  <circle cx="12" cy="12" r="3" />
                </svg>
              </button>
            </div>

            <input
              v-else-if="specOf(f).inputType === 'number'"
              v-model.number="form[f.key]"
              type="number"
              :min="f.min"
              :max="f.max"
              :step="f.step"
              :placeholder="widgetPlaceholderOf(f)"
              :disabled="fieldDisabled(f)"
            />

            <div v-else-if="specOf(f).datalist" class="input-wrap">
              <input
                v-model="form[f.key]"
                type="text"
                :list="`dl-${uid}-${f.key}`"
                :placeholder="f.placeholder"
                :disabled="fieldDisabled(f)"
              />
              <datalist :id="`dl-${uid}-${f.key}`">
                <option v-for="(s, i) in fieldSuggestions(f)" :key="i" :value="s" />
              </datalist>
            </div>

            <div v-else class="input-row">
              <input
                v-model="form[f.key]"
                :type="specOf(f).inputType ?? 'text'"
                :placeholder="widgetPlaceholderOf(f)"
                :disabled="fieldDisabled(f)"
              />
              <!-- 机制原生取值原语（字段声明 pick 时给入口；后端唤不起原生对话框） -->
              <button
                v-if="f.pick"
                type="button"
                class="icon-btn"
                :title="f.pick === DETAIL_PICK_DIRECTORY ? '选择目录' : '选择文件'"
                :disabled="fieldDisabled(f)"
                @click="onPick(f)"
              >
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              </button>
            </div>
            </div>
          </template>
        </div>
      </template>
    </div>

    <!-- 结构化子对象（widget = form）的编辑承载：递归复用本渲染器，保存回写父字段键 -->
    <Teleport to="body">
      <BaseModal v-if="subField" :visible="true" panel-class="sub-form-dialog" @close="closeSubForm">
        <DetailForm
          :definition="subDefinitionOf(subField)"
          :node="null"
          :values="subValues"
          :capabilities="EMPTY_CAPABILITIES"
          @save="onSubSave"
          @cancel="closeSubForm"
        />
      </BaseModal>
    </Teleport>
  </DetailShell>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import DetailShell from './DetailShell.vue'
import BaseModal from '@/components/common/BaseModal.vue'
import { pickNative } from '@/services/nativePick'
import {
  detailPresetFieldOptions,
  detailPresetFieldSuggestions,
  detailPresetOf,
  detailPresetPatch,
  evalDetailCondition,
  DETAIL_PICK_DIRECTORY,
  type DetailPick,
} from '@/schemas/vdfs-form'
import {
  widgetFromEdit,
  widgetInitialOf,
  widgetIsFullWidth,
  widgetIsReadonly,
  widgetPlaceholderOf,
  widgetSpecOf,
  widgetToEdit,
} from '@/registry/formWidgets'
import type {
  DetailAction,
  DetailBadge,
  DetailCondition,
  DetailDefinition,
  DetailField,
  VdfsNode,
} from '@/schemas/vdfs'

const props = withDefaults(
  defineProps<{
    /** 下发的详情页定义（本渲染器的唯一形态来源） */
    definition: DetailDefinition
    /** 被编辑的资源节点（null = 新建模式） */
    node: VdfsNode | null
    /**
     * 表单模型对象（字段名 → 当前值）：来自 `vdfs/read` 内容的 JSON parse，
     * 或级联选项节点的 `data`。异步到位（挂载后才返回）时会触发一次预填。
     */
    values?: Record<string, unknown> | null
    capabilities: Record<string, boolean>
    /** 机制动作注入（页面单一定义点计算：重命名 / 删除），与定义动作同排渲染于动作区。
     *  同 id 时**定义声明的那一份胜出**（去重规则见 schemas/vdfs-form.mergeDetailActions）。 */
    mechanismActions?: DetailAction[]
    saving?: boolean
    testing?: boolean
    /** 正在执行的机制动作 id（`delete` 的忙态来源于此——无论删除由谁声明，
     *  执行者都是页面的 `vdfs/delete`） */
    mechanismBusy?: string | null
    /** 新建模式下用于 ID 去重的现有 id 列表 */
    existingIds?: string[]
  }>(),
  {
    values: null,
    mechanismActions: () => [],
    saving: false,
    testing: false,
    mechanismBusy: null,
    existingIds: () => [],
  }
)

const emit = defineEmits<{
  /**
   * 统一保存入口：字段值对象（已按 widget 序列化）。
   * upload 绑定额外带上 `id`（节点名或按定义派生）与动作 payload 的附加键
   * （如 `skip_validation`）；其余绑定只交回纯字段值。
   */
  save: [values: Record<string, unknown>]
  test: []
  delete: []
  'set-default': []
  /** 机制导航动作：进入节点内部（payload.kind 指定子类别） */
  'open-container': [kind: string]
  /**
   * **未在本渲染器内实现**的动作：原样上抛动作标识（如 VDFS 节点动作
   * `export`）。动作语义归 provider，渲染器不做拦截——新增动作无需改动这里。
   */
  action: [id: string]
  cancel: []
}>()

const uid = Math.random().toString(36).slice(2, 8)

// ==================== 表单模型 ====================
// 字段值统一 any：v-model 双向绑定跨 7 种 widget 类型（checkbox/select/number/text）
const form = reactive<Record<string, any>>({})
const reveal = reactive<Record<string, boolean>>({})
const collapsed = reactive<Record<number, boolean>>({})

const isExisting = computed(() => Boolean(props.node?.name))
const isDefault = computed(() => Boolean(props.node && props.node.is_default === true))
const isUpload = computed(() => props.definition.binding === 'upload')
const isInfo = computed(() => props.definition.binding === 'info')

// ==================== 条件求值 ====================
/** 求值键：表单字段 / is_existing / is_default / cap.<name> */
function valueOf(key: string): unknown {
  if (key === 'is_existing') return isExisting.value
  if (key === 'is_default') return isDefault.value
  if (key.startsWith('cap.')) {
    const name = key.slice(4)
    return props.capabilities[name]
  }
  return form[key]
}

/** 条件求值：作用域 = 本表单的字段模型（+ 机制键 `is_existing` / `is_default` / `cap.*`） */
function evalCond(c: DetailCondition | null | undefined): boolean {
  return evalDetailCondition(c, valueOf)
}

/** 字段条件显隐（visible_when 不满足时整行不渲染） */
function fieldVisible(f: DetailField): boolean {
  return evalCond(f.visible_when)
}

/**
 * 字段是否**禁用**（`disabled_when` 成立才禁用；缺省 = 不禁用）。
 *
 * 与 `visible_when` 的分工：隐藏 = 这一项与当前场景无关；禁用 = 这一项存在、
 * 但此刻不能改（原因写在 `description` 里）。
 *
 * 判空是必须的，理由与动作侧 `actionDisabled` 逐字相同：`evalCond` 对空条件返回
 * `true`（那是为 `when` / `visible_when` 的「缺省即显示」服务的），
 * 写成 `!evalCond(...)` 会把「无条件」与「条件成立」两种相反情形都解释成不禁用。
 */
function fieldDisabled(f: DetailField): boolean {
  return f.disabled_when ? evalCond(f.disabled_when) : false
}

// ==================== 机制原生取值原语（DetailField.pick） ====================
//
// 后端唤不起原生对话框，故闭集（DETAIL_PICKS）里这几个取值由前端实现：
// 先取值、写入本字段，再由调用方按自己的绑定提交。
async function onPick(f: DetailField) {
  const picked = await pickNative(f.pick as DetailPick | undefined)
  if (picked != null) form[f.key] = picked
}

// ==================== 结构化子对象（widget = form） ====================
//
// 子对象仍是**本表单模型的一个键**：子表单保存时把整份字段值回写到 `form[key]`，
// 随外层一次提交——不产生第二个写入入口。
const EMPTY_CAPABILITIES: Record<string, boolean> = { mutable: false, test_connection: false }

/** 子表单固定的两个动作（子定义未声明时补上，声明了则以子定义为准） */
const SUB_FORM_ACTIONS: DetailAction[] = [
  { id: 'cancel', label: '取消', style: 'default' },
  { id: 'save', label: '确定', style: 'primary' },
]

const subField = ref<DetailField | null>(null)
/** 打开子表单时**定格**的子对象快照：props 恒等 → 不触发子表单的重置门闩 */
const subValues = ref<Record<string, unknown>>({})

function subLabelOf(f: DetailField, key: string): string {
  for (const sec of f.form?.sections ?? []) {
    const hit = sec.fields.find((x) => x.key === key)
    if (hit) return hit.label
  }
  return key
}

/** 子对象摘要（一行；避免把整棵子树摊进父表单） */
function subSummary(f: DetailField): string {
  const v = form[f.key]
  if (!v || typeof v !== 'object') return '未配置'
  const entries = Object.entries(v as Record<string, unknown>).filter(
    ([, val]) => val !== '' && val !== null && val !== undefined
  )
  if (!entries.length) return '未配置'
  return entries.map(([k, val]) => `${subLabelOf(f, k)}=${String(val)}`).join('，')
}

/** 子定义：补 `binding` 兜底、标题回落、以及缺省的两个动作 */
function subDefinitionOf(f: DetailField): DetailDefinition {
  const def: DetailDefinition = f.form ?? { binding: 'option', sections: [] }
  const declared = new Set((def.actions ?? []).map((a) => a.id))
  return {
    ...def,
    binding: def.binding || 'option',
    title_fallback: def.title_fallback || f.label,
    actions: [...(def.actions ?? []), ...SUB_FORM_ACTIONS.filter((a) => !declared.has(a.id))],
  }
}

function openSubForm(f: DetailField) {
  const cur = form[f.key]
  subValues.value = cur && typeof cur === 'object' ? { ...(cur as Record<string, unknown>) } : {}
  subField.value = f
}

function closeSubForm() {
  subField.value = null
}

function onSubSave(values: Record<string, unknown>) {
  if (subField.value) form[subField.value.key] = values
  subField.value = null
}

// ==================== widget 查表（呈现与编解码的唯一来源） ====================
//
// 「哪种 widget 怎么呈现 / 怎么编解码 / 占不占整行 / 参不参与保存」一律查
// `registry/formWidgets`。本渲染器不再出现任何按 widget 名的分支——新增一种
// widget 只需在那张表加一行；未登记的回落到 `text`（页面永不空白）。

/** 字段的 widget 规格（模板与逻辑共用同一份查表结果） */
function specOf(f: DetailField) {
  return widgetSpecOf(f.widget)
}

/** static 只读展示：options 值→标签映射，数组/对象友好展开 */
function staticDisplay(f: DetailField): string {
  const v = form[f.key]
  if (v == null || v === '') return '—'
  const opt = (f.options ?? []).find((o) => o.value === v)
  if (opt) return opt.label
  if (Array.isArray(v)) return v.length ? v.join('、') : '—'
  if (typeof v === 'object') {
    const entries = Object.entries(v as Record<string, unknown>)
    return entries.length ? entries.map(([k, val]) => `${k} ${val}`).join('、') : '—'
  }
  return String(v)
}

// ==================== 标题 / 徽标 / 动作 ====================
function firstNonEmpty(keys: string[] | undefined): string {
  for (const k of keys ?? []) {
    const v = valueOf(k)
    if (typeof v === 'string' && v.trim()) return v.trim()
    if (typeof v === 'number') return String(v)
  }
  return ''
}

const displayTitle = computed(
  () =>
    firstNonEmpty(props.definition.title_from) ||
    props.definition.title_fallback ||
    props.node?.title ||
    props.node?.name ||
    '详情'
)

const subtitleParts = computed(() =>
  (props.definition.subtitle_from ?? [])
    .map((k) => {
      const v = firstNonEmpty([k])
      if (!v) return ''
      // select 字段的原始值映射为选项标签（provider → OpenAI 等）
      const opt = selectFieldOptions.value[k]?.find((o) => o.value === v)
      return opt ? opt.label : v
    })
    .filter((s) => s.length > 0)
)

/** select 字段的静态/动态选项查找表（副标题标签映射用） */
const selectFieldOptions = computed<Record<string, Array<{ value: string; label: string }>>>(
  (): Record<string, Array<{ value: string; label: string }>> => {
    const map: Record<string, Array<{ value: string; label: string }>> = {}
    for (const sec of props.definition.sections) {
      for (const f of sec.fields) {
        if (specOf(f).tag === 'select' && !f.options_from_preset) map[f.key] = fieldOptions(f)
      }
    }
    return map
  }
)

const visibleBadges = computed<DetailBadge[]>(() =>
  (props.definition.badges ?? []).filter((b) => evalCond(b.when))
)

const visibleActions = computed<DetailAction[]>(() =>
  (props.definition.actions ?? []).filter((a) => evalCond(a.when))
)

/**
 * 定义动作的忙态。
 *
 * 删除单独一路：无论「删除」是详情定义声明的还是机制兜底注入的，执行者都是
 * 页面的 `vdfs/delete`（经 `@delete`），故它的忙态一律取机制动作忙态。
 * 其余动作（内置 `test`，以及 provider 自持的 VDFS 动作如 `export`）共用
 * 页面层的单一动作忙态——同一时刻只可能有一个动作在执行。
 */
function actionBusy(a: DetailAction): boolean {
  if (a.id === 'save') return props.saving
  if (a.id === 'delete') return props.mechanismBusy === 'delete'
  return props.testing
}

function actionDisabled(a: DetailAction): boolean {
  if (actionBusy(a)) return true
  // `disabled_when` = **条件成立才禁用**（缺省 ⇒ 不禁用）。
  // `evalCond` 对空条件返回 true（那是为 `when` 显隐服务的：无 when ⇒ 显示），
  // 所以这里必须显式判空，不能写成 `!evalCond(...)`——那样既会把「无条件」
  // 解释成「不禁用」，又把「条件成立」解释成「不禁用」，与字段语义正好相反。
  return a.disabled_when ? evalCond(a.disabled_when) : false
}

/** 定义动作的进行中 / 禁用标记（按索引对齐）。
 *  机制动作的忙态由 `DetailShell` 按 id 判定，不在此列。 */
const busyFlags = computed(() => visibleActions.value.map((a) => actionBusy(a)))
const disabledFlags = computed(() => visibleActions.value.map((a) => actionDisabled(a)))

function runAction(a: DetailAction) {
  switch (a.id) {
    case 'save': {
      // info 只读：没有 save 语义
      if (isInfo.value) return
      const values = buildSave()
      if (!isUpload.value) {
        // 数据来自外部的绑定：只交回纯字段值，不让动作载荷污染它
        emit('save', values)
        return
      }
      // upload 绑定：动作 payload（如 skip_validation）作为附加键并入
      emit('save', { ...values, ...((a.payload ?? {}) as Record<string, unknown>) })
      return
    }
    case 'cancel':
      emit('cancel')
      return
    case 'test':
      emit('test')
      return
    case 'delete':
      emit('delete')
      return
    case 'set-default':
      emit('set-default')
      return
    case 'open-container':
      emit('open-container', String((a.payload as Record<string, unknown> | undefined)?.kind ?? ''))
      return
    default:
      // 未内置的动作：原样上抛（VDFS 节点动作走这里，如 `export`）
      emit('action', a.id)
  }
}

// ==================== 保存（upload 绑定）：id/name 派生回落链 ====================
function isEmpty(v: unknown): boolean {
  return v == null || v === '' || (Array.isArray(v) && v.length === 0)
}

/** 可读 slug + `-2` 递增去重（机制内唯一实现，新建态 id 派生用） */
function generateId(base: string): string {
  const used = new Set(props.existingIds)
  const slug =
    base
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9-_]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'resource'
  let id = slug
  let counter = 2
  while (used.has(id)) {
    id = `${slug}-${counter}`
    counter++
  }
  return id
}

/**
 * 字段序列化（机制唯一实现）：list → string[]、map → 对象；static 只读。
 *
 * `ignoreVisibility = false`（upload 绑定）时，`visible_when` 不满足的字段
 * 不参与保存（如 mcp 的 stdio/http 互斥字段）；
 * `ignoreVisibility = true`（其余绑定）时保存全部字段——表单对应的是一份
 * **完整配置对象**（如心跳任务：关闭开关不得丢失间隔/提示词）。
 */
function buildValues(ignoreVisibility: boolean): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const sec of props.definition.sections) {
    for (const f of sec.fields) {
      if (widgetIsReadonly(f.widget)) continue
      if (!ignoreVisibility && !fieldVisible(f)) continue
      out[f.key] = widgetFromEdit(f.widget, form[f.key])
    }
  }
  return out
}

/**
 * 保存载荷（机制唯一入口）：
 * - 非 upload 绑定：纯字段值，由页面写回它自己的通道；
 * - upload 绑定：带上 id（既有节点用 `node.name`，新建态按定义派生），
 *   并补齐 name 回落链（首个非空字段补 name，不覆盖用户已填的 name）。
 */
function buildSave(): Record<string, unknown> {
  if (!isUpload.value) return buildValues(true)
  const id = props.node?.name || generateId(firstNonEmpty(props.definition.id_from) || 'resource')
  const values = buildValues(false)
  const manifest: Record<string, unknown> = { id, ...values }
  const nameFrom = firstNonEmpty(props.definition.name_from)
  const nameKey = props.definition.name_from?.[0]
  if (nameKey && nameFrom && isEmpty(form[nameKey])) {
    manifest[nameKey] = nameFrom
  }
  return manifest
}

// ==================== 生命周期 ====================
onMounted(() => {
  for (let i = 0; i < props.definition.sections.length; i++) {
    collapsed[i] = Boolean(props.definition.sections[i]?.collapsed)
  }
})

function toggleSection(i: number) {
  collapsed[i] = !collapsed[i]
}

// ==================== 预设联动 ====================
// 规则本身在 `schemas/vdfs-form`（纯函数，可直接单测）——此处只做状态编排：
// 「读表单值 → 算补丁 → 写回」。
const presetSpec = computed(() => props.definition.presets ?? null)
const currentPreset = computed(() => {
  const spec = presetSpec.value
  return spec ? detailPresetOf(spec, form[spec.field]) : null
})

/** 字段候选：静态 options，或 `options_from_preset` 时当前预设注入的候选 */
function fieldOptions(f: DetailField) {
  return detailPresetFieldOptions(f, currentPreset.value)
}

/** datalist 建议：静态 + 预设动态候选 */
function fieldSuggestions(f: DetailField): string[] {
  return detailPresetFieldSuggestions(f, currentPreset.value)
}

/** 应用预设联动算出的字段补丁（fill 策略见 schemas/vdfs-form.detailPresetPatch） */
function applyPreset(applySet: boolean) {
  const patch = detailPresetPatch(presetSpec.value, currentPreset.value, (k) => form[k], applySet)
  Object.assign(form, patch)
}

function onPresetFieldChange(f: DetailField) {
  if (presetSpec.value && f.key === presetSpec.value.field) {
    applyPreset(true)
  }
}

// ==================== 预填（watch values：编辑态 / 新建态重置） ====================
function initForm() {
  for (const sec of props.definition.sections) {
    for (const f of sec.fields) {
      form[f.key] = widgetInitialOf(f)
    }
  }
}

// 表单重置的身份门闩：记录上次绑定节点的 `${kind}:${name}` 与「表单模型对象
// 是否到位」。事件驱动的后台清单刷新会以新对象替换 node——身份未变时跳过重置，
// 保住编辑现场；仅身份变化（切换节点 / 新建↔编辑）或数据首次到位才走完整重置+预填。
// 「到位位」（`0`/`1`）必须纳入身份键：`values` 通常是**异步**到位的（VDFS 的
// `vdfs/read` 在挂载后才返回），挂载瞬间为 null。若身份键恒为常量，则
// 「挂载（null）→ 数据到达（对象）」的翻转会被门闩判成同一身份而 early-return，
// 导致预填永不执行（表现为：标题回落 fallback、字段全空）。
let lastItemKey: string | null | undefined
watch(
  () => props.values,
  () => {
    const n = props.node
    const ready = props.values ? 1 : 0
    const itemKey = n ? `${n.kind}:${n.name}:${ready}` : `new:${ready}`
    if (itemKey === lastItemKey) return // 同一节点的后台刷新 → 保留输入现场
    lastItemKey = itemKey
    initForm()
    // 预填来源：`values`（vdfs/read 的解析结果 / 选项节点 data）；
    // info 绑定的 static 字段在 values 缺省时取节点顶层的扩展字段
    const cfg =
      props.values ??
      (isInfo.value ? ((n as unknown as Record<string, unknown> | null) ?? null) : null)
    if (cfg && typeof cfg === 'object') {
      for (const sec of props.definition.sections) {
        for (const f of sec.fields) {
          if (cfg[f.key] !== undefined && cfg[f.key] !== null) form[f.key] = widgetToEdit(f.widget, cfg[f.key])
        }
      }
    }
    // 编辑态：仅刷新动态候选（不覆盖已填值）；新建态：按 fill 策略填充
    applyPreset(!isExisting.value)
    for (const k of Object.keys(reveal)) reveal[k] = false
  },
  { immediate: true }
)
</script>

<style scoped>
/* 标题块 / 徽标 / 副标题：DetailShell 的 header 是「一行」，本表单需要两行
   （标题行 + 副标题行），故走 #title 插槽自带排版。 */
.title-block {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

.title-line {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}

.title-text {
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.subtitle {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  margin: 0;
  display: flex;
  align-items: center;
  gap: 0.3rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.subtitle-part { display: inline-flex; align-items: center; gap: 0.3rem; }
.dot { opacity: 0.5; }

.badge {
  font-size: 0.65rem;
  padding: 0.1rem 0.45rem;
  border-radius: var(--radius-full);
  font-weight: var(--font-weight-medium);
  white-space: nowrap;
}
.badge.default { background: var(--success-bg); color: var(--success-fg); }
.badge.disabled { background: var(--surface-sunken); color: var(--text-muted); }
.badge.accent { background: var(--accent-subtle-bg); color: var(--accent); }

/* ============ 表单主体 ============ */
.form-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 1rem 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.setting-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.setting-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1.5rem;
  padding: 0.65rem 0;
  border-bottom: 1px solid var(--border-subtle);
}

.setting-item:last-child { border-bottom: none; }

.setting-item.column {
  flex-direction: column;
  align-items: stretch;
  gap: 0.5rem;
}

.setting-info { flex: 1; min-width: 0; }

.setting-info label {
  display: block;
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
  margin: 0;
}

.setting-desc {
  font-size: 0.72rem;
  color: var(--text-muted);
  margin: 0.15rem 0 0;
}

.required { color: var(--danger-fg); }

/* static 只读展示（info 绑定概览字段） */
.static-value {
  font-size: var(--font-size-base);
  color: var(--text-primary);
  font-family: var(--font-mono);
  word-break: break-all;
  text-align: right;
  max-width: 60%;
  user-select: text;
}
.setting-item.column .static-value {
  text-align: left;
  max-width: 100%;
}

/* 分区折叠开关 */
.advanced-toggle {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0.6rem 0 0.15rem;
  align-self: flex-start;
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-base);
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-md);
}
.advanced-toggle:hover {
  color: var(--accent);
  background: var(--surface-hover);
}
.chevron {
  display: inline-block;
  transition: transform var(--motion-fast) var(--motion-ease);
  font-size: 0.75rem;
}
.chevron.open { transform: rotate(90deg); }
.advanced-group { padding-left: 0.15rem; }

/* 表单控件 */
.setting-item input,
.setting-item select,
.setting-item textarea,
.input-row input,
.input-wrap input {
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: 0.4rem 0.55rem;
  font-size: var(--font-size-base);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-family: inherit;
  width: 17.5rem;
  max-width: 100%;
  box-sizing: border-box;
  transition: border-color var(--motion-fast) var(--motion-ease),
    box-shadow var(--motion-fast) var(--motion-ease);
}

.setting-item.column input,
.setting-item.column select,
.setting-item.column textarea {
  width: 100%;
}

.setting-item input:focus,
.setting-item select:focus,
.setting-item textarea:focus,
.input-row input:focus,
.input-wrap input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}

.setting-item input:disabled,
.setting-item select:disabled {
  background: var(--surface-sunken);
  color: var(--text-muted);
  cursor: not-allowed;
}

.setting-item textarea {
  resize: vertical;
  min-height: 3.75rem;
}

.input-wrap { display: flex; }
.input-row {
  display: flex;
  gap: 0.35rem;
  align-items: center;
}
.input-row input { flex: 1; min-width: 0; }

/* 结构化子对象（widget = form）：一行摘要 + 打开入口 */
.sub-summary {
  flex: 1;
  min-width: 0;
  font-size: var(--font-size-base);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: right;
}

/* 子表单弹窗外框：底色 / 圆角 / 阴影 / 层级由 BaseModal 提供，这里只写尺寸 */
.sub-form-dialog {
  width: 100%;
  max-width: 32rem;
  height: min(34rem, 84vh);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.625rem;
  height: 1.625rem;
  border: none;
  background: transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  color: var(--text-secondary);
  transition: all var(--motion-fast) var(--motion-ease);
  flex-shrink: 0;
}
.icon-btn:hover:not(:disabled) {
  background: var(--surface-hover);
  color: var(--text-primary);
}

/* Toggle */
.toggle {
  position: relative;
  display: inline-block;
  width: 2.25rem;
  height: 1.25rem;
  cursor: pointer;
  flex-shrink: 0;
}
.toggle input { opacity: 0; width: 0; height: 0; }
.toggle-slider {
  position: absolute;
  inset: 0;
  background: var(--border-strong);
  border-radius: var(--radius-full);
  transition: background var(--motion-base) var(--motion-ease);
}
.toggle-slider::before {
  content: '';
  position: absolute;
  width: 1rem;
  height: 1rem;
  left: var(--space-05);
  top: var(--space-05);
  background: var(--surface-panel);
  border-radius: 50%;
  transition: transform var(--motion-base) var(--motion-ease);
}
.toggle input:checked + .toggle-slider { background: var(--accent); }
.toggle input:checked + .toggle-slider::before { transform: translateX(1rem); }

@media (max-width: 45rem) {
  .setting-item { flex-direction: column; align-items: stretch; gap: 0.4rem; }
  .setting-item input,
  .setting-item select,
  .setting-item textarea,
  .input-row input,
  .input-wrap input { width: 100%; }
}
</style>

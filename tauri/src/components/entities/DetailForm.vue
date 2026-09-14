<!--
  DetailForm — 定义驱动的通用详情渲染器（机制内置，唯一实现）

  消费后端下发的 DetailDefinition（经 VDFS 节点的 `schema` 字段透传；预设联动 / 动态候选 /
  密码显隐 / 数字范围 / 折叠分区 / 条件徽标动作 / id·name 派生回落链），
  动态生成交互不复杂的详情页——新增此类详情 = 后端下发定义即可，
  前端零页面/零 ts 开发。

  绑定模式（definition.binding）：
  - upload  ：实体实体。预填 item.config；保存 emit save（VDFS 下由
              `vdfs/write` 承载，后端 validate_manifest 兜底）。
  - config  ：配置分区。mount 时经 load_path 拉取，保存经 save_path
              自持写回（后端 config/set 通道），内部管理 saving/toast。
  - info    ：只读概览。无保存，字段（static widget）取值来自
              item.config/extra，动作仅限 open-container/delete 等
              机制通道动作（如 agent bundle 概览）。
  - option  ：级联选项机制的表单选项（自动化表单）。预填自 `optionData`
              （由选项节点 `data` 下发），保存 emit option-save 纯字段值
              ——由选项机制按 `action.bind` 写回后调用后端服务。

  结构化 widget 表单模型约定（与后端 validate_manifest 两侧一致）：
  list = 字符串数组（编辑态每行一项）；map = 键值对（编辑态每行 KEY=VALUE）。

  解析顺序（详情页分发）：注册专属 editor → 本渲染器（有定义）→ 通用兜底。
  VDFS 下本渲染器由 `ext = form` 选中（VdfsFormDetail 薄适配）。
  会话聊天工作区 / appearance 即时生效 / about 信息展示等复杂详情
  不适用本渲染器，仍走注册 editor。
-->
<template>
  <div class="detail-form">
    <!-- 顶部：标题 + 徽标 + 操作（与注册 editor 的 form-header 同构） -->
    <header class="form-header">
      <div class="title-area">
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
      </div>
      <div v-if="allActions.length" class="header-actions">
        <EntityActions
          :actions="allActions"
          :busy="busyFlags"
          :disabled="disabledFlags"
          @run="runAction"
        />
      </div>
    </header>

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
            <div v-if="fieldVisible(f)" class="setting-item" :class="{ column: isFullWidth(f) }">
            <div class="setting-info">
              <label>{{ f.label }}<span v-if="f.required" class="required">*</span></label>
              <p v-if="f.description" class="setting-desc">{{ f.description }}</p>
            </div>

            <!-- static（只读展示：info 绑定概览 / 只读字段；options 作值→标签映射） -->
            <div v-if="f.widget === 'static'" class="static-value">{{ staticDisplay(f) }}</div>

            <!-- toggle -->
            <label v-else-if="f.widget === 'toggle'" class="toggle">
              <input type="checkbox" v-model="form[f.key]" />
              <span class="toggle-slider" />
            </label>

            <!-- select（静态或预设动态选项） -->
            <select
              v-else-if="f.widget === 'select'"
              v-model="form[f.key]"
              @change="onPresetFieldChange(f)"
            >
              <option v-for="o in fieldOptions(f)" :key="o.value" :value="o.value">{{ o.label }}</option>
            </select>

            <!-- textarea / list（每行一项）/ map（每行 KEY=VALUE） -->
            <textarea
              v-else-if="f.widget === 'textarea' || f.widget === 'list' || f.widget === 'map'"
              v-model="form[f.key]"
              :rows="f.rows ?? 3"
              :placeholder="structuredPlaceholder(f)"
              spellcheck="false"
            />

            <!-- text / password / number / datalist -->
            <div v-else-if="f.widget === 'password'" class="input-row">
              <input
                v-model="form[f.key]"
                :type="reveal[f.key] ? 'text' : 'password'"
                :placeholder="f.placeholder"
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
              v-else-if="f.widget === 'number'"
              v-model.number="form[f.key]"
              type="number"
              :min="f.min"
              :max="f.max"
              :step="f.step"
              :placeholder="f.placeholder"
            />

            <div v-else-if="f.widget === 'datalist'" class="input-wrap">
              <input
                v-model="form[f.key]"
                type="text"
                :list="`dl-${uid}-${f.key}`"
                :placeholder="f.placeholder"
              />
              <datalist :id="`dl-${uid}-${f.key}`">
                <option v-for="(s, i) in fieldSuggestions(f)" :key="i" :value="s" />
              </datalist>
            </div>

            <input
              v-else
              v-model="form[f.key]"
              type="text"
              :placeholder="f.placeholder"
            />
            </div>
          </template>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { callPlugin, reloadGatewayTransport } from '@/services/plugin'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import EntityActions from './EntityActions.vue'
import type {
  DetailAction,
  DetailBadge,
  DetailCondition,
  DetailDefinition,
  DetailField,
  EntitySummary,
} from '@/schemas/entities'

const props = withDefaults(
  defineProps<{
    /** 下发的详情页定义（本渲染器的唯一形态来源） */
    definition: DetailDefinition
    /** 选中的实体（null = 新建模式；upload 绑定据此预填 item.config） */
    item: EntitySummary | null
    /**
     * option 绑定：表单初始数据（字段名 → 值），由选项节点 `data` 下发。
     * 与 item.config 同构，但来源是「选项」而非「实体」。
     */
    optionData?: Record<string, unknown> | null
    capabilities: Record<string, boolean>
    /** 机制动作注入（页面单一定义点计算：容器入口/测试/删除，
     *  已排除定义声明过的动作），与定义动作同排渲染于 header-actions */
    mechanismActions?: DetailAction[]
    saving?: boolean
    testing?: boolean
    deleting?: boolean
    /** 新建模式下用于 ID 去重的现有 id 列表 */
    existingIds?: string[]
  }>(),
  { mechanismActions: () => [], saving: false, testing: false, deleting: false, existingIds: () => [] }
)

const emit = defineEmits<{
  /** 统一保存入口：id 为实体目录名，manifest 为完整配置，extra 合并动作 payload */
  save: [payload: { id: string; manifest: Record<string, unknown>; skipValidation: boolean }]
  /**
   * option 绑定保存：纯字段值（已按 widget 序列化，未含 id）。
   * 由选项机制写入 `action.bind` 指定路径后调用后端服务。
   */
  'option-save': [values: Record<string, unknown>]
  test: []
  delete: []
  'set-default': []
  /** 机制导航动作：路由推入容器实体页（payload.kind 指定容器类别） */
  'open-container': [kind: string]
  /**
   * **未在本渲染器内实现**的动作：原样上抛动作标识（如 VDFS 节点动作
   * `export`）。动作语义归 provider，渲染器不做拦截——新增动作无需改动这里。
   */
  action: [id: string]
  cancel: []
}>()

const toast = useToast()
const uid = Math.random().toString(36).slice(2, 8)

// ==================== 表单模型 ====================
// 字段值统一 any：v-model 双向绑定跨 7 种 widget 类型（checkbox/select/number/text）
const form = reactive<Record<string, any>>({})
const reveal = reactive<Record<string, boolean>>({})
const collapsed = reactive<Record<number, boolean>>({})
const configSaving = ref(false)
const configLoaded = ref(false)

const isExisting = computed(() => Boolean(props.item?.id))
const isDefault = computed(() => Boolean(props.item && props.item.is_default === true))
const isConfig = computed(() => props.definition.binding === 'config')
const isInfo = computed(() => props.definition.binding === 'info')
/** 级联选项机制的表单选项（自动化表单）：预填自 optionData、保存回选项机制 */
const isOption = computed(() => props.definition.binding === 'option')

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

function looseEq(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (a == null || b == null) return a == null && b == null
  try {
    return JSON.stringify(a) === JSON.stringify(b)
  } catch {
    return false
  }
}

function evalCond(c: DetailCondition | null | undefined): boolean {
  if (!c) return true
  if (c.all?.length) return c.all.every(evalCond)
  const v = valueOf(c.key)
  if (c.equals !== undefined && !looseEq(v, c.equals)) return false
  if (c.not_equals !== undefined && looseEq(v, c.not_equals)) return false
  if (c.truthy !== undefined && Boolean(v) !== c.truthy) return false
  return true
}

/** 字段条件显隐（visible_when 不满足时整行不渲染） */
function fieldVisible(f: DetailField): boolean {
  return evalCond(f.visible_when)
}

/** 整行布局：显式 full_width 或天然宽控件 */
function isFullWidth(f: DetailField): boolean {
  return Boolean(f.full_width) || ['textarea', 'list', 'map'].includes(f.widget)
}

// ==================== 结构化 widget（list/map/static） ====================
// 表单模型约定：list/map 编辑态为多行文本，保存时序列化回结构（与后端
// validate_manifest 两侧一致）；static 只读展示，不参与保存。

/** 编辑态占位：结构化 widget 给出格式提示 */
function structuredPlaceholder(f: DetailField): string {
  if (f.widget === 'list') return f.placeholder ?? '每行一项'
  if (f.widget === 'map') return f.placeholder ?? '每行一项：KEY=VALUE'
  return f.placeholder ?? ''
}

/** 编辑文本 → string[]（去空行/首尾空白） */
function parseList(text: unknown): string[] {
  if (!Array.isArray(text) && typeof text !== 'string') return []
  return String(text)
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
}

/** 编辑文本 → 键值对（首个 = 分隔；无 = 视为空值键） */
function parseMap(text: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  if (typeof text !== 'string') {
    if (text && typeof text === 'object') {
      for (const [k, v] of Object.entries(text as Record<string, unknown>)) out[k] = String(v)
    }
    return out
  }
  for (const line of text.split('\n')) {
    const t = line.trim()
    if (!t) continue
    const eq = t.indexOf('=')
    if (eq < 0) {
      out[t] = ''
    } else {
      out[t.slice(0, eq).trim()] = t.slice(eq + 1).trim()
    }
  }
  return out
}

/** 值 → 编辑文本（list 逐行、map 逐行 KEY=VALUE，其余原样） */
function toEditValue(widget: string, v: unknown): unknown {
  if (widget === 'list') return Array.isArray(v) ? v.map(String).join('\n') : ''
  if (widget === 'map') {
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      return Object.entries(v as Record<string, unknown>)
        .map(([k, val]) => `${k}=${val}`)
        .join('\n')
    }
    return ''
  }
  return v ?? ''
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
    props.item?.name ||
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
        if (f.widget === 'select' && !f.options_from_preset) map[f.key] = fieldOptions(f)
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

/** 完整动作行 = 定义动作（条件求值后）+ 机制动作注入（divider 分隔）。
 *  两段同源不同责：定义动作随定义下发，机制动作由页面单点计算注入。 */
const allActions = computed<DetailAction[]>(() => {
  const injected = props.mechanismActions ?? []
  if (!injected.length) return visibleActions.value
  if (!visibleActions.value.length) return injected
  return [
    ...visibleActions.value,
    { id: 'divider', label: '', style: 'divider' },
    ...injected,
  ]
})

function actionBusy(a: DetailAction): boolean {
  if (a.id === 'save') return configSaving.value || props.saving
  if (a.id === 'delete') return props.deleting
  // 其余动作（内置 `test`，以及 provider 自持的 VDFS 动作如 `export`）共用
  // 页面层的单一动作忙态——同一时刻只可能有一个动作在执行
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

/** EntityActions 按索引对齐的进行中/禁用标记 */
const busyFlags = computed(() => allActions.value.map((a) => actionBusy(a)))
const disabledFlags = computed(() => allActions.value.map((a) => actionDisabled(a)))

function runAction(a: DetailAction) {
  switch (a.id) {
    case 'save': {
      // option 绑定：保存纯字段值，交由级联选项机制按 action.bind 落库后调后端服务
      if (isOption.value) {
        emit('option-save', buildValues(true))
        return
      }
      if (isConfig.value || isInfo.value) {
        // config/info 绑定无 save 语义（配置分区自持保存；info 只读）
        if (isConfig.value) void saveConfig()
        return
      }
      // extra 动作 payload（如 skip_validation）合并进 manifest 并同步 emit 标志
      const extra = (a.payload ?? {}) as Record<string, unknown>
      const { id, manifest } = buildSave()
      emit('save', {
        id,
        manifest: { ...manifest, ...extra },
        skipValidation: extra.skip_validation === true,
      })
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
      .replace(/^-+|-+$/g, '') || 'entity'
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
 * `ignoreVisibility = true`（option 绑定）时保存全部字段——表单选项对应一份
 * **完整配置对象**（如心跳任务：关闭开关不得丢失间隔/提示词）。
 */
function buildValues(ignoreVisibility = false): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const sec of props.definition.sections) {
    for (const f of sec.fields) {
      if (f.widget === 'static') continue
      if (!ignoreVisibility && !fieldVisible(f)) continue
      if (f.widget === 'list') out[f.key] = parseList(form[f.key])
      else if (f.widget === 'map') out[f.key] = parseMap(form[f.key])
      else out[f.key] = form[f.key]
    }
  }
  return out
}

function buildSave(): { id: string; manifest: Record<string, unknown> } {
  let id = (props.item?.id as string) ?? ''
  if (!id) {
    id = generateId(firstNonEmpty(props.definition.id_from) || 'entity')
  }
  const manifest: Record<string, unknown> = { id, ...buildValues() }
  // 名称回落链：首个非空字段补 name（不覆盖用户已填的 name）
  const nameFrom = firstNonEmpty(props.definition.name_from)
  const nameKey = props.definition.name_from?.[0]
  if (nameKey && nameFrom && isEmpty(form[nameKey])) {
    manifest[nameKey] = nameFrom
  }
  return { id, manifest }
}

// ==================== 保存（config 绑定）：load/save_path 自持 ====================
onMounted(async () => {
  for (let i = 0; i < props.definition.sections.length; i++) {
    collapsed[i] = Boolean(props.definition.sections[i]?.collapsed)
  }
  if (!isConfig.value) return
  const loadPath = props.definition.load_path
  if (!loadPath) {
    configLoaded.value = true
    return
  }
  try {
    const cfg = await callPlugin<Record<string, unknown>>(loadPath, {})
    if (cfg && typeof cfg === 'object') Object.assign(form, cfg)
  } catch (err) {
    logger.error('DetailForm', `加载配置失败（${loadPath}）`, err)
    toast.showToast('error', '加载配置失败')
  } finally {
    configLoaded.value = true
  }
})

async function saveConfig() {
  const savePath = props.definition.save_path
  if (!savePath) return
  configSaving.value = true
  try {
    await callPlugin(savePath, { ...form })
    // 保存 gateway 配置后，重新读取出站配置使新配置立即生效
    if (savePath.startsWith('gateway/')) {
      await reloadGatewayTransport()
    }
    toast.showToast('success', '配置已保存')
  } catch (err) {
    toast.showToast('error', `保存失败: ${err}`)
  } finally {
    configSaving.value = false
  }
}

function toggleSection(i: number) {
  collapsed[i] = !collapsed[i]
}

// ==================== 预设联动 ====================
const presetSpec = computed(() => props.definition.presets ?? null)
const currentPreset = computed(() => {
  const spec = presetSpec.value
  if (!spec) return null
  return spec.presets.find((p) => p.value === form[spec.field]) ?? null
})

/** 字段候选：静态 options 优先，options_from_preset 时来自当前预设注入 */
function fieldOptions(f: DetailField) {
  if (f.options_from_preset) {
    const opts = currentPreset.value?.options?.[f.key] ?? []
    return opts.map((v) => ({ value: v, label: v }))
  }
  return f.options ?? []
}

/** datalist 建议：静态 suggestions + 预设动态候选 */
function fieldSuggestions(f: DetailField): string[] {
  const staticSug = f.suggestions ?? []
  if (!f.suggestions_from_preset) return staticSug
  const dyn = currentPreset.value?.options?.[f.key] ?? []
  return [...staticSug, ...dyn.filter((v) => !staticSug.includes(v))]
}

/**
 * 预设变更：注入动态候选（总是应用）；按 fill 策略填充 set 值
 * （if_empty = 仅空字段，always = 总是覆盖）。
 */
function applyPreset(applySet: boolean) {
  const spec = presetSpec.value
  if (!spec) return
  const preset = presetSpec.value!.presets.find((p) => p.value === form[spec.field])
  if (!preset) return
  if (applySet && preset.set) {
    const ifEmpty = spec.fill !== 'always'
    for (const [k, v] of Object.entries(preset.set)) {
      if (!ifEmpty || isEmpty(form[k])) form[k] = v
    }
  }
  // set_always：总是覆盖（如切换预设时协议校正为该预设支持的首个协议）
  if (preset.set_always) {
    for (const [k, v] of Object.entries(preset.set_always)) form[k] = v
  }
}

function onPresetFieldChange(f: DetailField) {
  if (presetSpec.value && f.key === presetSpec.value.field) {
    applyPreset(true)
  }
}

// ==================== 预填（watch item：编辑态 / 新建态重置） ====================
function initForm() {
  for (const sec of props.definition.sections) {
    for (const f of sec.fields) {
      form[f.key] = f.default ?? (f.widget === 'toggle' ? false : '')
    }
  }
}

// 表单重置的身份门闩：记录上次绑定实体的 `${kind}:${id}`。事件驱动的后台
// 清单刷新（refreshKind）会以新对象替换 item——身份未变时跳过重置，保住
// 编辑现场；仅身份变化（切换实体 / 新建↔编辑）才走完整重置+预填。
// option 绑定：身份恒为 'option'，预填来源为 props.optionData。
let lastItemKey: string | null | undefined
watch(
  () => (isOption.value ? props.optionData : props.item),
  () => {
    if (isConfig.value) return // config 绑定：onMounted 拉取，不随 item 重置
    const it = props.item
    const itemKey = isOption.value
      ? 'option'
      : it
        ? `${it.kind}:${it.id}`
        : null
    if (itemKey === lastItemKey) return // 同一实体的后台刷新 → 保留输入现场
    lastItemKey = itemKey
    initForm()
    // 预填来源：option → props.optionData（选项节点 data）；
    // 其余 → item.config（后端下发的完整配置）优先，
    // info 绑定的 static 字段取自 item 顶层（extra flatten 下发的概览字段）
    const cfg = isOption.value
      ? ((props.optionData ?? null) as Record<string, unknown> | null)
      : (((it?.config ?? null) as Record<string, unknown> | null) ??
        (isInfo.value ? ((it ?? null) as unknown as Record<string, unknown> | null) : null))
    if (cfg && typeof cfg === 'object') {
      for (const sec of props.definition.sections) {
        for (const f of sec.fields) {
          if (cfg[f.key] !== undefined && cfg[f.key] !== null) form[f.key] = toEditValue(f.widget, cfg[f.key])
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
.detail-form {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

/* ============ 顶部 header：与注册 editor 的 form-header 同构 ============ */
.form-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
  gap: 0.75rem;
  background: var(--surface-panel);
}

.title-area {
  display: flex;
  align-items: center;
  gap: 0.65rem;
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
}

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

.path-pill {
  display: inline-block;
  padding: 0 0.35rem;
  background: var(--surface-sunken);
  border-radius: var(--radius-xs);
  font-size: 0.65rem;
  color: var(--text-secondary);
  font-family: var(--font-mono);
  white-space: nowrap;
  cursor: pointer;
  transition: color var(--motion-fast) var(--motion-ease), background var(--motion-fast) var(--motion-ease);
}
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

.header-actions {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-shrink: 0;
  flex-wrap: wrap;
  justify-content: flex-end;
}

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

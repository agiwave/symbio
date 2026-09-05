<!--
  DetailForm — 定义驱动的通用详情渲染器（机制内置，唯一实现）

  消费后端 `resources/detail` 下发的 DetailDefinition（设计基准 = Model.vue
  表单复杂度：预设联动 / 动态候选 / 密码显隐 / 数字范围 / 折叠分区 /
  条件徽标动作 / id·name 派生回落链），动态生成交互不复杂的详情页——
  新增此类详情 = 后端下发定义即可，前端零页面/零 ts 开发。

  绑定模式（definition.binding）：
  - upload  ：实体资源。预填 item.config；保存 emit save（机制通道
              resources/upload manifest，后端 validate_manifest 兜底）。
  - config  ：配置分区。mount 时经 load_path 拉取，保存经 save_path
              自持写回（后端 config/set 通道），内部管理 saving/toast。

  解析顺序（WorkbenchView）：注册专属 editor → 本渲染器（有定义）→ 通用兜底。
  会话聊天工作区 / agent bundle 概览 / appearance 即时生效 / about 信息展示
  等复杂详情不适用本渲染器，仍走注册 editor。
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
      <div v-if="visibleActions.length" class="header-actions">
        <template v-for="(a, i) in visibleActions" :key="a.id + i">
          <span v-if="a.style === 'divider'" class="header-actions-divider" />
          <button
            v-else
            type="button"
            class="action-btn"
            :class="a.style"
            :disabled="actionDisabled(a)"
            @click="runAction(a)"
          >
            {{ actionLabel(a) }}
          </button>
        </template>
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
          <div
            v-for="f in sec.fields"
            :key="f.key"
            class="setting-item"
            :class="{ column: f.full_width || f.widget === 'textarea' }"
          >
            <div class="setting-info">
              <label>{{ f.label }}<span v-if="f.required" class="required">*</span></label>
              <p v-if="f.description" class="setting-desc">{{ f.description }}</p>
            </div>

            <!-- toggle -->
            <label v-if="f.widget === 'toggle'" class="toggle">
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

            <!-- textarea -->
            <textarea
              v-else-if="f.widget === 'textarea'"
              v-model="form[f.key]"
              :rows="f.rows ?? 3"
              :placeholder="f.placeholder"
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
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { callPlugin } from '@/services/plugin'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import type {
  DetailAction,
  DetailBadge,
  DetailCondition,
  DetailDefinition,
  DetailField,
  ResourceCapabilities,
  ResourceSummary,
} from '@/schemas/resources'

const props = withDefaults(
  defineProps<{
    /** 下发的详情页定义（本渲染器的唯一形态来源） */
    definition: DetailDefinition
    /** 选中的资源（null = 新建模式；upload 绑定据此预填 item.config） */
    item: ResourceSummary | null
    capabilities: ResourceCapabilities
    saving?: boolean
    testing?: boolean
    deleting?: boolean
    /** 新建模式下用于 ID 去重的现有 id 列表 */
    existingIds?: string[]
  }>(),
  { saving: false, testing: false, deleting: false, existingIds: () => [] }
)

const emit = defineEmits<{
  /** 统一保存入口：id 为资源目录名，manifest 为完整配置，extra 合并动作 payload */
  save: [payload: { id: string; manifest: Record<string, unknown>; skipValidation: boolean }]
  test: []
  delete: []
  'set-default': []
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

// ==================== 条件求值 ====================
/** 求值键：表单字段 / is_existing / is_default / cap.<name> */
function valueOf(key: string): unknown {
  if (key === 'is_existing') return isExisting.value
  if (key === 'is_default') return isDefault.value
  if (key.startsWith('cap.')) {
    const name = key.slice(4) as keyof ResourceCapabilities
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

function actionBusy(a: DetailAction): boolean {
  if (a.id === 'save') return configSaving.value || props.saving
  if (a.id === 'test') return props.testing
  if (a.id === 'delete') return props.deleting
  return false
}

function actionDisabled(a: DetailAction): boolean {
  return actionBusy(a) || !evalCond(a.disabled_when)
}

function actionLabel(a: DetailAction): string {
  if (actionBusy(a) && a.busy_label) return a.busy_label
  return a.label
}

function runAction(a: DetailAction) {
  switch (a.id) {
    case 'save': {
      if (isConfig.value) {
        void saveConfig()
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
    case 'test':
      emit('test')
      return
    case 'delete':
      emit('delete')
      return
    case 'set-default':
      emit('set-default')
      return
    default:
      logger.warn('DetailForm', '未知动作:', a.id)
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

function buildSave(): { id: string; manifest: Record<string, unknown> } {
  const manifest: Record<string, unknown> = { ...form }
  let id = (props.item?.id as string) ?? ''
  if (!id) {
    id = generateId(firstNonEmpty(props.definition.id_from) || 'resource')
  }
  manifest.id = id
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

watch(
  () => props.item,
  (it) => {
    if (isConfig.value) return // config 绑定：onMounted 拉取，不随 item 重置
    initForm()
    // 从列表项 config（后端 list_items 携带的完整配置）预填
    const cfg = (it?.config ?? null) as Record<string, unknown> | null
    if (cfg && typeof cfg === 'object') {
      for (const sec of props.definition.sections) {
        for (const f of sec.fields) {
          if (cfg[f.key] !== undefined && cfg[f.key] !== null) form[f.key] = cfg[f.key]
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

.header-actions-divider {
  display: inline-block;
  width: 1px;
  height: 1.125rem;
  background: var(--border-default);
  margin: 0 0.15rem;
  flex-shrink: 0;
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

/* 操作按钮 */
.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--accent);
  color: var(--text-on-accent);
  border: none;
  border-radius: var(--radius-md);
  padding: 0.35rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  white-space: nowrap;
  transition: background var(--motion-fast) var(--motion-ease), opacity var(--motion-fast) var(--motion-ease);
}
.action-btn:hover:not(:disabled) { background: var(--accent-hover); }
.action-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.action-btn.secondary {
  background: transparent;
  color: var(--text-primary);
  border: 1px solid var(--border-default);
}
.action-btn.secondary:hover:not(:disabled) { background: var(--surface-hover); }
.action-btn.danger {
  background: transparent;
  color: var(--danger-solid);
  border: 1px solid var(--border-default);
}
.action-btn.danger:hover:not(:disabled) { background: var(--danger-bg); }
.action-btn.icon {
  background: transparent;
  color: var(--text-secondary);
  border: none;
  padding: 0.35rem;
}
.action-btn.icon:hover:not(:disabled) { background: var(--surface-hover); color: var(--text-primary); }
.action-btn.icon.danger:hover:not(:disabled) { background: var(--danger-bg); color: var(--danger-solid); }

@media (max-width: 45rem) {
  .setting-item { flex-direction: column; align-items: stretch; gap: 0.4rem; }
  .setting-item input,
  .setting-item select,
  .setting-item textarea,
  .input-row input,
  .input-wrap input { width: 100%; }
}
</style>

/**
 * VDFS `ext = form` 的**宿主方言**（前端侧类型契约）
 *
 * 与后端对齐：详情定义由 provider 的 `detail_definition` 产出，随 `VdfsNode.schema`
 * **原样透传**到前端（VDFS 不解释其内容）。规范：docs/design/vdfs.md。
 *
 * **本文件是数据契约层：零组件知识**（不得导入任何 Vue 组件）。
 * 渲染它的是 `components/vdfs/DetailForm.vue`。
 *
 * 表单的**取值输入**与定义分开：定义说「有哪些字段」，字段当前值来自
 * `vdfs/read` 返回的 `VdfsContent.text`（JSON 文本，前端 parse 成对象后
 * 作为显式入参交给渲染器）。节点上**没有** config / extra 这类携带正文的字段。
 */

/**
 * 表单渲染器的能力位（`cap.<name>` 条件求值来源）。
 *
 * VDFS **不下发**能力表——它由渲染器按节点的访问位与详情定义声明的动作**自行计算**
 * （见 `VdfsFormDetail` 的 `capabilities`）。`DetailForm` 的 `capabilities`
 * 入参即为 `Record<string, boolean>`（当前仅 `mutable` / `test_connection`），不再另设类型。
 */

// ==================== 详情页定义（definition-driven detail） ====================

/**
 * 机制原生取值原语（`DetailField.pick`）——**跨栈闭集**，唯一定义处。
 *
 * 后端 `symbio_core/schemas/detail.rs` 用一组 `pub const DETAIL_PICK_*: &str`
 * 表达同一个闭集（不是枚举，故字面即线上取值）。`protocol-mirror-audit` 的 C 组
 * 按前缀提取后端取值、与本词表逐词比对。
 *
 * 语义：后端唤不起原生对话框，故字段可声明一个原语，由前端取值后写入该字段。
 */
export const DETAIL_PICK_DIRECTORY = 'directory'
export const DETAIL_PICK_FILE = 'file'
export const DETAIL_PICKS = [DETAIL_PICK_DIRECTORY, DETAIL_PICK_FILE] as const
export type DetailPick = (typeof DETAIL_PICKS)[number]

/** 条件谓词（徽标/动作显隐、字段条件显隐 visible_when）。`all` 存在时为 AND 组合 */
export interface DetailCondition {
  /**
   * 求值键：表单字段名，或特殊键 is_existing / is_default / cap.<name>。
   *
   * 可缺席：`all` 组合条件自己不带键（后端 `DetailCondition.key` 是
   * `#[serde(default)]` 的 `String`，组合条件下缺省为空串且从不被读取）。
   * 叶子条件则必须给出 `key`。
   */
  key?: string
  equals?: unknown
  not_equals?: unknown
  truthy?: boolean
  all?: DetailCondition[]
}

/** select 选项 / 值-标签对 */
export interface DetailOption {
  value: string
  label: string
  /** 候选项说明（紧凑渲染形态在候选菜单里显示的一行解释） */
  description?: string
}

/**
 * 表单字段定义。widget ∈ text|password|number|select|textarea|toggle|datalist|list|map|static|path|form
 * 结构化 widget 表单模型约定（渲染器与后端 validate_manifest 两侧一致）：
 * - list：字符串数组，编辑态每行一项
 * - map：字符串键值对，编辑态每行 KEY=VALUE
 * - static：只读展示（info 绑定），options 可作值→标签映射
 * - path：单行文本 + 原生选择入口（配 `pick`）
 * - form：结构化子对象，形状由 `form` 子定义描述
 */
export interface DetailField {
  key: string
  label: string
  description?: string
  required?: boolean
  widget: string
  /** 图标名（纯 UI 映射；缺省不显示图标） */
  icon?: string
  /** 条件显隐（不满足时整行不渲染） */
  visible_when?: DetailCondition
  /** 禁用条件：成立才禁用（缺省 = 不禁用）。与 DetailAction.disabled_when 同义 */
  disabled_when?: DetailCondition
  /** 机制原生取值原语（闭集 DETAIL_PICKS） */
  pick?: DetailPick
  placeholder?: string
  min?: number
  max?: number
  step?: number
  rows?: number
  /**
   * 候选项（`widget = "select"` 时渲染为候选菜单）。
   *
   * 对**没有候选菜单**的 widget（`path` / `form`），它退化为一张**值→标签表**：
   * 紧凑渲染形态（选项栏）按它把当前值压成一句话（规则见 `compactFieldText`）。
   * 例：`path` 字段给 `{value: "", label: "未选择目录"}` 以表达「未设置」，
   * `form` 字段给 `{value: "true", label: "已开启"}` 以表达子对象的开关态。
   */
  options?: DetailOption[]
  suggestions?: string[]
  options_from_preset?: boolean
  suggestions_from_preset?: boolean
  full_width?: boolean
  /**
   * 字段缺省值（未设置时用）。
   *
   * `widget = "form"` 时它是**子对象的缺省值**（一个对象），与纵向表单
   * `widgetInitialOf` 的语义一致——紧凑选项栏据此显示摘要，子表单据此预填。
   */
  default?: unknown
  /**
   * widget = 'form'：本字段值是结构化子对象，由这份子定义描述其字段。
   *
   * 紧凑渲染形态（会话选项栏）显示摘要的取值规则由 `compactFieldText` 实现：
   * 按子定义的 `title_from` 链取一个代表值 ⇒ 本字段 `options` 非空时按
   * 「值→标签」查表（`String(代表值)` 匹配）⇒ 仍无则回落 `label`。
   */
  form?: DetailDefinition
}

/** 分区（可折叠） */
export interface DetailSection {
  title?: string
  collapsed?: boolean
  fields: DetailField[]
}

/** 预设项：选中后按 set 填充字段，options 注入对应字段动态候选 */
export interface DetailPreset {
  value: string
  label: string
  /** 按 fill 策略填充（if_empty/always） */
  set?: Record<string, unknown>
  /** 总是覆盖（如协议校正） */
  set_always?: Record<string, unknown>
  options?: Record<string, string[]>
}

/** 预设联动规格：field 为触发字段；fill ∈ if_empty | always */
export interface DetailPresetSpec {
  field: string
  fill: string
  presets: DetailPreset[]
}

/** 标题区徽标（如「默认」「已停用」） */
export interface DetailBadge {
  when?: DetailCondition
  label: string
  style: string
}

/**
 * 动作按钮。id ∈ save|test|delete|set-default|open-container（payload.kind 指定子类别）、
 * 或 provider 自持的 VDFS 节点动作（`import` / `export` / `truncate`…，经 `vdfs/action` 转发）。
 * icon：图标名（可选）——语义动作 id 自带默认图标映射；仅当需要区分同 id 多形态
 * （如「跳过校验保存」）或自定义动作需要图标时显式指定；未知图标名回落为文字按钮。
 */
export interface DetailAction {
  id: string
  label: string
  style: string
  icon?: string
  when?: DetailCondition
  disabled_when?: DetailCondition
  payload?: Record<string, unknown>
  /**
   * 本动作的载荷是一个**本地整包文件**（`{filename, b64}`）；值是包的后缀（如 `zip`）。
   *
   * 后端唤不起原生对话框、也拿不到用户刚选的文件，故这一步只能由前端做：声明了
   * 它的动作在执行前先取一个本地文件，再把 `{filename, b64}` 作为载荷送出。
   * 与字段的 `pick` 是同一类声明（「这一步需要原生能力」），区别在**取值去向**：
   * 字段取到的是**路径**（写进字段值），动作取到的是**字节**（作为动作载荷）。
   *
   * 前端因此**不必认识「导入」这个动作**——它是形状判定（与 `actionFileOf`
   * 对结果的文件载荷判定对称），新增取文件的动作无需改动前端。
   */
  pack?: string
  busy_label?: string
}

/**
 * 详情页定义。binding ∈
 * - `upload`：清单型资源，保存交回「id + 完整字段值」（id 由节点名给出或按定义派生）；
 * - `info`  ：只读概览，static 字段取值来自节点顶层的扩展字段（flatten 的 attributes）；
 * - `option`：数据来自外部、保存只交回纯字段值 —— VDFS 的 `form` 节点与级联选项表单
 *             都落在这两种形态上（取值统一由渲染器的显式入参传入）。
 */
export interface DetailDefinition {
  binding: string
  title_from?: string[]
  title_fallback?: string
  subtitle_from?: string[]
  name_from?: string[]
  id_from?: string[]
  sections: DetailSection[]
  presets?: DetailPresetSpec
  badges?: DetailBadge[]
  actions?: DetailAction[]
}

// ==================== 条件求值（机制唯一实现） ====================
//
// 「条件是否成立」原先长在 `DetailForm.vue` 里（`evalCond` / `looseEq` 两个局部
// 函数）。选项栏（`ChatOptionBar`）同样要判 `disabled_when`——「工作目录已锁定」
// 就是它。规则若各写一份，同一条 `disabled_when` 在纵向表单与紧凑选项栏里会给出
// 不同答案，而这种分歧没有任何守卫会红。故下沉为纯函数：**规则在这里，作用域在
// 调用方**（纵向表单的求值键来自表单模型，选项栏的来自「节点属性 + 字段值」）。

/** 宽松相等（`equals` / `not_equals` 用）：先比引用，再比 JSON 形状 */
export function looseDetailEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (a == null || b == null) return a == null && b == null
  try {
    return JSON.stringify(a) === JSON.stringify(b)
  } catch {
    return false
  }
}

/**
 * 条件求值。`valueOf` 提供「求值键 → 值」（作用域由调用方决定）。
 *
 * 缺省 `true` 是**为 `when` / `visible_when` 的「缺省即显示」服务**的——
 * 因此判「禁用」时不可写成 `!evalDetailCondition(...)`：那会把「无条件」与
 * 「条件成立」两种相反情形都解释成不禁用（见 `DetailForm` 的 `fieldDisabled`）。
 */
export function evalDetailCondition(
  c: DetailCondition | null | undefined,
  valueOf: (key: string) => unknown,
): boolean {
  if (!c) return true
  if (c.all?.length) return c.all.every((sub) => evalDetailCondition(sub, valueOf))
  // 叶子条件：`key` 缺席（后端缺省空串）⇒ 取不到任何值，比较器按常规语义判（
  // 缺省 `true` 是刻意的——见上文「缺省即显示」的说明）。
  const v = valueOf(c.key ?? '')
  if (c.equals !== undefined && !looseDetailEqual(v, c.equals)) return false
  if (c.not_equals !== undefined && looseDetailEqual(v, c.not_equals)) return false
  if (c.truthy !== undefined && Boolean(v) !== c.truthy) return false
  return true
}

// ==================== 紧凑渲染形态（选项栏）的取值规则 ====================
//
// `DetailDefinition` 有两种渲染形态：纵向表单（`DetailForm`）与紧凑选项栏
// （`ChatOptionBar`）。后者每个字段只占一个按钮，故必须把「当前值」压成一句话。
// 这条规则与 `DetailField.form` 的文档是同一件事，实现收在这里（纯函数，可脱离
// 组件单测），组件只做「取文本 → 渲染」。

/** 路径末段（`widget = "path"` 的展示格式；与后端 `basename` 语义一致） */
export function basenameOf(p: string): string {
  return p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
}

/** 「值 → 标签」查表（`options` 里 `String(value)` 相等的第一项） */
export function detailOptionLabel(f: DetailField, v: unknown): string | undefined {
  return f.options?.find((o) => o.value === String(v))?.label
}

/**
 * 结构化子对象的**代表值**：按子定义 `title_from` 链在子对象里取第一个非空值。
 *
 * 链上取不到（如 `title_from` 指向布尔而子对象缺该键）时返回 `undefined`——
 * 调用方据此回落，而不是把「取不到」当成一个值。
 */
export function formRepresentative(f: DetailField, value: unknown): unknown {
  if (!value || typeof value !== 'object') return undefined
  for (const key of f.form?.title_from ?? []) {
    const v = (value as Record<string, unknown>)[key]
    if (v !== undefined && v !== null && v !== '') return v
  }
  return undefined
}

/**
 * 选项栏按钮文本（紧凑形态的**唯一**取值规则）。
 *
 * 1. **生效值** = 当前值 ?? 字段 `default`（未设置时按定义缺省显示；与候选选中态
 *    同源，否则会出现「按钮写着中风险、菜单里一个都没勾」）；
 * 2. `widget = "form"` 先按子定义 `title_from` 取代表值，其余 widget 直接用生效值；
 * 3. 代表值为空 ⇒ 查 `options` 里 `value = ""` 的项（如「未选择目录」）⇒ 回落
 *    字段 `label`；
 * 4. `widget = "path"` 取路径末段；其余按 `options` 查表，
 *    查不到时原样显示值（对象则回落 `label`，避免把整棵子树印在按钮上）。
 */
export function compactFieldText(f: DetailField, value: unknown): string {
  const src = value ?? f.default
  const rep = f.widget === 'form' ? formRepresentative(f, src) : src
  if (rep == null || rep === '') return detailOptionLabel(f, '') ?? f.label
  if (f.widget === 'path') return basenameOf(String(rep))
  if (typeof rep === 'object') return detailOptionLabel(f, rep) ?? f.label
  return detailOptionLabel(f, rep) ?? String(rep)
}

// ==================== 动作区装配（机制唯一实现） ====================

/** 动作分组的视觉分隔符（`VdfsActions` 已内置其渲染） */
export const DETAIL_ACTION_DIVIDER: DetailAction = { id: 'divider', label: '', style: 'divider' }

/**
 * 把「渲染器自有动作」与「机制动作」装配成一行动作区。
 *
 * 动作来自两个来源，职责不同：`own` 随资源形态而变（save / reset / test /
 * open-container…），由渲染器声明；`mechanism` 是页面对**任何已落盘的可写节点**
 * 都能做的默认动作（重命名 / 删除），由页面单点算好
 * （`useVdfs.mechanismActions`）——渲染器不该各算一遍。
 *
 * 规则只有两条，因此只有这一份实现：
 *
 * 1. 两段之间插一个 `DETAIL_ACTION_DIVIDER`（视觉分组）；
 * 2. **同 id 时渲染器声明的那一份胜出** —— 渲染器对「这是哪个动作」更具体
 *    （如后端详情定义给 `delete` 配了更贴切的文案），机制只负责保证它存在。
 *    于是「定义声明了 delete」与「机制兜底提供 delete」不会渲染成两个按钮。
 *
 * 返回的三个数组**等长且按索引对齐**（`VdfsActions` 的入参形状）：自有动作的
 * 忙态取自调用方给的等长数组；机制动作同时最多只有一个在跑，故按 id 判定。
 */
export function mergeDetailActions(
  own: DetailAction[],
  mechanism: DetailAction[],
  ownBusy: boolean[] = [],
  ownDisabled: boolean[] = [],
  mechanismBusyId: string | null = null,
): { actions: DetailAction[]; busy: boolean[]; disabled: boolean[] } {
  const actions: DetailAction[] = []
  const busy: boolean[] = []
  const disabled: boolean[] = []
  own.forEach((a, i) => {
    actions.push(a)
    busy.push(Boolean(ownBusy[i]))
    disabled.push(Boolean(ownDisabled[i]))
  })
  const declared = new Set(own.map((a) => a.id))
  const extra = mechanism.filter((a) => !declared.has(a.id))
  if (extra.length) {
    if (actions.length) {
      actions.push(DETAIL_ACTION_DIVIDER)
      busy.push(false)
      disabled.push(false)
    }
    for (const a of extra) {
      actions.push(a)
      busy.push(a.id === mechanismBusyId)
      disabled.push(false)
    }
  }
  return { actions, busy, disabled }
}

// ==================== 预设联动（机制唯一实现） ====================
//
// 「选中某个预设 ⇒ 联动填充若干字段 / 注入若干动态候选」这一整套规则原先写在
// `DetailForm.vue` 的 computed 与 `applyPreset` 里，只能经**组件挂载**间接覆盖。
// 而它恰是本层最绕的一段（fill 策略 / set_always / 动态候选 / 建议合并）——
// 最该被直接钉住的规则，覆盖却最薄。故下沉为纯函数：**规则在这里，状态在组件**。

/** 判定空值（`set` 的 `if_empty` 策略用；与上传绑定的 id/name 回落链同口径） */
function isDetailValueEmpty(v: unknown): boolean {
  return v == null || v === '' || (Array.isArray(v) && v.length === 0)
}

/** 由触发字段的当前值解出命中的预设（无规格 / 未命中 → `null`） */
export function detailPresetOf(
  spec: DetailPresetSpec | null | undefined,
  triggerValue: unknown,
): DetailPreset | null {
  if (!spec) return null
  return spec.presets.find((p) => p.value === triggerValue) ?? null
}

/**
 * 字段候选：`options_from_preset` 时取当前预设注入的候选（**值即标签**——
 * 预设给的是协议名 / 模型名一类标识，没有另一套展示文案），否则用静态 options。
 */
export function detailPresetFieldOptions(
  f: DetailField,
  preset: DetailPreset | null,
): DetailOption[] {
  if (!f.options_from_preset) return f.options ?? []
  return (preset?.options?.[f.key] ?? []).map((v) => ({ value: v, label: v }))
}

/** datalist 建议：静态 `suggestions` 在前 + 预设动态候选在后（去重） */
export function detailPresetFieldSuggestions(
  f: DetailField,
  preset: DetailPreset | null,
): string[] {
  const staticSug = f.suggestions ?? []
  if (!f.suggestions_from_preset) return staticSug
  const dyn = preset?.options?.[f.key] ?? []
  return [...staticSug, ...dyn.filter((v) => !staticSug.includes(v))]
}

/**
 * 预设变更要写入的**字段补丁**（不直接改表单模型：规则层不持状态）。
 *
 * 两条规则，缺一不可：
 *
 * 1. `set` 按 `spec.fill` 策略填充 —— `if_empty` 只补空字段（不覆盖用户已填的），
 *    `always` 一律覆盖；
 * 2. `set_always` **总是**覆盖，且**不受 `applySet` 影响**（如切换预设时把协议
 *    校正为该预设支持的首个协议——「编辑态不重填」不该把它一并跳过）。
 *
 * @param applySet `false` = 只应用 `set_always`。编辑态预填走这条路：不覆盖用户
 *   已填的一般字段，但仍要校正必须一致的字段。
 * @param valueOf  取字段当前值（表单模型在组件里，规则层只问不存）
 */
export function detailPresetPatch(
  spec: DetailPresetSpec | null | undefined,
  preset: DetailPreset | null,
  valueOf: (key: string) => unknown,
  applySet: boolean,
): Record<string, unknown> {
  const patch: Record<string, unknown> = {}
  if (spec && preset && applySet && preset.set) {
    const ifEmpty = spec.fill !== 'always'
    for (const [k, v] of Object.entries(preset.set)) {
      if (!ifEmpty || isDetailValueEmpty(valueOf(k))) patch[k] = v
    }
  }
  // set_always 后写：同一键同时出现在 set 与 set_always 时，以后者为准
  if (preset?.set_always) {
    for (const [k, v] of Object.entries(preset.set_always)) patch[k] = v
  }
  return patch
}

// ==================== 结构化取值编解码（机制唯一实现） ====================
//
// 表单模型约定（渲染器与后端 `validate_manifest` 两侧一致）：
//   list = 字符串数组（编辑态每行一项）；map = 键值对（编辑态每行 `KEY=VALUE`）。
//
// 这组函数是「编辑态文本 ↔ 存储态结构」的**唯一**转换实现。此前它长在
// DetailForm 内部、与 widget 分支绑在一起（新增一种结构化 widget 要同时改解析、
// 序列化与模板三处）；提到此处后，`registry/formWidgets` 的 widget 表只引用它，
// 且它可脱离组件直接单测。

/** 编辑文本 → string[]（去空行 / 首尾空白；已是数组时逐项转字符串） */
export function parseListValue(text: unknown): string[] {
  if (!Array.isArray(text) && typeof text !== 'string') return []
  return String(text)
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
}

/** 编辑文本 → 键值对（首个 `=` 分隔；无 `=` 视为空值键） */
export function parseMapValue(text: unknown): Record<string, string> {
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

/**
 * 存储态 → 编辑文本（list 逐行、map 逐行 `KEY=VALUE`）。
 *
 * 只对**结构化** widget 有意义；其余 widget 的编辑态就是存储态本身，
 * 由 `registry/formWidgets` 的 widget 表直接透传，不必经过这里。
 */
export function editTextOf(kind: 'list' | 'map', v: unknown): string {
  if (kind === 'list') return Array.isArray(v) ? v.map(String).join('\n') : ''
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    return Object.entries(v as Record<string, unknown>)
      .map(([k, val]) => `${k}=${val}`)
      .join('\n')
  }
  return ''
}

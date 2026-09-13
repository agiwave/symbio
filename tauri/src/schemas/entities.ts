/**
 * VDFS `ext = form` 的**宿主方言**（前端侧类型契约）
 *
 * 与后端对齐：symbio/src/symbio_core/schemas/entities.rs（详情页定义部分）。
 *
 * **前端定位（S5 / S8 / S11 后）**：统一实体页、`services/entities.ts` 与
 * `entities/*` 调用协议均已下线，前端不再有任何实体协议调用点。保留本文件只因为
 * `DetailDefinition` 是 VDFS 的宿主方言——节点 `schema` 字段**透传**它，
 * `VdfsFormDetail` 据此渲染表单（字段 / 分区 / 徽标 / 动作 / 预设联动）；
 * `EntitySummary` / `EntityCapabilities` 是这套方言的附属形状。
 *
 * 已于 S11 删除的协议时代类型（`ProviderInfo` / `ProvidersResponse` /
 * `EntitiesListResponse` / `EntityUploadResponse` / `EntityStatusResponse` /
 * `ContainerKindInfo` / `DetailDefinitionResponse` / `ENTITY_LABELS`）不再需要：
 * 资源类别来自 `vdfs/providers`、列表来自 `vdfs/list`、动作来自 `vdfs/action`。
 */

/**
 * 实体能力开关——决定统一页面启用哪些模块 */
export interface EntityCapabilities {
  /** 以上传 zip 为主（文件名即实体目录名） */
  zip_upload: boolean
  /** 是否有独立表单 */
  independent_form: boolean
  /** 列表项是否有实时状态 */
  realtime_status: boolean
  /**
   * 列表头是否提供「刷新」动作。清单可能被外部修改的类型为 true；
   * 清单由生命周期事件通道自持同步（session）或固定（setting）为 false。
   * 前端据此决定列表头刷新按钮是否渲染（§3.4），不得自行判断。
   */
  refreshable: boolean
  /** 是否可写（可上传新增 / 删除） */
  mutable: boolean
  /** 是否支持连接测试 */
  test_connection: boolean
  /** 是否默认只读 */
  read_only: boolean
}

/** 实体概要（VDFS 详情渲染器的输入形状之一） */
export interface EntitySummary {
  kind: string
  /** 提供方（插件）显示名；由后端统一回填 */
  provider?: string
  name: string
  id: string
  description?: string
  summary?: string
  updated_at?: number
  status: string
  status_detail?: string
  /** 树视图：父节点 id（容器内相对路径；根层缺省） */
  parent?: string
  /** 树视图：可展开提示（false = 叶子；缺省按可展开处理，展开为空则收敛） */
  expandable?: boolean
  // 类型特有扩展字段（flatten）
  [extra: string]: unknown
}

// ==================== 详情页定义（definition-driven detail） ====================

/** 条件谓词（徽标/动作显隐、字段条件显隐 visible_when）。`all` 存在时为 AND 组合 */
export interface DetailCondition {
  /** 求值键：表单字段名，或特殊键 is_existing / is_default / cap.<name> */
  key: string
  equals?: unknown
  not_equals?: unknown
  truthy?: boolean
  all?: DetailCondition[]
}

/** select 选项 */
export interface DetailOption {
  value: string
  label: string
}

/**
 * 表单字段定义。widget ∈ text|password|number|select|textarea|toggle|datalist|list|map|static
 * 结构化 widget 表单模型约定（渲染器与后端 validate_manifest 两侧一致）：
 * - list：字符串数组，编辑态每行一项
 * - map：字符串键值对，编辑态每行 KEY=VALUE
 * - static：只读展示（info 绑定），options 可作值→标签映射
 */
export interface DetailField {
  key: string
  label: string
  description?: string
  required?: boolean
  widget: string
  /** 条件显隐（不满足时整行不渲染） */
  visible_when?: DetailCondition
  placeholder?: string
  min?: number
  max?: number
  step?: number
  rows?: number
  options?: DetailOption[]
  suggestions?: string[]
  options_from_preset?: boolean
  suggestions_from_preset?: boolean
  full_width?: boolean
  default?: unknown
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
 * 动作按钮。id ∈ save|test|delete|set-default|open-container（payload.kind 指定容器类别）或自定义。
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
  busy_label?: string
}

/** 详情页定义。binding ∈ upload（实体实体，保存走 entities/upload）| config（配置分区，经 load/save_path 读写）| info（只读概览，字段取值来自 item.config/extra） */
export interface DetailDefinition {
  binding: string
  load_path?: string
  save_path?: string
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

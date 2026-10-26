/**
 * protocol-mirror-audit 回归测试
 *
 * 与 mechanism-audit.test.mjs 同一立场：**只会亮绿灯的守卫等于没有守卫**。
 * 这里对三组规则各注入真实违规：
 * - A 组（常量镜像）：改值 / 改名 / 前端持有后端没有的常量 ⇒ 必须变红；
 * - B 组（缺席检查）：把会话地址段常量写回前端 ⇒ 必须变红；
 *   但写进 `__tests__` ⇒ 必须**不**红（测试持有协议夹具是它的职责）。
 * - C 组（闭集词表）：枚举加变体 / 词表多词 / 丢 `rename_all` / 词表元素不是常量
 *   ⇒ 必须变红。
 * - H 组（meta 键白名单）：未登记读点 / owner 字面消失 / 动态下标 ⇒ 必须变红；
 *   注释与字符串里的字段名、非 ChatNode 文件 ⇒ 必须不红。
 * 另有一条跑真实仓库，防本守卫在真仓库上误报。
 *
 * ## 夹具为什么要铺满 C / D 两份清单的每一条
 *
 * `ENUM_SETS` 与 `STRUCT_SETS` 都是脚本里的固定清单（枚举 ↔ 词表、结构体 ↔ 接口的
 * 对应关系无法自动推断，只能登记），所以基线仓库必须把**清单里的每一条**都铺出来——
 * 少一张词表、少一对结构体，那条就会报「文件 / 枚举 / 结构体 / 接口不存在」，基线测试
 * 直接变红。这不是冗余：它同时证明了「夹具与两份清单没脱节」。
 *
 * 清单条目数会随机制下线而变化（例如会话选项机制退场后 C 组 8 → 6、D 组 23 → 18），
 * 所以这里不写死数字——**基线断言的数字才是唯一口径**，注释跟着它走没有意义。
 *
 * 跑法：node --test scripts/protocol-mirror-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./protocol-mirror-audit.mjs', import.meta.url))

const CHAT_RS = 'crates/symbio-kernel/src/chat/chat_node.rs'
const CHAT_TS = 'tauri/src/schemas/chat_node.ts'
const DETAIL_RS = 'crates/symbio-kernel/src/present/detail.rs'
const FORM_TS = 'tauri/src/schemas/vdfs-form.ts'
const RISK_LEVEL_RS = 'crates/symbio-kernel/src/capability/mod.rs'
const SESSION_META_TS = 'tauri/src/schemas/session_meta.ts'
const READING_RS = 'crates/symbio-kernel/src/present/reading.rs'
const READING_TS = 'tauri/src/schemas/reading.ts'

/** 后端结构体（最小形态）：`pub struct X { pub a: String, }` */
const rsStruct = (name, ...fields) =>
  [`pub struct ${name} {`, ...fields.map((f) => `    pub ${f}: String,`), '}'].join('\n')

/** 前端接口（最小形态） */
const tsIface = (name, ...fields) =>
  [`export interface ${name} {`, ...fields.map((f) => `  ${f}?: string`), '}'].join('\n')

/** 后端闭集枚举。`style` 缺省 `snake_case`（C 组也支持 `lowercase`） */
const rsEnum = (name, variants, style = 'snake_case') =>
  [
    `#[serde(rename_all = "${style}")]`,
    `pub enum ${name} {`,
    ...variants.map((v) => `    ${v},`),
    '}',
  ].join('\n')

/** 前端词表数组（元素可以是裸字面量，也可以是本文件的字符串常量名） */
const tsArray = (name, ...items) => `export const ${name} = [${items.join(', ')}] as const`

/** 后端：四张词表对应的闭集枚举（含 `#[default]` 与文档注释，顺带验证解析器跳过它们） */
const CHAT_RS_SRC = [
  '/// 消息角色',
  '#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]',
  '#[serde(rename_all = "snake_case")]',
  '#[derive(Default)]',
  'pub enum NodeRole {',
  '    #[default]',
  '    User,',
  '    Assistant,',
  '}',
  '',
  '#[derive(Debug)]',
  '#[serde(rename_all = "snake_case")]',
  'pub enum NodeKind {',
  '    Text,',
  '    ToolCall,',
  '    UserPrompt,',
  '}',
  '',
  '#[derive(Debug)]',
  '#[serde(rename_all = "snake_case")]',
  'pub enum NodeStatus {',
  '    Pending,',
  '    WaitingUserAction,',
  '    Failed,',
  '}',
  '',
  '#[derive(Debug)]',
  '#[serde(rename_all = "snake_case")]',
  'pub enum ResumeAction {',
  '    RetrySubtree,',
  '    Retry,',
  '    Approve,',
  '}',
  '',
  rsStruct('ChatNode', 'id', 'parent_id'),
  '',
  // H 组：meta.tool_name 的 owner 侧字面（真实仓库住在 `symbio_core/chat/chat_node.rs`）
  'pub const META_TOOL_NAME: &str = "tool_name";',
].join('\n')

/** 前端：四张词表（常量 + 由常量数组派生的类型） */
const CHAT_TS_SRC = [
  "export const CHAT_ROLE_USER = 'user'",
  "export const CHAT_ROLE_ASSISTANT = 'assistant'",
  'export const CHAT_ROLES = [CHAT_ROLE_USER, CHAT_ROLE_ASSISTANT] as const',
  '',
  "export const NODE_KIND_TEXT = 'text'",
  "export const NODE_KIND_TOOL_CALL = 'tool_call'",
  "export const NODE_KIND_USER_PROMPT = 'user_prompt'",
  'export const NODE_KINDS = [',
  '  NODE_KIND_TEXT,',
  '  NODE_KIND_TOOL_CALL,',
  '  NODE_KIND_USER_PROMPT,',
  '] as const',
  '',
  "export const NODE_STATUS_PENDING = 'pending'",
  "export const NODE_STATUS_WAITING_USER_ACTION = 'waiting_user_action'",
  "export const NODE_STATUS_FAILED = 'failed'",
  'export const NODE_STATUSES = [',
  '  NODE_STATUS_PENDING,',
  '  NODE_STATUS_WAITING_USER_ACTION,',
  '  NODE_STATUS_FAILED,',
  '] as const',
  '',
  "export const RESUME_ACTION_RETRY_SUBTREE = 'retry_subtree'",
  "export const RESUME_ACTION_RETRY = 'retry'",
  "export const RESUME_ACTION_APPROVE = 'approve'",
  'export const RESUME_ACTIONS = [',
  '  RESUME_ACTION_RETRY_SUBTREE,',
  '  RESUME_ACTION_RETRY,',
  '  RESUME_ACTION_APPROVE,',
  '] as const',
  '',
  tsIface('ChatNode', 'id', 'parent_id'),
].join('\n')

/** 后端 vdfs 插件私有常量源（A 组的自动发现范围） */
const PROTOCOL_RS_SRC = [
  'pub const VDFS_LIST: &str = "vdfs/list";',
].join('\n')

/**
 * 跨插件可见的路由常量源（宿主的 `routes.rs`）。
 *
 * `vdfs/root` / `vdfs/watch` / `vdfs/unwatch` 三个路由**跨插件可见**，故常量归宿主的
 * `routes.rs` 且按「一域一前缀」改名 `ROUTE_VDFS_*`——它们不再住在 vdfs 插件里，前端仍按
 * `VDFS_*` 镜像，因此别名目标指向本文件（见 `protocol-mirror-audit.mjs` 的 `ALIASES`）。
 */
const ROUTE_RS_SRC = [
  'pub const ROUTE_VDFS_ROOT: &str = "vdfs/root";',
].join('\n')

/** 后端 vdfs 词表（A 组的另一常量源；词表常量住在 `vdfs/words.rs`） */
const VDFS_WORDS_RS_SRC = [
  'pub const VDFS_KIND_MESSAGES: &str = "messages";',
  'pub const VDFS_EXT_SESSION: &str = "session";',
  'pub const VDFS_EXT_MESSAGE: &str = "message";',
].join('\n')

/**
 * 后端 vdfs 域类型（D 组 9 对结构体的后端侧）——拆分后分散在 `vdfs/` 各文件，
 * 夹具按真实文件路径分发。
 */
const VDFS_TYPES_RS = {
  'crates/symbio-kernel/src/vdfs/access.rs': [rsStruct('VdfsAccess', 'read', 'write')].join('\n'),
  'crates/symbio-kernel/src/vdfs/node.rs': [
    rsStruct('VdfsNode', 'path', 'name'),
    rsStruct('VdfsItem', 'path'),
    rsStruct('VdfsNewType', 'ext', 'title'),
  ].join('\n'),
  'crates/symbio-kernel/src/vdfs/content.rs': [rsStruct('VdfsContent', 'path', 'text')].join('\n'),
  'crates/symbio-kernel/src/vdfs/error.rs': [
    rsStruct('VdfsFieldError', 'field', 'message'),
    rsStruct('VdfsValidationError', 'message', 'fields'),
  ].join('\n'),
  'crates/symbio-kernel/src/vdfs/request.rs': [rsStruct('VdfsWriteResponse', 'path', 'created')].join('\n'),
  'crates/symbio-kernel/src/vdfs/change.rs': [rsStruct('VdfsChange', 'path', 'change')].join('\n'),
}

/** 前端 vdfs.ts：只放后端有对应协议词 / 字段的——多放一个就命中「未登记」检查 */
const VDFS_TS_SRC = [
  "export const VDFS_LIST = 'vdfs/list'",
  "export const VDFS_ROOT_OP = 'vdfs/root'",
  "export const VDFS_EVENT_KIND = 'vdfs'",
  "export const VDFS_KIND_MESSAGES = 'messages'",
  "export const VDFS_EXT_SESSION = 'session'",
  "export const VDFS_EXT_MESSAGE = 'message'",
  '',
  tsIface('VdfsNode', 'path', 'name'),
  tsIface('VdfsItem', 'path'),
  tsIface('VdfsChange', 'path', 'change'),
  tsIface('VdfsAccess', 'read', 'write'),
  tsIface('VdfsContent', 'path', 'text'),
  tsIface('VdfsNewType', 'ext', 'title'),
  tsIface('VdfsWriteResponse', 'path', 'created'),
  tsIface('VdfsFieldError', 'field', 'message'),
  tsIface('VdfsValidationError', 'message', 'fields'),
].join('\n')

/**
 * 后端 detail.rs：D 组 9 对（详情表单宿主方言）的后端侧。
 *
 * 真仓库里这些结构体各自带 `#[serde(default)]` 与成片的 `skip_serializing_if`
 * ——夹具用最小形态即可：D 组比的是**字段名**，`skip_serializing_if` 是条件跳过
 * （字段仍下发），不改变字段名集合。属性处理本身由下面 D 组的专项用例覆盖。
 */
const DETAIL_RS_SRC = [
  rsStruct('DetailCondition', 'key', 'equals', 'not_equals', 'truthy', 'all'),
  rsStruct('DetailOption', 'value', 'label', 'description'),
  rsStruct(
    'DetailField',
    'key',
    'label',
    'description',
    'required',
    'widget',
    'icon',
    'visible_when',
    'disabled_when',
    'pick',
    'placeholder',
    'min',
    'max',
    'step',
    'rows',
    'options',
    'suggestions',
    'options_from_preset',
    'suggestions_from_preset',
    'full_width',
    'default',
    'form',
  ),
  rsStruct('DetailSection', 'title', 'collapsed', 'fields'),
  rsStruct('DetailPreset', 'value', 'label', 'set', 'set_always', 'options'),
  rsStruct('DetailPresetSpec', 'field', 'fill', 'presets'),
  rsStruct('DetailBadge', 'when', 'label', 'style'),
  rsStruct('DetailAction', 'id', 'label', 'style', 'icon', 'when', 'disabled_when', 'payload', 'busy_label', 'pack'),
  rsStruct(
    'DetailDefinition',
    'binding',
    'title_from',
    'title_fallback',
    'subtitle_from',
    'name_from',
    'id_from',
    'sections',
    'presets',
    'badges',
    'actions',
  ),
  '',
  // C 组：`DETAIL_PICK_*` 是**常量组**表达的闭集（不是枚举）——字面即线上取值
  'pub const DETAIL_PICK_DIRECTORY: &str = "directory";',
  'pub const DETAIL_PICK_FILE: &str = "file";',
].join('\n')

/** 前端 vdfs-form.ts：D 组 9 对的前端侧 */
const FORM_TS_SRC = [
  tsIface('DetailCondition', 'key', 'equals', 'not_equals', 'truthy', 'all'),
  tsIface('DetailOption', 'value', 'label', 'description'),
  tsIface(
    'DetailField',
    'key',
    'label',
    'description',
    'required',
    'widget',
    'icon',
    'visible_when',
    'disabled_when',
    'pick',
    'placeholder',
    'min',
    'max',
    'step',
    'rows',
    'options',
    'suggestions',
    'options_from_preset',
    'suggestions_from_preset',
    'full_width',
    'default',
    'form',
  ),
  tsIface('DetailSection', 'title', 'collapsed', 'fields'),
  tsIface('DetailPreset', 'value', 'label', 'set', 'set_always', 'options'),
  tsIface('DetailPresetSpec', 'field', 'fill', 'presets'),
  tsIface('DetailBadge', 'when', 'label', 'style'),
  tsIface('DetailAction', 'id', 'label', 'style', 'icon', 'when', 'disabled_when', 'payload', 'busy_label', 'pack'),
  tsIface(
    'DetailDefinition',
    'binding',
    'title_from',
    'title_fallback',
    'subtitle_from',
    'name_from',
    'id_from',
    'sections',
    'presets',
    'badges',
    'actions',
  ),
  '',
  // C 组：这里用**常量名**引用（真仓库 `vdfs-form.ts` 就是这种写法：词要在组件里
  // 按名使用，如 `pick === DETAIL_PICK_DIRECTORY`），顺带覆盖词表的常量解析分支
  "export const DETAIL_PICK_DIRECTORY = 'directory'",
  "export const DETAIL_PICK_FILE = 'file'",
  tsArray('DETAIL_PICKS', 'DETAIL_PICK_DIRECTORY', 'DETAIL_PICK_FILE'),
].join('\n')

/** 后端 capability/mod.rs：C 组的 `lowercase` 闭集（`CapabilityRiskLevel`） */
const RISK_LEVEL_RS_SRC = [
  rsEnum('CapabilityRiskLevel', ['Low', 'Medium', 'High'], 'lowercase'),
  '',
  // 同文件另一个 `lowercase` 枚举：前端**没有**镜像它，故不登记（登记了才会红）
  rsEnum('AutonomyLevel', ['ReadOnly', 'Supervised', 'Full'], 'lowercase'),
].join('\n')

/** 前端 session_meta.ts：`RiskLevel` 的镜像词表 */
const SESSION_META_TS_SRC = [
  tsArray('SESSION_RISK_LEVELS', "'low'", "'medium'", "'high'"),
  '',
  // 与 `RiskLevel` 同处的 `SessionMode`：后端**没有**对应枚举，故不进 C 组
  tsArray('SESSION_MODES', "'auto'", "'interactive'"),
].join('\n')

/** 后端 present/reading.rs：读数方言的 4 个结构体 + 1 个闭集 */
const READING_RS_SRC = [
  rsStruct('ReadingRow', 'label', 'value', 'hint', 'tone'),
  rsStruct('ReadingSection', 'title', 'rows', 'bar'),
  rsStruct('ReadingBar', 'level', 'marks', 'tone'),
  rsStruct('ReadingPanel', 'title', 'subtitle', 'sections'),
  '',
  rsEnum('ReadingTone', ['Normal', 'Warn'], 'lowercase'),
].join('\n')

/** 前端 schemas/reading.ts：读数方言的镜像侧（4 个接口 + 1 张词表） */
const READING_TS_SRC = [
  tsIface('ReadingRow', 'label', 'value', 'hint', 'tone'),
  tsIface('ReadingSection', 'title', 'rows', 'bar'),
  tsIface('ReadingBar', 'level', 'marks', 'tone'),
  tsIface('ReadingPanel', 'title', 'subtitle', 'sections'),
  '',
  tsArray('READING_TONES', "'normal'", "'warn'"),
].join('\n')

/** H 组：一个提到 ChatNode、且读了一个已登记 meta 键的前端生产文件 */
const META_READER_TS = 'tauri/src/registry/messageTypes.ts'
const META_READER_TS_SRC = [
  "import type { ChatNode } from '@/schemas/chat_node'",
  'export function toolNameOf(node: Pick<ChatNode, "meta">): string | undefined {',
  '  return (node.meta as Record<string, unknown> | undefined)?.tool_name as string | undefined',
  '}',
].join('\n')

/**
 * 后端 home/schemas.rs：D 组「模块限定」一族的后端侧。
 *
 * 四个模块各有一个 `Response` 而契约各不相同——这正是 `module` 限定存在的理由：
 * 不限定时抽取器按名字取文件里**第一个**匹配，四对登记会拿同一个类型比四个接口。
 * 每个模块头带一条 `Corresponding Frontend`（E 组的后端侧）。
 */
const HOME_SCHEMAS_RS = 'crates/symbio-plugin-home/src/schemas.rs'
const HOME_SCHEMAS_RS_SRC = [
  'pub mod home_reload {',
  '    //! Corresponding Frontend: tauri/src/schemas/home.ts',
  '    pub struct Request {',
  '        pub homedir: Option<String>',
  '    }',
  '    pub struct Response {',
  '        pub old_homedir: String',
  '        pub new_homedir: String',
  '        pub reloaded_plugins: usize',
  '        pub homedir_changed: bool',
  '        pub bootstrap_path: String',
  '    }',
  '}',
  'pub mod home_get_workspace {',
  '    //! Corresponding Frontend: tauri/src/schemas/home.ts',
  '    pub struct Response {',
  '        pub workdir: String',
  '        pub expanded_path: String',
  '        pub recent_workspaces: Vec<String>',
  '    }',
  '}',
  'pub mod home_set_workspace {',
  '    //! Corresponding Frontend: tauri/src/schemas/home.ts',
  '    pub struct Request {',
  '        pub path: String',
  '    }',
  '    pub struct Response {',
  '        pub workdir: String',
  '        pub expanded_path: String',
  '        pub recent_workspaces: Vec<String>',
  '    }',
  '}',
  'pub mod home_get_homedir {',
  '    //! Corresponding Frontend: tauri/src/schemas/home.ts',
  '    pub struct Response {',
  '        pub homedir: String',
  '        pub bootstrap_path: String',
  '    }',
  '}',
  '',
].join('\n')

/**
 * 前端 home.ts：namespace = 后端模块名、类型名 = 后端结构体名。
 *
 * 接口体里每行一个 `name: type`（与后端夹具同形）——D 组按行读字段，两侧同形才比得动。
 */
const HOME_TS = 'tauri/src/schemas/home.ts'
const HOME_TS_SRC = [
  '// Corresponding Backend: crates/symbio-plugin-home/src/schemas.rs',
  'export namespace home_reload {',
  '  export interface Request {',
  '    homedir?: string',
  '  }',
  '  export interface Response {',
  '    old_homedir: string',
  '    new_homedir: string',
  '    reloaded_plugins: number',
  '    homedir_changed: boolean',
  '    bootstrap_path: string',
  '  }',
  '}',
  'export namespace home_get_workspace {',
  '  export interface Response {',
  '    workdir: string',
  '    expanded_path: string',
  '    recent_workspaces: string[]',
  '  }',
  '}',
  'export namespace home_set_workspace {',
  '  export interface Request {',
  '    path: string',
  '  }',
  '  export interface Response {',
  '    workdir: string',
  '    expanded_path: string',
  '    recent_workspaces: string[]',
  '  }',
  '}',
  'export namespace home_get_homedir {',
  '  export interface Response {',
  '    homedir: string',
  '    bootstrap_path: string',
  '  }',
  '}',
  '',
].join('\n')

/** 全部一致且不含禁用常量时的最小仓库（相对仓库根的路径 → 内容） */
const BASE = {
  'crates/symbio-kernel/src/vdfs/words.rs': VDFS_WORDS_RS_SRC,
  ...VDFS_TYPES_RS,
  'crates/symbio-plugin-vdfs/src/protocol.rs': PROTOCOL_RS_SRC,
  'crates/symbio-plugin-api/src/routes.rs': ROUTE_RS_SRC,
  // E 组：这条跨栈导航头指向真实存在的 `tauri/src/schemas/vdfs.ts`
  'crates/symbio-core/src/event_bus/mod.rs':
    '// Corresponding Frontend: tauri/src/schemas/vdfs.ts\npub const EVENT_BUS_KIND_VDFS: &str = "vdfs";\n',
  'tauri/src/schemas/vdfs.ts': VDFS_TS_SRC,
  [CHAT_RS]: CHAT_RS_SRC,
  [CHAT_TS]: CHAT_TS_SRC,
  [DETAIL_RS]: DETAIL_RS_SRC,
  [FORM_TS]: FORM_TS_SRC,
  [RISK_LEVEL_RS]: RISK_LEVEL_RS_SRC,
  [SESSION_META_TS]: SESSION_META_TS_SRC,
  [READING_RS]: READING_RS_SRC,
  [READING_TS]: READING_TS_SRC,
  // D 组模块限定一族 + E 组反方向（前端 `Corresponding Backend`）
  [HOME_SCHEMAS_RS]: HOME_SCHEMAS_RS_SRC,
  [HOME_TS]: HOME_TS_SRC,
  [META_READER_TS]: META_READER_TS_SRC,
  'tauri/src/services/session.ts': 'export const x = 1\n',
}

/**
 * 在临时目录搭一个最小仓库并跑守卫。
 *
 * @param {Record<string, string>} overrides 覆盖 / 新增（值为 null 表示删掉该键）
 * @param {string[]} [extraArgs]
 */
function mirror(overrides = {}, extraArgs = []) {
  const files = { ...BASE }
  for (const [k, v] of Object.entries(overrides)) {
    if (v === null) delete files[k]
    else files[k] = v
  }
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'protocol-mirror-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `--repo=${root}`, ...extraArgs], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 20000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

// ==================== 基线 ====================

test('全部一致 → 退出码 0', () => {
  const r = mirror()
  assert.equal(r.status, 0, r.stdout)
  assert.match(
    r.stdout,
    /A 组 \d+ 条常量镜像 \+ B 组 2 项缺席检查 \+ C 组 7 张闭集词表 \+ D 组 29 对结构体字段 \+ E 组 6 条跨栈导航头 \+ G 组 ChatNode 顶层字段消费守卫 \+ H 组 1 个 meta 键读点/,
  )
})

test('真实仓库当前状态通过（防本守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 20000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout)
})

// ==================== A 组：常量镜像 ====================

test('后端改了转写列表的 kind、前端没跟 → 变红', () => {
  const r = mirror({
    'crates/symbio-kernel/src/vdfs/words.rs': [
      'pub const VDFS_KIND_MESSAGES: &str = "transcript";',
      'pub const VDFS_EXT_SESSION: &str = "session";',
      'pub const VDFS_EXT_MESSAGE: &str = "message";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_KIND_MESSAGES/)
  assert.match(r.stdout, /取值不一致/)
})

test('后端改了会话 ext、前端没跟 → 变红', () => {
  const r = mirror({
    'crates/symbio-kernel/src/vdfs/words.rs': [
      'pub const VDFS_KIND_MESSAGES: &str = "messages";',
      'pub const VDFS_EXT_SESSION: &str = "conversation";',
      'pub const VDFS_EXT_MESSAGE: &str = "message";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_EXT_SESSION/)
  assert.match(r.stdout, /取值不一致/)
})

test('后端改了 op 路由（自动发现，无需登记）→ 变红', () => {
  const r = mirror({
    'crates/symbio-plugin-vdfs/src/protocol.rs': [
      'pub const VDFS_LIST: &str = "vdfs/ls";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_LIST/)
})

test('后端改了别名目标（`ROUTE_VDFS_ROOT`）→ 变红，且报出别名关系', () => {
  // 别名目标住在宿主的 `routes.rs`（跨插件可见的路由归宿主的地址模块）。
  const r = mirror({
    'crates/symbio-plugin-api/src/routes.rs': [
      'pub const ROUTE_VDFS_ROOT: &str = "vdfs/rooted";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_ROOT_OP/)
  assert.match(r.stdout, /ROUTE_VDFS_ROOT/)
})

test('前端持有后端没有的 VDFS_* 常量且未登记 → 变红（不许静默漏网）', () => {
  const r = mirror({
    'tauri/src/schemas/vdfs.ts': `${VDFS_TS_SRC}\nexport const VDFS_TOTALLY_NEW = 'nope'\n`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_TOTALLY_NEW/)
  assert.match(r.stdout, /未登记/)
})

test('前端把常量改成别名（不再是第二份真相）→ 不红', () => {
  const r = mirror({
    'tauri/src/schemas/vdfs.ts': `${VDFS_TS_SRC}\nexport const VDFS_EXT_TEXT = VDFS_EXT_MESSAGE\n`,
  })
  // `VDFS_EXT_TEXT` 后端没有，但它右侧不是字面量 ⇒ 不进镜像集，也不触发「未登记」
  assert.equal(r.status, 0, r.stdout)
})

test('后端常量源文件被删 → 变红（不是静默跳过）', () => {
  const r = mirror({ 'crates/symbio-kernel/src/vdfs/words.rs': null })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /后端常量源不存在/)
})

test('前端常量源文件被删 → 变红（不是静默跳过）', () => {
  const r = mirror({ 'tauri/src/schemas/vdfs.ts': null })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端常量源不存在/)
})

// ==================== B 组：缺席检查 ====================

test('把会话挂载段常量写回前端生产代码 → 变红', () => {
  const r = mirror({
    'tauri/src/services/session.ts': "export const VDFS_SESSION_DIR = 'session'\n",
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_SESSION_DIR/)
})

test('把转写段常量写回前端生产代码 → 变红', () => {
  const r = mirror({
    'tauri/src/services/session.ts': "export const VDFS_SEG_MESSAGES = '消息'\n",
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_SEG_MESSAGES/)
})

test('同样的常量写在 __tests__ 里 → 不红（测试持有协议夹具是它的职责）', () => {
  const r = mirror({
    'tauri/src/services/__tests__/session.spec.ts':
      "const SCHEME = { mountDir: '.vdfs/session', messagesSeg: '消息' }\nexport const VDFS_SESSION_DIR = 'session'\n",
  })
  assert.equal(r.status, 0, r.stdout)
})

// ==================== C 组：闭集词表 ====================

test('后端枚举加了变体、前端词表没跟 → 变红', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace('    Failed,\n}', '    Failed,\n    Paused,\n}'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /NodeStatus/)
  assert.match(r.stdout, /前端词表缺少：paused/)
})

test('前端词表多出一个取值 → 变红', () => {
  const r = mirror({
    [CHAT_TS]: `${CHAT_TS_SRC}\nexport const NODE_STATUS_PAUSED = 'paused'\n`,
  })
  // 只加常量不改数组 ⇒ 不红；把常量加进数组才红
  assert.equal(r.status, 0, r.stdout)

  const r2 = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '  NODE_STATUS_FAILED,',
      '  NODE_STATUS_FAILED,\n  NODE_STATUS_PAUSED,',
    ).replace(
      "export const NODE_STATUS_FAILED = 'failed'",
      "export const NODE_STATUS_FAILED = 'failed'\nexport const NODE_STATUS_PAUSED = 'paused'",
    ),
  })
  assert.equal(r2.status, 1)
  assert.match(r2.stdout, /前端词表多出：paused/)
})

test('后端枚举丢了 rename_all = "snake_case" → 变红（否则转换会「看起来正确」）', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace(
      '#[serde(rename_all = "snake_case")]\n#[derive(Default)]\npub enum NodeRole',
      '#[derive(Default)]\npub enum NodeRole',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /NodeRole/)
  assert.match(r.stdout, /rename_all/)
})

test('词表数组的元素既不是常量也不是字面量 → 变红（漏引号的写法）', () => {
  const r = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '[CHAT_ROLE_USER, CHAT_ROLE_ASSISTANT]',
      '[CHAT_ROLE_USER, SOME_UNDEFINED_THING]',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /既不是本文件的字符串常量，也不是字符串字面量/)
})

test('词表数组允许**裸字面量**元素（词只出现一次时不必硬造常量名）', () => {
  const r = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '[CHAT_ROLE_USER, CHAT_ROLE_ASSISTANT]',
      "['user', 'assistant']",
    ),
  })
  assert.equal(r.status, 0, r.stdout)
})

test('`rename_all = "lowercase"` 的枚举也能守（C 组自动按声明取值分派转换）', () => {
  const r = mirror({
    [RISK_LEVEL_RS]: RISK_LEVEL_RS_SRC.replace(
      '    High,',
      '    High,\n    Critical,',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /RiskLevel/)
  assert.match(r.stdout, /前端词表缺少：critical/)
})

test('不支持的 `rename_all` 取值 → 报「不支持」而不是猜一个转换规则', () => {
  const r = mirror({
    [RISK_LEVEL_RS]: RISK_LEVEL_RS_SRC.replace(
      'rename_all = "lowercase"',
      'rename_all = "camelCase"',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /rename_all = "camelCase" 本检查器不支持/)
})

test('后端枚举文件被删 → 变红（不是静默跳过）', () => {
  const r = mirror({ [CHAT_RS]: null })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /后端文件不存在/)
})

test('前端词表文件被删 → 变红（不是静默跳过）', () => {
  const r = mirror({ [CHAT_TS]: null })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端文件不存在/)
})

test('词表数组被改名 → 变红（不是静默跳过）', () => {
  const r = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace('export const RESUME_ACTIONS = [', 'export const RESUME_ACTIONS_X = ['),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /未找到词表数组 RESUME_ACTIONS/)
})

// ==================== D 组：结构体字段 ====================

const VDFS_NODE_RS = 'crates/symbio-kernel/src/vdfs/node.rs'

test('读数行字段改名、前端没跟 → 变红（T-202 守卫真的在看）', () => {
  const r = mirror({
    [READING_RS]: READING_RS_SRC.replace('pub label: String,', 'pub caption: String,'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：label/)
})

test('读数语气词表多一档 → 变红（T-202 词表组真的在看）', () => {
  const r = mirror({
    [READING_TS]: READING_TS_SRC.replace("'normal'", "'normal', 'danger'"),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /前端词表多出：danger/)
})

test('后端结构体字段改名、前端没跟 → 变红', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace('pub parent_id: String,', 'pub parent_msg_id: String,'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：parent_id/)
})

test('前端多出一个字段且未登记 → 变红（不许静默漏网）', () => {
  const r = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '  id?: string\n  parent_id?: string\n}',
      '  id?: string\n  parent_id?: string\n  ghost?: string\n}',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：ghost/)
})

test('前端自持字段（tsLocal 已登记）→ 不红', () => {
  const r = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '  id?: string\n  parent_id?: string\n}',
      '  id?: string\n  parent_id?: string\n  parent?: ChatNode\n  children?: ChatNode[]\n}',
    ),
  })
  assert.equal(r.status, 0, r.stdout)
})

test('#[serde(skip_serializing)] 的字段不下发 → 前端持有它即变红', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace(
      '    pub parent_id: String,',
      '    #[serde(skip_serializing)]\n    pub parent_id: String,',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：parent_id/)
})

test('#[serde(skip_serializing_if)] 是**条件**跳过 → 字段仍下发，不红', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace(
      '    pub parent_id: String,',
      '    #[serde(skip_serializing_if = "Option::is_none")]\n    pub parent_id: String,',
    ),
  })
  assert.equal(r.status, 0, r.stdout)
})

test('#[serde(flatten)] 的字段名不出现在线格式里 → 前端持有它即变红', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace(
      '    pub parent_id: String,',
      '    #[serde(flatten)]\n    pub parent_id: String,',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：parent_id/)
})

test('#[serde(rename)] → 比对的是**线格式名**，不是 Rust 字段名', () => {
  const renamed = VDFS_TYPES_RS[VDFS_NODE_RS].replace(
    rsStruct('VdfsNode', 'path', 'name'),
    [
      'pub struct VdfsNode {',
      '    pub path: String,',
      '    #[serde(rename = "n")]',
      '    pub name: String,',
      '}',
    ].join('\n'),
  )
  // 前端仍写 `name`，而后端线格式是 `n` ⇒ 红
  const r = mirror({ [VDFS_NODE_RS]: renamed })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：name/)

  // 前端改用线格式名 ⇒ 不红
  const r2 = mirror({
    [VDFS_NODE_RS]: renamed,
    'tauri/src/schemas/vdfs.ts': VDFS_TS_SRC.replace(
      tsIface('VdfsNode', 'path', 'name'),
      tsIface('VdfsNode', 'path', 'n'),
    ),
  })
  assert.equal(r2.status, 0, r2.stdout)
})

test('后端结构体级 rename_all → 报「不支持」而不是默默算错', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace(
      'pub struct ChatNode {',
      '#[serde(rename_all = "camelCase")]\npub struct ChatNode {',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /结构体级 rename_all/)
})

test('详情方言：后端 DetailField 字段改名 → 变红（证明这批登记确实在比对）', () => {
  const r = mirror({
    [DETAIL_RS]: DETAIL_RS_SRC.replace(
      '    pub visible_when: String,',
      '    pub visible_when_x: String,',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：visible_when/)
})

test('常量组闭集：前端词表多出一个词 → 变红（反向也不许静默）', () => {
  const r = mirror({
    [FORM_TS]: FORM_TS_SRC.replace(
      'export const DETAIL_PICKS = [DETAIL_PICK_DIRECTORY, DETAIL_PICK_FILE] as const',
      "export const DETAIL_PICKS = [DETAIL_PICK_DIRECTORY, DETAIL_PICK_FILE, 'ghost'] as const",
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端词表多出：ghost/)
})

test('home 模块限定：后端该模块少一个字段 → 变红（证明限定真的在比对）', () => {
  const r = mirror({
    [HOME_SCHEMAS_RS]: HOME_SCHEMAS_RS_SRC.replace(
      ['pub mod home_get_homedir {', "    //! Corresponding Frontend: tauri/src/schemas/home.ts", '    pub struct Response {', '        pub homedir: String', '        pub bootstrap_path: String', '    }', '}'].join('\n'),
      ['pub mod home_get_homedir {', "    //! Corresponding Frontend: tauri/src/schemas/home.ts", '    pub struct Response {', '        pub homedir: String', '    }', '}'].join('\n'),
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /home_get_homedir::Response/)
  assert.match(r.stdout, /前端持有后端不存在的字段：bootstrap_path/)
})

test('home 模块限定：字段只在**另一个**同名 Response 里有 → 仍然变红', () => {
  // `old_homedir` 是 `home_reload::Response` 的字段，`home_get_homedir::Response` 里没有。
  // 不限定模块时这条会**静默通过**：按名字取文件里第一个匹配的做法在整份文件里搜得着
  // `old_homedir`，于是把错配的一对当成一致。四份不同契约必须各自锁在自己的模块体内比较。
  const r = mirror({
    [HOME_TS]: HOME_TS_SRC.replace(
      ['export namespace home_get_homedir {', '  export interface Response {', '    homedir: string'].join('\n'),
      ['export namespace home_get_homedir {', '  export interface Response {', '    homedir: string', '    old_homedir: string'].join('\n'),
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /home_get_homedir\.Response/)
  assert.match(r.stdout, /前端持有后端不存在的字段：old_homedir/)
})

test('home 模块限定：后端模块整体改名 → 报「未找到模块」而不是退回全文件搜索', () => {
  const r = mirror({
    [HOME_SCHEMAS_RS]: HOME_SCHEMAS_RS_SRC.replace('pub mod home_get_homedir', 'pub mod home_homedir'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /后端未找到模块 home_get_homedir/)
})

test('常量组不存在 → 变红（不是静默跳过）', () => {
  const r = mirror({
    [DETAIL_RS]: DETAIL_RS_SRC.replace(/pub const DETAIL_PICK_[A-Z]+: &str = "[a-z]+";/g, ''),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /后端未找到常量组 DETAIL_PICK_\*/)
})

test('详情方言取值原语：后端加一个 `DETAIL_PICK_*` → 前端词表没跟，变红', () => {
  // 「原生取值」是**通用方言**的能力（`DetailField.pick`），取值集合的唯一定义处
  // 是 `schemas/vdfs-form.ts::DETAIL_PICKS`，本条按 `DETAIL_PICK_` 前缀提取后端
  // 取值逐词比对。它原先守的是级联选项的私有原语（`OPTION_PICK_*` ↔
  // `OPTION_PICKS`），那套机制下线后随旧家一并删除——能力本身没丢，只是换了家。
  const r = mirror({
    [DETAIL_RS]: DETAIL_RS_SRC.replace(
      'pub const DETAIL_PICK_FILE: &str = "file";',
      'pub const DETAIL_PICK_FILE: &str = "file";\npub const DETAIL_PICK_COLOR: &str = "color";',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端词表缺少：color/)
})

test('详情方言取值原语：后端改了 `DETAIL_PICK_*` 的取值 → 变红（词表不是装饰）', () => {
  const r = mirror({
    [DETAIL_RS]: DETAIL_RS_SRC.replace(
      'pub const DETAIL_PICK_DIRECTORY: &str = "directory";',
      'pub const DETAIL_PICK_DIRECTORY: &str = "folder";',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端词表缺少：folder/)
  assert.match(r.stdout, /前端词表多出：directory/)
})

test('跨栈导航头指向不存在的文件 → 变红（前端改名后这条头就悬空了）', () => {
  const r = mirror({
    'crates/symbio-core/src/event_bus/mod.rs':
      '// Corresponding Frontend: tauri/src/protocols/chat_input.ts\npub const EVENT_BUS_KIND_VDFS: &str = "vdfs";\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /目标不存在/)
})

test('跨栈导航头指向真实文件 → 不红（两个方向都判）', () => {
  const r = mirror()
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /✓ 后端 symbio\/src\/symbio_core\/event_bus\/mod\.rs:1 → tauri\/src\/schemas\/vdfs\.ts/)
  // 反方向：前端 `Corresponding Backend` 同样有守卫（此前它是免检的）
  assert.match(r.stdout, /✓ 前端 tauri\/src\/schemas\/home\.ts:1 → symbio\/src\/plugins\/home\/schemas\.rs/)
})

test('前端 Corresponding Backend 悬空 → 变红（只守后端一侧时这条会静默）', () => {
  const r = mirror({
    [HOME_TS]: HOME_TS_SRC.replace(
      '// Corresponding Backend: crates/symbio-plugin-home/src/schemas.rs',
      '// Corresponding Backend: crates/symbio-core/src/schemas/home_reload.rs',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端 tauri\/src\/schemas\/home\.ts:1 → .*（目标不存在）/)
})

test('没有任何跨栈导航头 → 不红（没写不违规，写了假指针才违规）', () => {
  const r = mirror({
    'crates/symbio-core/src/event_bus/mod.rs': 'pub const EVENT_BUS_KIND_VDFS: &str = "vdfs";\n',
    [HOME_SCHEMAS_RS]: HOME_SCHEMAS_RS_SRC.replace(
      /^(\s*)\/\/! Corresponding Frontend: .*$\n/gm,
      '',
    ),
    [HOME_TS]: HOME_TS_SRC.replace('// Corresponding Backend: crates/symbio-plugin-home/src/schemas.rs\n', ''),
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /E 组 0 条跨栈导航头/)
})

// ==================== G 组：ChatNode 顶层字段消费守卫 ====================

/** 把 ChatNode 换成带某字段的版本（后端结构体与前端接口同步加，D 组不误报） */
const chatNodeWith = (extra) => ({
  [CHAT_RS]: CHAT_RS_SRC.replace(
    rsStruct('ChatNode', 'id', 'parent_id'),
    rsStruct('ChatNode', 'id', 'parent_id', extra),
  ),
  [CHAT_TS]: CHAT_TS_SRC.replace(
    tsIface('ChatNode', 'id', 'parent_id'),
    tsIface('ChatNode', 'id', 'parent_id', extra),
  ),
})

test('顶层字段只被 meta 代理消费（fake_field）→ 变红', () => {
  const r = mirror({
    ...chatNodeWith('fake_field'),
    // 唯一消费点走 node.meta.fake_field——没有任何 `node.fake_field` 的直接读法
    'tauri/src/g/reader.ts': 'export function r(node) {\n  return node.meta.fake_field\n}\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /fake_field/)
  assert.match(r.stdout, /只被 meta 代理消费/)
})

test('顶层字段被前端直接消费（total）→ 不红', () => {
  const r = mirror({
    ...chatNodeWith('total'),
    // 直接读法（`.total` 不经 meta）⇒ G 判绿；即便同时有 meta 读法也不该红
    'tauri/src/g/reader.ts':
      'export function r(node) {\n  return node.stats.total + node.meta.total\n}\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('顶层字段零命中（谁都不读）→ 不红（死字段不归 G 管）', () => {
  const r = mirror(chatNodeWith('orphaned'))
  // interface 有 orphaned，但生产码里既无 `.orphaned` 也无 `meta.orphaned`
  assert.equal(r.status, 0, r.stdout)
})

test('CLI 侧直接消费也算直接（Rust 读法）→ 不红', () => {
  const r = mirror({
    ...chatNodeWith('seq_no'),
    'crates/symbio-cli/src/render.rs': 'fn show(node: &ChatNode) -> u64 {\n    node.seq_no\n}\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

// ==================== H 组：meta 键白名单 ====================

test('未登记的 meta 读点 → 变红（新增键必须先登记）', () => {
  const r = mirror({
    [META_READER_TS]: META_READER_TS_SRC.replace('?.tool_name', '?.ghost'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /未登记的 meta 键：ghost/)
})

test('后端 owner 文件里删掉键字面 → 变红（改名 / 删除后前端会静默读 undefined）', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace('pub const META_TOOL_NAME: &str = "tool_name";', ''),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /owner 文件里已找不到这个字面/)
  assert.match(r.stdout, /tool_name/)
})

test('动态 meta 键访问 → 变红（白名单只对可枚举的读点有效）', () => {
  const r = mirror({
    [META_READER_TS]: META_READER_TS_SRC.replace(
      'return (node.meta as Record<string, unknown> | undefined)?.tool_name as string | undefined',
      'const k = "tool_name"\n  return (node.meta as Record<string, unknown> | undefined)?.[k]',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /动态 meta 键访问/)
})

test('注释与字符串里的 meta 字段名不算读点 → 不红', () => {
  const r = mirror({
    [META_READER_TS]: `${META_READER_TS_SRC}\n// meta.ghost\nexport const s = 'meta.phantom'\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

test('前端自持的键（owner: null）不要求后端存在 → 不红', () => {
  const r = mirror({
    [META_READER_TS]: `${META_READER_TS_SRC}\nexport function workdirOf(node: Pick<ChatNode, 'meta'>) {\n  return node.meta?.workdir\n}\n`,
  })
  assert.equal(r.status, 0, r.stdout)
})

test('没有提到 ChatNode 的文件不进入 H 组扫描 → `import.meta.env` 不误报', () => {
  const r = mirror({
    'tauri/src/utils/logger.ts': 'export const env = import.meta.env\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('在 ChatNode 文件里一个 meta 读点都扫不到 → 变红（读空不是全绿）', () => {
  const r = mirror({
    [META_READER_TS]:
      "import type { ChatNode } from '@/schemas/chat_node'\nexport const x: ChatNode = { id: 'x' }\n",
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /扫到 0 个 meta 读点/)
})


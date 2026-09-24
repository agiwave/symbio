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

const CHAT_RS = 'symbio/src/symbio_core/schemas/session/chat_message.rs'
const CHAT_TS = 'tauri/src/schemas/chat_message.ts'
const DETAIL_RS = 'symbio/src/symbio_core/schemas/detail.rs'
const FORM_TS = 'tauri/src/schemas/vdfs-form.ts'
const RISK_LEVEL_RS = 'symbio/src/plugins/local/policy/policy_types.rs'
const SESSION_META_TS = 'tauri/src/schemas/session_meta.ts'

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
  'pub enum MessageRole {',
  '    #[default]',
  '    User,',
  '    Assistant,',
  '}',
  '',
  '#[derive(Debug)]',
  '#[serde(rename_all = "snake_case")]',
  'pub enum MessageType {',
  '    Text,',
  '    ToolCall,',
  '    UserPrompt,',
  '}',
  '',
  '#[derive(Debug)]',
  '#[serde(rename_all = "snake_case")]',
  'pub enum MessageStatus {',
  '    Pending,',
  '    WaitingUserAction,',
  '    Failed,',
  '}',
  '',
  '#[derive(Debug)]',
  '#[serde(rename_all = "snake_case")]',
  'pub enum ResumeAction {',
  '    RetryTurn,',
  '    Retry,',
  '    Approve,',
  '}',
  '',
  rsStruct('ChatMessage', 'id', 'parent_id'),
].join('\n')

/** 前端：四张词表（常量 + 由常量数组派生的类型） */
const CHAT_TS_SRC = [
  "export const CHAT_ROLE_USER = 'user'",
  "export const CHAT_ROLE_ASSISTANT = 'assistant'",
  'export const CHAT_ROLES = [CHAT_ROLE_USER, CHAT_ROLE_ASSISTANT] as const',
  '',
  "export const MESSAGE_TYPE_TEXT = 'text'",
  "export const MESSAGE_TYPE_TOOL_CALL = 'tool_call'",
  "export const MESSAGE_TYPE_USER_PROMPT = 'user_prompt'",
  'export const MESSAGE_TYPES = [',
  '  MESSAGE_TYPE_TEXT,',
  '  MESSAGE_TYPE_TOOL_CALL,',
  '  MESSAGE_TYPE_USER_PROMPT,',
  '] as const',
  '',
  "export const MESSAGE_STATUS_PENDING = 'pending'",
  "export const MESSAGE_STATUS_WAITING_USER_ACTION = 'waiting_user_action'",
  "export const MESSAGE_STATUS_FAILED = 'failed'",
  'export const MESSAGE_STATUSES = [',
  '  MESSAGE_STATUS_PENDING,',
  '  MESSAGE_STATUS_WAITING_USER_ACTION,',
  '  MESSAGE_STATUS_FAILED,',
  '] as const',
  '',
  "export const RESUME_ACTION_RETRY_TURN = 'retry_turn'",
  "export const RESUME_ACTION_RETRY = 'retry'",
  "export const RESUME_ACTION_APPROVE = 'approve'",
  'export const RESUME_ACTIONS = [',
  '  RESUME_ACTION_RETRY_TURN,',
  '  RESUME_ACTION_RETRY,',
  '  RESUME_ACTION_APPROVE,',
  '] as const',
  '',
  tsIface('ChatMessage', 'id', 'parent_id'),
].join('\n')

/** 后端常量源之一（A 组的自动发现范围 + `VDFS_ROOT` 别名目标） */
const PROTOCOL_RS_SRC = [
  'pub const VDFS_ROOT: &str = "vdfs/root";',
  'pub const VDFS_LIST: &str = "vdfs/list";',
].join('\n')

/** 后端 vdfs_provider（A 组的另一常量源 + D 组 8 对结构体的后端侧） */
const VDFS_PROVIDER_RS_SRC = [
  'pub const VDFS_KIND_MESSAGES: &str = "messages";',
  'pub const VDFS_EXT_SESSION: &str = "session";',
  'pub const VDFS_EXT_MESSAGE: &str = "message";',
  '',
  rsStruct('VdfsNode', 'path', 'name'),
  rsStruct('VdfsChange', 'path', 'change'),
  rsStruct('VdfsAccess', 'read', 'write'),
  rsStruct('VdfsContent', 'path', 'text'),
  rsStruct('VdfsNewType', 'ext', 'title'),
  rsStruct('VdfsWriteResponse', 'path', 'created'),
  rsStruct('VdfsFieldError', 'field', 'message'),
  rsStruct('VdfsValidationError', 'message', 'fields'),
].join('\n')

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

/** 后端 policy_types.rs：C 组的 `lowercase` 闭集（`RiskLevel`） */
const RISK_LEVEL_RS_SRC = [
  rsEnum('RiskLevel', ['Low', 'Medium', 'High'], 'lowercase'),
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

/** 全部一致且不含禁用常量时的最小仓库（相对仓库根的路径 → 内容） */
const BASE = {
  'symbio/src/symbio_core/vdfs_provider.rs': VDFS_PROVIDER_RS_SRC,
  'symbio/src/plugins/vdfs/protocol.rs': PROTOCOL_RS_SRC,
  // E 组：这条跨栈导航头指向真实存在的 `tauri/src/schemas/vdfs.ts`
  'symbio/src/symbio_core/event_bus.rs':
    '// Corresponding Frontend: tauri/src/schemas/vdfs.ts\npub const KIND_VDFS: &str = "vdfs";\n',
  'tauri/src/schemas/vdfs.ts': VDFS_TS_SRC,
  [CHAT_RS]: CHAT_RS_SRC,
  [CHAT_TS]: CHAT_TS_SRC,
  [DETAIL_RS]: DETAIL_RS_SRC,
  [FORM_TS]: FORM_TS_SRC,
  [RISK_LEVEL_RS]: RISK_LEVEL_RS_SRC,
  [SESSION_META_TS]: SESSION_META_TS_SRC,
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
    /A 组 \d+ 条常量镜像 \+ B 组 2 项缺席检查 \+ C 组 6 张闭集词表 \+ D 组 18 对结构体字段 \+ E 组 1 条跨栈导航头/,
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
    'symbio/src/symbio_core/vdfs_provider.rs': [
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
    'symbio/src/symbio_core/vdfs_provider.rs': [
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
    'symbio/src/plugins/vdfs/protocol.rs': [
      'pub const VDFS_ROOT: &str = "vdfs/root";',
      'pub const VDFS_LIST: &str = "vdfs/ls";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_LIST/)
})

test('后端改了别名目标（VDFS_ROOT）→ 变红，且报出别名关系', () => {
  const r = mirror({
    'symbio/src/plugins/vdfs/protocol.rs': [
      'pub const VDFS_ROOT: &str = "vdfs/rooted";',
      'pub const VDFS_LIST: &str = "vdfs/list";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /VDFS_ROOT_OP/)
  assert.match(r.stdout, /VDFS_ROOT/)
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
  const r = mirror({ 'symbio/src/symbio_core/vdfs_provider.rs': null })
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
  assert.match(r.stdout, /MessageStatus/)
  assert.match(r.stdout, /前端词表缺少：paused/)
})

test('前端词表多出一个取值 → 变红', () => {
  const r = mirror({
    [CHAT_TS]: `${CHAT_TS_SRC}\nexport const MESSAGE_STATUS_PAUSED = 'paused'\n`,
  })
  // 只加常量不改数组 ⇒ 不红；把常量加进数组才红
  assert.equal(r.status, 0, r.stdout)

  const r2 = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '  MESSAGE_STATUS_FAILED,',
      '  MESSAGE_STATUS_FAILED,\n  MESSAGE_STATUS_PAUSED,',
    ).replace(
      "export const MESSAGE_STATUS_FAILED = 'failed'",
      "export const MESSAGE_STATUS_FAILED = 'failed'\nexport const MESSAGE_STATUS_PAUSED = 'paused'",
    ),
  })
  assert.equal(r2.status, 1)
  assert.match(r2.stdout, /前端词表多出：paused/)
})

test('后端枚举丢了 rename_all = "snake_case" → 变红（否则转换会「看起来正确」）', () => {
  const r = mirror({
    [CHAT_RS]: CHAT_RS_SRC.replace(
      '#[serde(rename_all = "snake_case")]\n#[derive(Default)]\npub enum MessageRole',
      '#[derive(Default)]\npub enum MessageRole',
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /MessageRole/)
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

const VDFS_PROVIDER = 'symbio/src/symbio_core/vdfs_provider.rs'

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
      '  id?: string\n  parent_id?: string\n  parent?: ChatMessage\n  children?: ChatMessage[]\n}',
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
  const renamed = VDFS_PROVIDER_RS_SRC.replace(
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
  const r = mirror({ [VDFS_PROVIDER]: renamed })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /前端持有后端不存在的字段：name/)

  // 前端改用线格式名 ⇒ 不红
  const r2 = mirror({
    [VDFS_PROVIDER]: renamed,
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
      'pub struct ChatMessage {',
      '#[serde(rename_all = "camelCase")]\npub struct ChatMessage {',
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
    'symbio/src/symbio_core/event_bus.rs':
      '// Corresponding Frontend: tauri/src/protocols/chat_input.ts\npub const KIND_VDFS: &str = "vdfs";\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /目标不存在/)
})

test('跨栈导航头指向真实文件 → 不红', () => {
  const r = mirror()
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /✓ symbio\/src\/symbio_core\/event_bus\.rs:1 → tauri\/src\/schemas\/vdfs\.ts/)
})

test('没有任何跨栈导航头 → 不红（没写不违规，写了假指针才违规）', () => {
  const r = mirror({
    'symbio/src/symbio_core/event_bus.rs': 'pub const KIND_VDFS: &str = "vdfs";\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /E 组 0 条跨栈导航头/)
})

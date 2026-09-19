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
 * ## 夹具为什么要铺满四张词表
 *
 * `ENUM_SETS` 是脚本里的固定清单（枚举 ↔ 词表的对应关系无法自动推断，只能登记），
 * 所以基线仓库必须把**每一条**都铺出来——少一张，那条就会报「文件/枚举不存在」，
 * 基线测试直接变红。这不是冗余：它同时证明了「夹具与 `ENUM_SETS` 没脱节」。
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
].join('\n')

/** 后端常量源之一（A 组的自动发现范围 + `VDFS_ROOT` 别名目标） */
const PROTOCOL_RS_SRC = [
  'pub const VDFS_ROOT: &str = "vdfs/root";',
  'pub const VDFS_LIST: &str = "vdfs/list";',
].join('\n')

/** 后端 vdfs_provider（A 组的另一常量源） */
const VDFS_PROVIDER_RS_SRC = [
  'pub const VDFS_KIND_MESSAGES: &str = "messages";',
  'pub const VDFS_EXT_SESSION: &str = "session";',
  'pub const VDFS_EXT_MESSAGE: &str = "message";',
].join('\n')

/** 前端 vdfs.ts：只放后端有对应协议词的常量——多放一个就命中「未登记」检查 */
const VDFS_TS_SRC = [
  "export const VDFS_LIST = 'vdfs/list'",
  "export const VDFS_ROOT_OP = 'vdfs/root'",
  "export const VDFS_EVENT_KIND = 'vdfs'",
  "export const VDFS_KIND_MESSAGES = 'messages'",
  "export const VDFS_EXT_SESSION = 'session'",
  "export const VDFS_EXT_MESSAGE = 'message'",
].join('\n')

/** 全部一致且不含禁用常量时的最小仓库（相对仓库根的路径 → 内容） */
const BASE = {
  'symbio/src/symbio_core/vdfs_provider.rs': VDFS_PROVIDER_RS_SRC,
  'symbio/src/plugins/vdfs/protocol.rs': PROTOCOL_RS_SRC,
  'symbio/src/symbio_core/event_bus.rs': 'pub const KIND_VDFS: &str = "vdfs";\n',
  'tauri/src/schemas/vdfs.ts': VDFS_TS_SRC,
  [CHAT_RS]: CHAT_RS_SRC,
  [CHAT_TS]: CHAT_TS_SRC,
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
  assert.match(r.stdout, /A 组 \d+ 条常量镜像 \+ B 组 2 项缺席检查 \+ C 组 4 张闭集词表/)
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

test('词表数组的元素不是本文件的字符串常量 → 变红', () => {
  const r = mirror({
    [CHAT_TS]: CHAT_TS_SRC.replace(
      '[CHAT_ROLE_USER, CHAT_ROLE_ASSISTANT]',
      "[CHAT_ROLE_USER, 'assistant']",
    ),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /不是本文件里的字符串常量/)
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

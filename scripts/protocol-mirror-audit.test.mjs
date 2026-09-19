/**
 * protocol-mirror-audit 回归测试
 *
 * 与 mechanism-audit.test.mjs 同一立场：**只会亮绿灯的守卫等于没有守卫**。
 * 这里对两类规则各注入真实违规：
 * - A 组（镜像对）：改值 / 删常量 / 删文件 ⇒ 必须变红；
 * - B 组（缺席检查）：把会话地址段常量写回前端 ⇒ 必须变红；
 *   但写进 `__tests__` ⇒ 必须**不**红（测试持有协议夹具是它的职责）。
 * 另有一条跑真实仓库，防本守卫在真仓库上误报。
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

/** 全部一致且不含禁用常量时的最小仓库（相对仓库根的路径 → 内容） */
const BASE = {
  'symbio/src/symbio_core/vdfs_provider.rs': [
    'pub const VDFS_KIND_MESSAGES: &str = "messages";',
    'pub const VDFS_EXT_SESSION: &str = "session";',
    'pub const VDFS_EXT_MESSAGE: &str = "message";',
  ].join('\n'),
  'tauri/src/schemas/vdfs.ts': [
    "export const VDFS_KIND_MESSAGES = 'messages'",
    "export const VDFS_EXT_SESSION = 'session'",
    "export const VDFS_EXT_MESSAGE = 'message'",
  ].join('\n'),
  'tauri/src/services/session.ts': "export const x = 1\n",
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

test('全部一致 → 退出码 0', () => {
  const r = mirror()
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /三组镜像一致 \+ 两项缺席检查通过/)
})

test('后端改了转写列表的 kind、前端没跟 → X-001 变红', () => {
  const r = mirror({
    'symbio/src/symbio_core/vdfs_provider.rs': [
      'pub const VDFS_KIND_MESSAGES: &str = "transcript";',
      'pub const VDFS_EXT_SESSION: &str = "session";',
      'pub const VDFS_EXT_MESSAGE: &str = "message";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-001/)
  assert.match(r.stdout, /取值不一致/)
})

test('后端改了会话 ext、前端没跟 → X-002 变红', () => {
  const r = mirror({
    'symbio/src/symbio_core/vdfs_provider.rs': [
      'pub const VDFS_KIND_MESSAGES: &str = "messages";',
      'pub const VDFS_EXT_SESSION: &str = "conversation";',
      'pub const VDFS_EXT_MESSAGE: &str = "message";',
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-002/)
})

test('前端常量被删 → 变红（不是静默跳过）', () => {
  const r = mirror({
    'tauri/src/schemas/vdfs.ts': [
      "export const VDFS_EXT_SESSION = 'session'",
      "export const VDFS_EXT_MESSAGE = 'message'",
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-001/)
  assert.match(r.stdout, /未找到常量 VDFS_KIND_MESSAGES/)
})

test('后端文件被删 → 变红（不是静默跳过）', () => {
  const r = mirror({ 'symbio/src/symbio_core/vdfs_provider.rs': null })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /后端文件不存在/)
})

test('把会话挂载段常量写回前端生产代码 → X-004 变红', () => {
  const r = mirror({
    'tauri/src/services/session.ts': "export const VDFS_SESSION_DIR = 'session'\n",
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-004/)
})

test('把转写段常量写回前端生产代码 → X-005 变红', () => {
  const r = mirror({
    'tauri/src/services/session.ts': "export const VDFS_SEG_MESSAGES = '消息'\n",
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-005/)
})

test('同样的常量写在 __tests__ 里 → 不红（测试持有协议夹具是它的职责）', () => {
  const r = mirror({
    'tauri/src/services/__tests__/session.spec.ts':
      "const SCHEME = { mountDir: '.vdfs/session', messagesSeg: '消息' }\nexport const VDFS_SESSION_DIR = 'session'\n",
  })
  assert.equal(r.status, 0, r.stdout)
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

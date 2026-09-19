/**
 * protocol-mirror-audit 回归测试
 *
 * 与 mechanism-audit.test.mjs 同一立场：**只会亮绿灯的守卫等于没有守卫**。
 * 这里注入三种真实偏差（改值 / 删常量 / 删文件），断言脚本确实变红；
 * 再断言一致时确实为 0，把「通过」和「没在工作」区分开。
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

/** 四组常量全部一致时的最小仓库（相对仓库根的路径 → 内容） */
const BASE = {
  'symbio/src/symbio_core/ids.rs': 'pub const PLUGIN_SESSION: &str = "session";\n',
  'symbio/src/symbio_core/vdfs_provider.rs': [
    'pub const VDFS_EXT_SESSION: &str = "session";',
    'pub const VDFS_EXT_MESSAGE: &str = "message";',
  ].join('\n'),
  'symbio/src/plugins/session/plugin/nodes.rs':
    'pub(crate) const SEG_MESSAGES: &str = "消息";\n',
  'tauri/src/schemas/vdfs.ts': [
    "export const VDFS_SESSION_DIR = 'session'",
    "export const VDFS_SEG_MESSAGES = '消息'",
    "export const VDFS_EXT_SESSION = 'session'",
    "export const VDFS_EXT_MESSAGE = 'message'",
  ].join('\n'),
}

/**
 * 在临时目录搭一个最小仓库并跑守卫。
 *
 * @param {Record<string, string>} overrides 覆盖 / 新增（值为 null 表示删掉该键）
 */
function mirror(overrides = {}) {
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
    const result = spawnSync(process.execPath, [script, `--repo=${root}`], {
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

test('四组常量一致 → 退出码 0', () => {
  const r = mirror()
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /四组跨栈常量全部一致/)
})

test('后端改了段名、前端没跟 → X-002 变红', () => {
  const r = mirror({
    'symbio/src/plugins/session/plugin/nodes.rs':
      'pub(crate) const SEG_MESSAGES: &str = "transcript";\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-002/)
  assert.match(r.stdout, /取值不一致/)
})

test('后端改了挂载名、前端没跟 → X-001 变红', () => {
  const r = mirror({
    'symbio/src/symbio_core/ids.rs': 'pub const PLUGIN_SESSION: &str = "conversations";\n',
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-001/)
})

test('前端常量被删 → 变红（不是静默跳过）', () => {
  const r = mirror({
    'tauri/src/schemas/vdfs.ts': [
      "export const VDFS_SESSION_DIR = 'session'",
      "export const VDFS_EXT_SESSION = 'session'",
      "export const VDFS_EXT_MESSAGE = 'message'",
    ].join('\n'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-002/)
  assert.match(r.stdout, /未找到常量 VDFS_SEG_MESSAGES/)
})

test('后端文件被删 → 变红（不是静默跳过）', () => {
  const r = mirror({ 'symbio/src/symbio_core/ids.rs': null })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /X-001/)
  assert.match(r.stdout, /后端文件不存在/)
})

test('真实仓库当前状态一致（防本守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 20000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout)
})

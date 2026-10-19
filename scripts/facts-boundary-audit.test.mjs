// facts-boundary-audit 回归测试
//
// 判定型守卫最危险的失效形态是**永远不命中**：规则写错、排除范围写宽、或清单漂移后
// 静默跳过——三种情况下它都会安静地亮绿灯。所以每条规则都注入真实违规并断言退出码 1；
// 另外钉住「不是违规」的三件事（存储层可开档、测试夹具可开档、注释里的词不算）。
// 最后一条跑真实仓库：夹具只能证明规则引擎自洽，证明不了十二个文件与当前结构一致。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(scriptDir, 'facts-boundary-audit.mjs')

/** 与守卫里的清单一致：夹具要让每个文件都存在，否则 FB-002 会先报清单漂移。 */
const CAPABILITY_FILES = [
  'symbio/src/plugins/loop/skill_events.rs',
  'symbio/src/plugins/loop/task_events.rs',
  'symbio/src/plugins/loop/memory_events.rs',
  'symbio/src/plugins/session/worker_progress.rs',
  'symbio/src/plugins/loop/tools/tool_executor/breaker.rs',
  'symbio/src/plugins/loop/tools/tool_executor/elimination.rs',
  'symbio/src/plugins/session/stats.rs',
  'symbio/src/symbio_core/projection/rebuild.rs',
  'symbio/src/plugins/session/transcript/inbox.rs',
  'symbio/src/plugins/session/transcript/autonomous.rs',
  'symbio/src/plugins/session/message_events.rs',
  'symbio/src/plugins/loop/derived_facts.rs',
]

/** 一棵最小仓库：十二个能力文件默认是空的，`extra` 覆盖 / 追加其它文件。 */
function repo(extra = {}) {
  const files = {}
  for (const rel of CAPABILITY_FILES) files[rel] = '// 空能力文件\n'
  return { ...files, ...extra }
}

function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'facts-boundary-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 10000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const out = (r) => r.stdout + r.stderr

// ── FB-001：生产开档只在存储层 ─────────────────────────────────────────

test('FB-001 反例：生产模块自己 open ⇒ 红', () => {
  const r = audit(
    repo({
      'symbio/src/plugins/session/turn_exec.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-001/, out(r))
  assert.match(out(r), /turn_exec\.rs:1/, out(r))
})

test('FB-001 反例：open_readonly 同样算开档', () => {
  const r = audit(
    repo({
      'symbio/src/plugins/session/stats.rs':
        'fn f() { let _ = EventWalStore::open_readonly(p); }\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-001/, out(r))
})

test('FB-001 正例：存储层与测试夹具可以直接开档', () => {
  const r = audit(
    repo({
      'symbio/src/plugins/session/store/wal.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
      'symbio/src/plugins/session/transcript/inbox.test.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
      'symbio/src/plugins/session/store/tests.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

// ── FB-002：能力面零存储知识 ───────────────────────────────────────────

test('FB-002 反例：能力文件拼路径 ⇒ 红', () => {
  const r = audit(
    repo({
      'symbio/src/plugins/loop/skill_events.rs':
        'fn f() { let p = paths::session_dir(root, id).join(V2_WAL_FILE); }\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-002/, out(r))
  assert.match(out(r), /skill_events\.rs:1/, out(r))
})

test('FB-002 正例：词只出现在注释里不算违规', () => {
  const r = audit(
    repo({
      'symbio/src/plugins/loop/tools/tool_executor/breaker.rs':
        '//! 历史教训：`open_readonly` 失败曾让台账恒 0。\n/// 路径走 `paths::session_dir`/`V2_WAL_FILE` 的写法已退场。\nfn f() {}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('FB-002 反例：清单里的文件不见了 ⇒ 红（清单与结构漂移）', () => {
  const files = repo()
  delete files['symbio/src/symbio_core/projection/rebuild.rs']
  const r = audit(files)
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-002/, out(r))
  assert.match(out(r), /rebuild\.rs/, out(r))
})

// ── 真实仓库：夹具证明不了十二个文件与当前结构一致 ─────────────────────

test('真实仓库通过（十二个能力文件与结构一致）', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30000,
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /FB-001/, out(r))
  assert.match(out(r), /FB-002/, out(r))
})

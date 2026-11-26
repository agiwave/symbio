// facts-boundary-audit 回归测试
//
// 判定型守卫最危险的失效形态是**永远不命中**：规则写错、排除范围写宽、或清单漂移后
// 静默跳过——三种情况下它都会安静地亮绿灯。所以每条规则都注入真实违规并断言退出码 1；
// 另外钉住「不是违规」的三件事（存储层可开档、测试夹具可开档、注释里的词不算）。
// 最后一条跑真实仓库：夹具只能证明规则引擎自洽，证明不了能力文件清单与当前结构一致。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(scriptDir, 'facts-boundary-audit.mjs')

/** 与守卫里的清单一致：夹具要让每个文件都存在，否则 FB-002 会先报清单漂移。 */
const CAPABILITY_FILES = [
  'crates/symbio-plugin-task/src/task.rs',
  'crates/symbio-plugin-tool/src/breaker.rs',
  'crates/symbio-plugin-session/src/stats.rs',
  'crates/symbio-plugin-session/src/rebuild.rs',
  'crates/symbio-plugin-session/src/transcript/inbox.rs',
  'crates/symbio-plugin-session/src/transcript/autonomous.rs',
  'crates/symbio-plugin-session/src/message_events.rs',
  'crates/symbio-providers/src/orchestrator/ring/facts.rs',
]

/** 单槽登记的生产侧夹具：FB-003 要恰好一家开面、FB-004 至多一家地址根，与真实仓同形。 */
const OPENER_REL = 'crates/symbio-plugin-session/src/store/fact_source.rs'
const VDFS_REL = 'crates/symbio-plugin-vdfs/src/plugin.rs'

/** 一棵最小仓库：能力文件清单默认是空的，`extra` 覆盖 / 追加其它文件。 */
function repo(extra = {}) {
  const files = {}
  for (const rel of CAPABILITY_FILES) files[rel] = '// 空能力文件\n'
  files[OPENER_REL] = 'static SESSION_FACT_OPENER: SessionFactOpener = SessionFactOpener;\nsymbio_platform::sdk::submit_fact_source_opener!(SESSION_FACT_OPENER);\n'
  files[VDFS_REL] = 'symbio_plugin_api::inventory::submit! {\n    symbio_plugin_api::AddrRootDecl(VDFS_ADDR_ROOT)\n}\n'
  return { ...files, ...extra }
}

function audit(files) {
  return withTempTree({}, (root) => {
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
  }, { prefix: 'facts-boundary-audit-' })
}

const out = (r) => r.stdout + r.stderr

// ── FB-001：生产开档只在存储层 ─────────────────────────────────────────

test('FB-001 反例：生产模块自己 open ⇒ 红', () => {
  const r = audit(
    repo({
      'crates/symbio-plugin-session/src/turn_exec.rs':
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
      'crates/symbio-plugin-session/src/stats.rs':
        'fn f() { let _ = EventWalStore::open_readonly(p); }\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-001/, out(r))
})

test('FB-001 正例：存储层与测试夹具可以直接开档', () => {
  const r = audit(
    repo({
      'crates/symbio-plugin-session/src/store/wal.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
      'crates/symbio-plugin-session/src/transcript/inbox.test.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
      'crates/symbio-plugin-session/src/store/tests.rs':
        'fn f() { let _ = EventWalStore::open(p); }\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

// ── FB-002：能力面零存储知识 ───────────────────────────────────────────

test('FB-002 反例：能力文件拼路径 ⇒ 红', () => {
  const r = audit(
    repo({
      'crates/symbio-plugin-task/src/task.rs':
        'fn f() { let p = paths::session_dir(root, id).join(V2_WAL_FILE); }\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-002/, out(r))
  assert.match(out(r), /task\.rs:1/, out(r))
})

test('FB-002 正例：词只出现在注释里不算违规', () => {
  const r = audit(
    repo({
      'crates/symbio-plugin-tool/src/breaker.rs':
        '//! 历史教训：`open_readonly` 失败曾让台账恒 0。\n/// 路径走 `paths::session_dir`/`V2_WAL_FILE` 的写法已退场。\nfn f() {}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('FB-002 反例：清单里的文件不见了 ⇒ 红（清单与结构漂移）', () => {
  const files = repo()
  delete files['crates/symbio-plugin-session/src/rebuild.rs']
  const r = audit(files)
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-002/, out(r))
  assert.match(out(r), /rebuild\.rs/, out(r))
})

// ── FB-003 / FB-004：两处单槽登记 ─────────────────────────────────────
//
// 同一个病（`inventory::iter().next()` 单槽），两个方向的判定不同：开面 0 家让读面硬错，
// 所以红；地址根 0 家只是「构建里没有 vdfs 插件」的既定降级，所以绿 + 提示。

test('FB-003 反例：两处登记开面 ⇒ 红，并点名两处', () => {
  const r = audit(
    repo({
      'crates/symbio-plugin-session/src/store/second_opener.rs':
        'static OTHER: OtherOpener = OtherOpener;\nsymbio_platform::sdk::submit_fact_source_opener!(OTHER);\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-003.*单槽/s)
  assert.match(out(r), /fact_source\.rs:2/s)
  assert.match(out(r), /second_opener\.rs:2/s)
})

test('FB-003 反例：零处登记 ⇒ 红（不是「存储边收窄完成」）', () => {
  const files = repo()
  files[OPENER_REL] = '// 空文件\n'
  const r = audit(files)
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-003.*零登记/s)
})

test('FB-003 正例：注释里写宏名不算登记；测试夹具登记也不算', () => {
  const files = repo()
  files[OPENER_REL] = '// 将来会有人写 submit_fact_source_opener!(X);\n'
  files['crates/symbio-plugin-session/src/store/fact_source.test.rs'] =
    'symbio_platform::sdk::submit_fact_source_opener!(X);\n'
  const r = audit(files)
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-003.*零登记/s, '注释与夹具都不是登记')
})

test('FB-003 正例：登记点搬家不红，但提示核对 ', () => {
  const files = repo()
  files[OPENER_REL] = '// 空文件\n'
  files['crates/symbio-plugin-memory/src/opener.rs'] = 'symbio_platform::sdk::submit_fact_source_opener!(X);\n'
  const r = audit(files)
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /FB-003[^\n]*已不在 session/s)
})

test('FB-004 反例：两处声明地址根 ⇒ 红，并点名两处', () => {
  const r = audit(
    repo({
      'crates/symbio-plugin-other/src/plugin.rs':
        'symbio_plugin_api::inventory::submit! {\n    symbio_plugin_api::AddrRootDecl("other-root")\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /FB-004.*单槽/s)
  assert.match(out(r), /symbio-plugin-vdfs\/src\/plugin\.rs:2/s)
  assert.match(out(r), /symbio-plugin-other\/src\/plugin\.rs:2/s)
})

test('FB-004 正例：零处声明 ⇒ 绿 + 提示（没有 vdfs 插件是合法降级）', () => {
  const files = repo()
  files[VDFS_REL] = '// 空文件\n'
  const r = audit(files)
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /FB-004[^\n]*零声明/s)
  assert.match(out(r), /合法降级/s)
})

test('FB-004 正例：注释与定义行都不算声明', () => {
  const files = repo()
  files[VDFS_REL] = '// symbio_plugin_api::AddrRootDecl(VDFS_ADDR_ROOT) 只是注释\n'
  files['crates/symbio-plugin-api/src/inventory.rs'] = 'pub struct AddrRootDecl(&\'static str);\n'
  const r = audit(files)
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /FB-004[^\n]*零声明/s, '注释与 pub struct 定义都不是声明')
})

// ── 真实仓库：夹具证明不了能力文件清单与当前结构一致 ─────────────────────

test('真实仓库通过（能力文件清单与结构一致）', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30000,
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /FB-001/, out(r))
  assert.match(out(r), /FB-002/, out(r))
  assert.match(out(r), /FB-003 开面登记 1 家/, out(r))
  assert.match(out(r), /FB-004 地址根声明 1 家/, out(r))
})

// projection-purity 回归测试
//
// 判定型守卫最危险的失效形态是**永远不命中**——规则写错、排除范围写宽、豁免通道
// 写死，三种都表现为安静地亮绿灯。所以这里逐条注入真实违规并断言变红，再逐条钉
// 「不是违规」的一侧（共享读侧可导入、词只在注释里、测试文件不扫、投影外有生产
// 消费方即可、空理由不算豁免），最后跑一次真实仓库——夹具只能证明规则引擎自洽，
// 证明不了「产投影清单」与当前结构一致。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(scriptDir, 'projection-purity.mjs')

const DIR = 'symbio/src/symbio_core/projection'
const MOD = `${DIR}/mod.rs`
const RECALL = `${DIR}/recall.rs`
const REPUTATION = `${DIR}/reputation.rs`
const CONSUMER = 'symbio/src/plugins/loop/x.rs'

/** 干净夹具：一个产投影 `recall.rs` + 它投影外的生产消费方。`null` ⇒ 删掉该文件。 */
function repo(extra = {}) {
  const files = {
    [MOD]: 'pub struct Projection;\nimpl Projection {\n    pub fn new(_f: impl Fn()) {}\n}\n',
    [RECALL]:
      'use super::Projection;\n\npub fn recall() -> Projection<()> {\n    Projection::new(|| ())\n}\n',
    [CONSUMER]: 'fn f() {\n    let _ = recall();\n}\n',
  }
  for (const [rel, content] of Object.entries(extra)) {
    if (content === null) delete files[rel]
    else files[rel] = content
  }
  return files
}

function audited(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'projection-purity-'))
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

test('干净夹具通过（违规用例才可归因到对应判据码）', () => {
  const r = audited(repo())
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /PP-001/, out(r))
})

// ── PP-001：产投影不导入产投影兄弟 ─────────────────────────────────────

test('PP-001 反例：`use super::<产投影兄弟>;` ⇒ 红', () => {
  const r = audited(
    repo({
      [REPUTATION]: 'use super::Projection;\n\npub fn reputation() {\n    Projection::new(|| ())\n}\n',
      [RECALL]:
        'use super::reputation;\nuse super::Projection;\n\npub fn recall() -> Projection<()> {\n    Projection::new(|| ())\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-001/, out(r))
  assert.match(out(r), /recall\.rs:1/, out(r))
})

test('PP-001 正例：框架与共享读侧（非产投影）可导入 ⇒ 绿', () => {
  const r = audited(
    repo({
      [`${DIR}/mutations.rs`]: 'pub fn snapshot_entries() -> u32 {\n    0\n}\n',
      [`${DIR}/rebuild.rs`]: 'pub fn rebuild_all() -> u32 {\n    0\n}\n',
      [RECALL]:
        'use super::Projection;\nuse super::mutations::{snapshot_entries};\nuse super::rebuild::rebuild_all;\n\npub fn recall() -> Projection<()> {\n    Projection::new(|| ())\n}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

// ── PP-002：投影体内禁隐式全局依赖 ─────────────────────────────────────

test('PP-002 反例：体内读时钟 ⇒ 红且定位到行', () => {
  const r = audited(
    repo({
      [RECALL]:
        'use super::Projection;\n\npub fn recall() -> Projection<()> {\n    let t = SystemTime::now();\n    let _ = t;\n    Projection::new(|| ())\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-002/, out(r))
  assert.match(out(r), /recall\.rs:4/, out(r))
})

test('PP-002 正例：词只出现在注释里不算 ⇒ 绿', () => {
  const r = audited(
    repo({
      [RECALL]:
        'use super::Projection;\n\n// 历史：锚曾用 SystemTime::now() 取，现改为形参 now（与 recall 同口径）。\n/// 同上：`rand::` 与 `static mut` 一并禁用。\npub fn recall() -> Projection<()> {\n    Projection::new(|| ())\n}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('PP-002 豁免：文件内一行 `projection-purity-allow PP-002: 理由` ⇒ 绿', () => {
  const r = audited(
    repo({
      [RECALL]:
        'use super::Projection;\n\n// projection-purity-allow PP-002: 夹具里这条豁免本身就是要钉的判据\npub fn recall() -> Projection<()> {\n    let t = SystemTime::now();\n    let _ = t;\n    Projection::new(|| ())\n}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

// ── PP-003：构造器必须有投影外的生产消费方 ─────────────────────────────

test('PP-003 反例：构造器在投影外无生产消费方 ⇒ 红', () => {
  const r = audited(repo({ [CONSUMER]: null }))
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-003/, out(r))
  assert.match(out(r), /`recall`/, out(r))
})

test('PP-003 反例：只被测试文件消费不算生产消费方 ⇒ 红', () => {
  const r = audited(
    repo({
      [CONSUMER]: null,
      'symbio/src/plugins/loop/x.test.rs': 'fn f() {\n    let _ = recall();\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-003/, out(r))
})

test('PP-003 正例：投影外调用该构造器 ⇒ 绿', () => {
  const r = audited(repo())
  assert.equal(r.status, 0, out(r))
})

test('PP-003 豁免：构造器名挂非空理由 ⇒ 绿；空理由 ⇒ 仍红', () => {
  const head = 'use super::Projection;\n'
  const body = 'pub fn recall() -> Projection<()> {\n    Projection::new(|| ())\n}\n'
  const ok = audited(
    repo({
      [CONSUMER]: null,
      [RECALL]: `${head}// projection-purity-allow recall: 夹具里这条豁免是判据本身\n${body}`,
    }),
  )
  assert.equal(ok.status, 0, out(ok))
  const empty = audited(
    repo({ [CONSUMER]: null, [RECALL]: `${head}// projection-purity-allow recall:\n${body}` }),
  )
  assert.equal(empty.status, 1, out(empty))
  assert.match(out(empty), /PP-003/, out(empty))
})

// ── 排除范围与「读不出」不静默 ─────────────────────────────────────────

test('排除范围：`*.test.rs` / `tests.rs` 不扫（夹具里的违规不算）⇒ 绿', () => {
  const r = audited(
    repo({
      [`${DIR}/recall.test.rs`]:
        'use super::recall;\nfn f() {\n    let _ = SystemTime::now();\n}\n',
      [`${DIR}/tests.rs`]: 'fn f() {\n    let _ = SystemTime::now();\n}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('读不出投影目录 ⇒ exit 1（判据没有输入，宁红不静默）', () => {
  const r = audited({})
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /读不出投影目录/, out(r))
})

// ── 真实仓库：夹具证明不了产投影清单与当前结构一致 ─────────────────────

test('真实仓库通过', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30000,
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /PP-003/, out(r))
})

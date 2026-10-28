// projection-purity 回归测试
//
// 判定型守卫最危险的失效形态是**永远不命中**——规则写错、排除范围写宽、豁免通道
// 写死，三种都表现为安静地亮绿灯。所以这里逐条注入真实违规并断言变红，再逐条钉
// 「不是违规」的一侧（共享读侧可导入、词只在注释里、测试文件不扫、投影外有生产
// 消费方即可、空理由不算豁免），最后跑一次真实仓库——夹具只能证明规则引擎自洽，
// 证明不了「产投影清单」与当前结构一致。
//
// PP-004 / PP-005 是**棘轮**，不是模式匹配：现值来自真实扫描（夹具恒为 1/1），判红
// 只能靠把基线收紧，所以两格的反例都走 `CORE_PROJECTION_BASELINE`；而「放宽要挂一条
// 打开得了的决策」这条，判据读本脚本自己的源码，夹具就得连假脚本一起摆进来。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(scriptDir, 'projection-purity.mjs')

const DIR = 'crates/symbio-core/src/projection'
const MOD = `${DIR}/mod.rs`
const RECALL = `${DIR}/recall.rs`
const REPUTATION = `${DIR}/reputation.rs`
const CONSUMER = 'crates/symbio-plugin-loop/src/x.rs'

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

function audited(files, env = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'projection-purity-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1', ...env },
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
      'crates/symbio-plugin-loop/src/x.test.rs': 'fn f() {\n    let _ = recall();\n}\n',
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

// ── PP-004：core 投影面只许降 ──────────────────────────────────────────
//
// 夹具只有一张产投影文件、一个构造器 ⇒ 现值恒为 `files 1 / ctors 1`。棘轮判的是
// 「现值有没有越过基线格」，所以判红只能靠把基线**收紧**（环境变量只给测试用，
// 真仓门禁不设——`gate.d/35-baseline.mjs` 另判「覆盖值不得比默认值更松」）。

test('PP-004 反例：基线收到 0:0 而实测 1 ⇒ 红，且给出下沉路径', () => {
  const r = audited(repo(), { CORE_PROJECTION_BASELINE: '0:0' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-004/, out(r))
  assert.match(out(r), /core 产投影文件数变宽：实测 1 > 基线 0/, out(r))
  assert.match(out(r), /core 投影构造器条数变宽：实测 1 > 基线 0/, out(r))
  assert.match(out(r), /下沉给那个 owner/, out(r))
  assert.match(out(r), /symbio_plugin_api::views/, out(r))
  assert.match(out(r), /D-221/, out(r))
})

test('PP-004 正例：现值不超过基线 ⇒ 绿（1/1 落在夹具放宽后的格内）', () => {
  const r = audited(repo(), { CORE_PROJECTION_BASELINE: '1:1' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /PP-004 core files 1\/1 · ctors 1\/1/, out(r))
})

test('PP-004 现值落在基线下方 ⇒ 绿但出声提醒同步基线', () => {
  const r = audited(repo(), { CORE_PROJECTION_BASELINE: '6:6' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /已收到基线下方/, out(r))
  assert.match(out(r), /files 1<6/, out(r))
})

test('PP-004 基线读不出数字 ⇒ 红（判据没有输入，宁红不静默）', () => {
  const r = audited(repo(), { CORE_PROJECTION_BASELINE: 'abc:1' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-004 基线格 `files` 不是数字/, out(r))
})

test('PP-004 基线少一格 ⇒ 红：漏掉的那格也是读不出', () => {
  const r = audited(repo(), { CORE_PROJECTION_BASELINE: '1' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-004 基线格 `ctors` 不是数字/, out(r))
})

// ── PP-005：放宽注释必须挂得住一条真实决策 ─────────────────────────────
//
// 判据读本脚本**自己**的源码，夹具里因此要连脚本一起放进来。这里的假脚本只承载注释行：
// 内容与判据无关，被扫的就是 `projection-purity-allow` 那一行。
const fakeScript = (...waivers) =>
  ['#!/usr/bin/env node', '// 夹具用的假脚本：只为了承载放宽注释。', ...waivers, ''].join('\n')

const docsFixture = {
  'docs/003-design/025-能力包与扩展面.md': '# 能力包与扩展面\n\n### D-221 投影可下沉\n\n正文。\n',
}

test('PP-005 正例：挂住夹具里真实存在的 `### D-221` ⇒ 绿', () => {
  const r = audited({
    ...repo(),
    ...docsFixture,
    'scripts/projection-purity.mjs': fakeScript(
      '// projection-purity-allow files: D-221 夹具里这条豁免本身就是要钉的判据',
    ),
  })
  assert.equal(r.status, 0, out(r))
})

test('PP-005 反例：放宽的不是投影面的格 ⇒ 红', () => {
  const r = audited({
    ...repo(),
    ...docsFixture,
    'scripts/projection-purity.mjs': fakeScript(
      '// projection-purity-allow widgets: D-221 这一格根本不存在',
    ),
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-005 `projection-purity-allow widgets` 放宽的不是投影面的格/, out(r))
})

test('PP-005 反例：理由里没有决策编号 ⇒ 红（批次需要 / 顺手都不算）', () => {
  const r = audited({
    ...repo(),
    'scripts/projection-purity.mjs': fakeScript(
      '// projection-purity-allow files: 这批急着发，先把数字抬上去',
    ),
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-005 放宽 `files` 的理由里没有决策编号/, out(r))
})

test('PP-005 反例：编号写了但锚点打不开 ⇒ 红（指向不存在的决策不算豁免）', () => {
  const r = audited({
    ...repo(),
    ...docsFixture,
    'scripts/projection-purity.mjs': fakeScript(
      '// projection-purity-allow ctors: D-999 这条决策并不存在',
    ),
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-005 放宽 `ctors` 引用了 `D-999`/, out(r))
})

test('PP-005 夹具没有本脚本 ⇒ 跳过（不误判「豁免全绿」，也不误判红）', () => {
  const r = audited(repo(), { CORE_PROJECTION_BASELINE: '1:1' })
  assert.equal(r.status, 0, out(r))
})

// ── 折叠根不止 core：判据跟着折叠走（D-221）────────────────────────────
//
// 下沉之后，`crates/*/src/projections/` 与 core 的 `projection/` 同受 PP-001 / PP-002 /
// PP-003 判定——否则「把折叠交还 owner」顺带就把这条折叠移出判据；而 PP-004 只数
// core 那一根，否则棘轮判的会变成「仓库里一共有几条折叠」。
const SESS_DIR = 'crates/symbio-plugin-session/src/projections'
const SESS_CONSUMER = 'crates/symbio-plugin-session/src/stats.rs'

test('插件折叠根：产投影兄弟互导 ⇒ PP-001 红', () => {
  const r = audited(
    repo({
      [`${SESS_DIR}/mod.rs`]: 'pub mod fallback;\npub mod slo;\n',
      [`${SESS_DIR}/fallback.rs`]:
        'use symbio_core::Projection;\n\npub fn fallback_rate() -> Projection<u8> {\n    Projection::new(|| 0)\n}\n',
      [`${SESS_DIR}/slo.rs`]:
        'use super::fallback::declared_tier;\nuse symbio_core::Projection;\n\npub fn slo_report() -> Projection<u8> {\n    let _ = declared_tier();\n    Projection::new(|| 0)\n}\n',
      [SESS_CONSUMER]: 'fn f() {\n    let _ = fallback_rate();\n    let _ = slo_report();\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-001/, out(r))
  assert.match(out(r), /slo\.rs:1/, out(r))
  assert.match(out(r), /共享读侧/, out(r))
})

test('插件折叠根：共享读侧（不产投影）可被兄弟导 ⇒ 绿', () => {
  const r = audited(
    repo({
      [`${SESS_DIR}/mod.rs`]: 'pub mod fallback;\npub mod slo;\npub mod tiers;\n',
      [`${SESS_DIR}/tiers.rs`]: 'pub(super) fn declared_tier() -> String {\n    String::new()\n}\n',
      [`${SESS_DIR}/fallback.rs`]:
        'use symbio_core::Projection;\n\npub fn fallback_rate() -> Projection<u8> {\n    Projection::new(|| 0)\n}\n',
      [`${SESS_DIR}/slo.rs`]:
        'use super::tiers::declared_tier;\nuse symbio_core::Projection;\n\npub fn slo_report() -> Projection<u8> {\n    let _ = declared_tier();\n    Projection::new(|| 0)\n}\n',
      [SESS_CONSUMER]: 'fn f() {\n    let _ = fallback_rate();\n    let _ = slo_report();\n}\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('插件折叠根：体内读时钟 ⇒ PP-002 一样红（判据不留在 core）', () => {
  const r = audited(
    repo({
      [`${SESS_DIR}/mod.rs`]: 'pub mod span;\n',
      [`${SESS_DIR}/span.rs`]:
        'use symbio_core::Projection;\n\npub fn span() -> Projection<u8> {\n    let t = SystemTime::now();\n    let _ = t;\n    Projection::new(|| 0)\n}\n',
      [SESS_CONSUMER]: 'fn f() {\n    let _ = span();\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-002/, out(r))
  assert.match(out(r), /span\.rs:4/, out(r))
})

test('插件折叠根：死构造器 ⇒ PP-003 一样红（消费方要在折叠目录之外）', () => {
  const r = audited(
    repo({
      [`${SESS_DIR}/mod.rs`]: 'pub mod checkpoint;\n',
      [`${SESS_DIR}/checkpoint.rs`]:
        'use symbio_core::Projection;\n\npub fn checkpoint() -> Projection<u8> {\n    Projection::new(|| 0)\n}\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /PP-003/, out(r))
  assert.match(out(r), /`checkpoint`/, out(r))
})

test('PP-004 只数 core 那一根：插件多一条折叠不影响棘轮 ⇒ 绿', () => {
  const r = audited(
    repo({
      [`${SESS_DIR}/mod.rs`]: 'pub mod span;\n',
      [`${SESS_DIR}/span.rs`]:
        'use symbio_core::Projection;\n\npub fn span() -> Projection<u8> {\n    Projection::new(|| 0)\n}\n',
      [SESS_CONSUMER]: 'fn f() {\n    let _ = span();\n}\n',
    }),
    { CORE_PROJECTION_BASELINE: '1:1' },
  )
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /PP-004 core files 1\/1 · ctors 1\/1/, out(r))
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

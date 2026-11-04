/**
 * loop-purity-audit 回归测试
 *
 * 判的是「环有没有悄悄把世界语义拿回来」（LP-001…005，口径见 `DE-047` §4）。
 * 这一族的失效形态是**读不出 ⇒ 没什么可判 ⇒ 通过**，所以反向用例必须钉住
 * 「红的是哪一条码」，而不是只断言 exit 1。
 *
 * 跑法：node --test scripts/loop-purity-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./loop-purity-audit.mjs', import.meta.url))
const BASELINE_REL = 'scripts/loop-purity-baseline.json'
const KNOWN_RED = 'scripts/known-red.json'
const CARGO = 'crates/symbio-plugin-loop/Cargo.toml'
const LIB = 'crates/symbio-plugin-loop/src/lib.rs'
const ROWS = 'crates/symbio-plugin-loop/src/loop_rows.rs'
const CONTROL = 'crates/symbio-plugin-loop/src/control_decl.rs'
const RING = 'crates/symbio-plugin-loop/src/ring.rs'
const STAGE = 'crates/symbio-plugin-loop/src/stage_events.rs'
const API_PORT = 'crates/symbio-plugin-api/src/port.rs'

function cleanFiles() {
  return {
    [CARGO]: '[package]\nname = "symbio-plugin-loop"\n\n[dependencies]\nsymbio-kernel = { path = "../symbio-kernel" }\nsymbio-plugin-api = { path = "../symbio-plugin-api" }\n',
    [LIB]: 'pub fn ok() -> u8 {\n    1\n}\n',
    [ROWS]: 'pub fn rows() -> u8 {\n    1\n}\n',
    [CONTROL]: 'pub fn decls() -> u8 {\n    1\n}\n',
    [RING]: 'pub fn drive() -> u8 {\n    1\n}\n',
    [STAGE]:
      'pub enum StageName {\n    Decide,\n}\n\nimpl StageName {\n    pub fn name(self) -> &\'static str {\n        match self {\n            Self::Decide => "decide",\n        }\n    }\n}\n',
    [API_PORT]:
      'pub trait PortProvider: Send + Sync {\n    fn ports(&self) -> &\'static [&\'static str];\n}\n\npub trait PortExecutor: Send + Sync {\n    async fn execute(&self, input: u64) -> u64;\n}\n',
  }
}

function cleanBaseline() {
  return {
    lp001PayloadImports: {},
    lp002ForbiddenDeps: [],
    lp003WorldRoots: {},
    lp004WorldNamedContracts: 0,
    lp005WorldPayloadSig: 0,
  }
}

/** LP-006 的规则内例外：冻结的 `turn.stage` 必须挂 D-321，不许匿名。 */
function cleanKnownRed(entries = [], exemptions) {
  return {
    note: '夹具：known-red 不是豁免',
    entries,
    exemptions: exemptions ?? [
      {
        code: 'LP-006',
        what: '效果事实名 turn.stage',
        adr: 'D-321',
        why: 'e2e 冻结断言',
        clearWhen: 'B3 展示面 turn 拆除',
      },
    ],
  }
}

function sandbox(files, baseline = cleanBaseline(), knownRed = cleanKnownRed()) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loop-purity-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  if (baseline !== null) {
    const abs = path.join(root, BASELINE_REL)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, JSON.stringify(baseline, null, 2))
  }
  if (knownRed !== null) {
    const abs = path.join(root, KNOWN_RED)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, JSON.stringify(knownRed, null, 2))
  }
  // 豁免决策号必须真的存在（`adrExists` 只读 docs）
  const adr = path.join(root, 'docs', '003-design', '048.md')
  fs.mkdirSync(path.dirname(adr), { recursive: true })
  fs.writeFileSync(adr, '### D-321 撤销 D-320\n')
  return { root, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

test('干净夹具：五面为 0 且与基线一致 ⇒ 通过', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /LP-001/)
  } finally {
    box.cleanup()
  }
})

test('LP-001 新增：loop 生产源码 import 端口载荷类型 ⇒ 红', () => {
  const files = cleanFiles()
  files[LIB] = 'use symbio_plugin_api::port::{PortPayload, ToolCallRequest};\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-001/)
    assert.match(r.out, /lib\.rs 0 → 2/)
  } finally {
    box.cleanup()
  }
})

test('LP-002 新增：Cargo 清单多一条 provider crate ⇒ 红', () => {
  const files = cleanFiles()
  files[CARGO] = files[CARGO].replace(
    'symbio-plugin-api = { path = "../symbio-plugin-api" }',
    'symbio-plugin-api = { path = "../symbio-plugin-api" }\nsymbio-provider-memory = { path = "../symbio-provider-memory" }',
  )
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-002/)
    assert.match(r.out, /symbio-provider-memory/)
  } finally {
    box.cleanup()
  }
})

test('LP-002 放行机制门面 symbio-plugin-api', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-003 新增：loop 生产源码出现 batch 词根 ⇒ 红', () => {
  const files = cleanFiles()
  files[LIB] = 'pub fn run_batch() -> u8 {\n    0\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-003/)
    assert.match(r.out, /lib\.rs 0 → 1/)
  } finally {
    box.cleanup()
  }
})

test('LP-003：注释里的词不算（注释由 D-016 管）', () => {
  const files = cleanFiles()
  files[LIB] = '// 这里曾经是一个 batch / gate 的世界\npub fn ok() -> u8 {\n    1\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-004 新增：端口受理面出现世界具名的类型 ⇒ 红', () => {
  const files = cleanFiles()
  files[API_PORT] = `${files[API_PORT]}\npub struct ModelAsk;\n`
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-004/)
    assert.match(r.out, /0 → 1/)
  } finally {
    box.cleanup()
  }
})

test('LP-004：注释里的世界词不算（名字才是接口事实）', () => {
  const files = cleanFiles()
  files[API_PORT] =
    '// 这个文件曾经有 tool / model 具名方法\n' + files[API_PORT]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-005 新增：端口 trait 签名里出现具名世界载荷 ⇒ 红', () => {
  const files = cleanFiles()
  files[API_PORT] =
    'pub trait PortProvider: Send + Sync {\n    fn ports(&self) -> &\'static [&\'static str];\n    fn model(&self) -> Option<ModelProvider>;\n}\n\npub trait PortExecutor: Send + Sync {\n    async fn execute(&self, input: u64) -> u64;\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-005/)
  } finally {
    box.cleanup()
  }
})

test('LP-005：两个端口 trait 找不到 ⇒ exit 2（读不出就不亮绿灯）', () => {
  const files = cleanFiles()
  files[API_PORT] = 'pub fn ports() -> u8 {\n    0\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /PortProvider/)
  } finally {
    box.cleanup()
  }
})

test('端口受理面文件缺失 ⇒ exit 2（不许当「没有端口接口」放行）', () => {
  const files = cleanFiles()
  delete files[API_PORT]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /端口受理面/)
  } finally {
    box.cleanup()
  }
})

test('收窄未同批下调基线 ⇒ 红（棘轮必须跟着收紧走）', () => {
  const baseline = { lp001PayloadImports: {}, lp002ForbiddenDeps: [], lp003WorldRoots: { 'lib.rs': 1 } }
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /收窄未同批下调基线/)
  } finally {
    box.cleanup()
  }
})

test('基线缺失 ⇒ exit 2（不许当「无存量」放行）', () => {
  const box = sandbox(cleanFiles(), null)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /基线/)
  } finally {
    box.cleanup()
  }
})

test('源目录缺失 ⇒ exit 2（读不出就不亮绿灯）', () => {
  const files = cleanFiles()
  for (const rel of Object.keys(files)) {
    if (rel.startsWith('crates/symbio-plugin-loop/src/')) delete files[rel]
  }
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /源目录|扫到 0/)
  } finally {
    box.cleanup()
  }
})

// ───────────────────────────── LP-006 ─────────────────────────────

test('LP-006 反例：债键含 round ⇒ 红（未登记）', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub const ROWS: &[Decl] = &[Decl { key: &["scope", "round"] }];\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-006/)
    assert.match(r.out, /\[未登记\]/)
    assert.match(r.out, /key "round"/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：eq 字面量命中阶段名 ⇒ 红', () => {
  const files = cleanFiles()
  files[ROWS] =
    'pub const PATTERN: Pattern = Pattern { until: &[("stage", Lit::Str("decide"))] };\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /stage-eq "decide"/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：eq 值经 STEP_* 常量解析命中阶段名 ⇒ 红（改名不算绿）', () => {
  const files = cleanFiles()
  files[ROWS] =
    'pub const STEP_DECIDE: &str = "decide";\n\npub const PATTERN: Pattern = Pattern { until: &[("id", Lit::Str(STEP_DECIDE))] };\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /stage-eq "decide"/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 正例：eq 值解析不到的标识符（some::PLAIN）不算阶段名 ⇒ 不红', () => {
  const files = cleanFiles()
  files[ROWS] =
    'pub const PATTERN: Pattern = Pattern { owes: &[("port", Lit::Str(some::PLAIN))] };\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：eq 的字段名是 round ⇒ 红（开账条件不许按轮）', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub const PATTERN: Pattern = Pattern { eq: &[("round", Lit::Uint(0))] };\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /eq-key "round"/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：key 含 anchor（轮锚换名）⇒ 红，行内 allow 没带决策号也红', () => {
  const files = cleanFiles()
  files[ROWS] = '// baseline-allow: anchor 是事实身份\npub const ROWS: &[Decl] = &[Decl { key: &["scope", "anchor"] }];\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /key "anchor"/)
    assert.match(r.out, /行内 allow/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 正例：key 含 anchor 但行内 baseline-allow 带真实决策号 ⇒ 不红', () => {
  const files = cleanFiles()
  files[ROWS] =
    '// baseline-allow: 该 anchor 是事实身份而不是序号 D-321\npub const ROWS: &[Decl] = &[Decl { key: &["scope", "anchor"] }];\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：行内 allow 指向不存在的决策 ⇒ 红', () => {
  const files = cleanFiles()
  files[ROWS] =
    '// baseline-allow: 事实身份 D-999\npub const ROWS: &[Decl] = &[Decl { key: &["scope", "anchor"] }];\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /不存在的决策 D-999/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：Decl 的 phase 阶段号与 anchor 排序数值 ⇒ 红', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub const ROWS: &[Decl] = &[Decl { phase: 1, anchor: 10, key: &["port"] }];\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /decl-field "phase" ×1/)
    assert.match(r.out, /decl-field "anchor" ×1/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：生产函数名 open_stage / next_round ⇒ 红', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub fn next_stage_round() -> u8 {\n    0\n}\n'
  files[CONTROL] = 'pub fn has_next_round() -> bool {\n    false\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /fn "next_stage_round"/)
    assert.match(r.out, /fn "has_next_round"/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 正例：效果事实名 turn.stage 在效果声明域 ⇒ 不红（D-321 冻结）', () => {
  const files = cleanFiles()
  files[ROWS] =
    'pub const EVENT_TURN_STAGE: &str = "turn.stage";\n\npub fn effect_facts() -> [&\'static str; 1] {\n    [EVENT_TURN_STAGE]\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /D-321/)
    assert.match(r.out, /LP-006/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 反例：turn.stage 回流成 until 判据 ⇒ 红（例外只覆盖效果声明域）', () => {
  const files = cleanFiles()
  files[ROWS] =
    'pub const PATTERN: Pattern = Pattern { until: &[("kind", Lit::Str("turn.stage"))] };\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /stage-debt "turn.stage"/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 台账形状：刀次拼错（U5）⇒ 红（刀列必须指向真实单元）', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub const ROWS: &[Decl] = &[Decl { key: &["scope", "round"] }];\n'
  const ledger = cleanKnownRed([
    {
      code: 'LP-006',
      file: ROWS,
      kind: 'key',
      token: 'round',
      count: 1,
      problem: '轮次进了债键',
      knife: '刀 U5：轮次退出债键',
      clearWhen: 'round 退出债键',
    },
  ])
  const box = sandbox(files, cleanBaseline(), ledger)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /不是已知单元/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 台账过期：实测少于登记 ⇒ 红（已清除未销账）', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub const ROWS: &[Decl] = &[Decl { key: &["scope", "round"] }];\n'
  const ledger = cleanKnownRed([
    {
      code: 'LP-006',
      file: ROWS,
      kind: 'key',
      token: 'round',
      count: 2,
      problem: '轮次进了债键',
      knife: '刀 U3：轮次退出债键',
      clearWhen: 'round 退出债键',
    },
  ])
  const box = sandbox(files, cleanBaseline(), ledger)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /台账多于实测/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 匿名豁免 ⇒ 红（例外必须带决策号）', () => {
  const files = cleanFiles()
  files[ROWS] =
    'pub const EVENT_TURN_STAGE: &str = "turn.stage";\n\npub fn effect_facts() -> [&\'static str; 1] {\n    [EVENT_TURN_STAGE]\n}\n'
  const ledger = cleanKnownRed([], [])
  const box = sandbox(files, cleanBaseline(), ledger)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /不许匿名豁免/)
  } finally {
    box.cleanup()
  }
})

test('LP-006 已知红：台账计数与实测一致 ⇒ 仍然 exit 1（known-red 不是豁免）', () => {
  const files = cleanFiles()
  files[ROWS] = 'pub const ROWS: &[Decl] = &[Decl { key: &["scope", "round"] }];\n'
  const ledger = cleanKnownRed([
    {
      code: 'LP-006',
      file: ROWS,
      kind: 'key',
      token: 'round',
      count: 1,
      problem: '轮次进了债键',
      knife: '刀 U3：轮次退出债键',
      clearWhen: 'round 退出债键',
    },
  ])
  const box = sandbox(files, cleanBaseline(), ledger)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /\[known-red\]/)
  } finally {
    box.cleanup()
  }
})

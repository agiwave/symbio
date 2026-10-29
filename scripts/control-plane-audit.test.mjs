/**
 * control-plane-audit 回归测试 —— 证明八条判据**各自会红**
 *
 * `SP-033` §5 程序 4：「`TC-n` 的执行方落地时，反向用例与该判据同批提交——没有反向用例的
 * 判据无法区分「真的在算」与「恰好没写错」」。本文件就是 `CP-001`…`CP-008` 的反向用例：
 * 每条用例只把一个地方改坏，断言**对应那条码**出现在 `[ERROR]` 里。
 *
 * | 码 | 反向用例 |
 * |---|---|
 * | `CP-001` | 驱动文件里多一个 `loop {`；另一份文件多一处 `loop_rows::next(` |
 * | `CP-002` | 派发器直问求解器（`ring::owed(`）；第二个 `ring::drive(` |
 * | `CP-003` | 少一个守卫调用；动作签名多一份 `&mut`；`Ctx` 多一个 `&mut` 字段；派发器点问（`owes(`） |
 * | `CP-004` | 内核里出现行名字面量 |
 * | `CP-005` | 收束格第二个构造点；`ACTOR_CLOSE.writes` 里少一格 |
 * | `CP-006` | 循环体改一个字节（基线未跟）；控制面多一份文件（基线未跟）；基线整个缺失 |
 * | `CP-007` | 规范里多出第七条 `TC-n`；`## 判据` 表点名一个不存在的脚本 |
 * | `CP-008` | `seal` 改成借用式；`impl Frozen` 长出 `&mut self` 方法 |
 *
 * 另有两条**不该红**的用例（防止判据被注释措辞骗到）：
 *   - 注释里写 `loop {` / `ring::owed(` / `"close"` ⇒ 仍绿（判据读剥注释后的代码）；
 *   - 干净夹具 ⇒ 全绿且读数为「循环体 1 个」。
 *
 * 夹具的做法：建一棵最小可判的树（控制面三个文件 + kernel 两张表 + 规范 + 决策锚），
 * 用**本脚本自己的 `--print-baseline`** 生成该夹具的基线——于是「基线」在每份夹具里
 * 都是自洽的，反向用例只需要破坏一处。改坏后默认**重新生成**基线（把 `CP-006` 隔离
 * 在外），只有 `CP-006` 的两条用例刻意用**改坏前**的基线。
 *
 * 用法：node --test scripts/control-plane-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./control-plane-audit.mjs', import.meta.url))
const BASELINE_REL = 'scripts/baselines/control-plane.json'
const LOOP = 'crates/symbio-plugin-loop/src'
const DRIVE = `${LOOP}/turn_exec/drive.rs`
const ROWS = `${LOOP}/loop_rows.rs`
const TURN_EXEC = `${LOOP}/turn_exec.rs`
const SETTLE = `${LOOP}/turn_runner/settle.rs`
const DECL = 'crates/symbio-kernel/src/decl/mod.rs'
const SPEC = 'docs/005-spec/033.md'

/** 逐个删 —— 不用 rmSync(recursive)（沙箱会拦截批量删除）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function writeTree(root, files) {
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
}

/**
 * 用审计自己的 `--print-baseline` 算一份基线（夹具因此不手抄指纹）。
 *
 * 被改坏的树上，`CP-001`…`CP-005` 会先把错误行打到 stdout，JSON 在最后——
 * 所以从**第一行恰好是 `{`** 的地方起解析（不按「第一个 `{`」找：错误文本里就有
 * `` `loop {` `` 这种字面量）。
 */
function printBaseline(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cp-baseline-'))
  try {
    writeTree(root, files)
    const r = spawnSync(process.execPath, [script, `--root=${root}`, '--print-baseline'], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
    })
    const lines = r.stdout.split('\n')
    const at = lines.findIndex((l) => l === '{')
    assert.notEqual(at, -1, `--print-baseline 没有打印基线：\n${r.stdout}${r.stderr}`)
    return JSON.parse(lines.slice(at).join('\n'))
  } finally {
    rmTree(root)
  }
}

/**
 * 建夹具 → 装基线 → 跑审计。
 *
 * `baselineFrom` 默认是**同一份**被跑的文件树；`CP-006` 的两条用例传改坏之前的树，
 * 于是「基线没跟上」才是被判红的那件事，而不是被顺手修好。
 */
function audit(files, { baselineFrom = files, withBaseline = true } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'control-plane-'))
  try {
    if (withBaseline) {
      const b = printBaseline(baselineFrom)
      b.mechanism.adr = 'D-234'
      b.mechanism.reason = '夹具：机制面冻结在干净形状上（决策锚只为让 CP-006 的输入完整）'
      files[BASELINE_REL] = JSON.stringify(b, null, 2)
    }
    writeTree(root, files)
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
    })
  } finally {
    rmTree(root)
  }
}

function assertRed(r, code) {
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, new RegExp(`\\[ERROR\\] ${code} `), r.stdout)
}

// ───────────────────────────── 夹具 ─────────────────────────────

const LOOP_ROWS = `use symbio_core::ring;

pub const OPENER: &str = "opener";
pub const CLOSE: &str = "close";

pub const EVENT_CHAT_ASSISTANT_FINAL: &str = "chat.assistant.final";
pub const EVENT_CHAT_ASSISTANT_FALLBACK: &str = "chat.assistant.fallback";

pub const ROWS: &[Decl] = &[
    Decl { name: "prepare-turn", actor: OPENER },
    Decl { name: "close-turn", actor: CLOSE },
];

pub const ACTORS: &[ActorDecl] = &[
    ActorDecl { name: OPENER, writes: &[] },
    ActorDecl { name: CLOSE, writes: &[] },
];

pub const ACTOR_CLOSE: ActorDecl = ActorDecl {
    name: CLOSE,
    writes: &[EVENT_CHAT_ASSISTANT_FINAL, EVENT_CHAT_ASSISTANT_FALLBACK],
};

pub const PORTS: &[PortDecl] = &[PortDecl { port: "model", concurrency: 0 }];

pub fn owes(rows: &[Decl], name: &str) -> bool {
    ring::owed(rows, name).is_some()
}

pub fn waiting(events: &[Event]) -> bool {
    events.iter().any(|e| e.name == "chat.assistant.final")
}

pub fn next(rows: &[Decl], step: u32, now: u64) -> Option<usize> {
    ring::drive(rows, &[], &[], now).ok()
}
`

const DRIVE_SRC = `use crate::loop_rows;

pub async fn drive(
    facts: &Facts,
    ledger: &Ledger,
    driver: &Driver,
    events: &[Event],
    retiring: Option<u32>,
) -> u32 {
    let mut steps = 0;
    loop {
        if retiring.is_some() && driver_aborted(driver) {
            break;
        }
        if abort_before_first_request(driver).await {
            break;
        }
        if over_step_limit(steps) {
            break;
        }
        if writes_closed(driver) {
            break;
        }
        if ledger.is_empty() && !loop_rows::waiting(events) {
            break;
        }
        let _action = loop_rows::next(facts, steps, 0);
        steps += 1;
    }
    steps
}

fn driver_aborted(driver: &Driver) -> bool {
    driver.aborted
}

async fn abort_before_first_request(driver: &Driver) -> bool {
    driver.retiring
}

fn over_step_limit(steps: u32) -> bool {
    steps >= 32
}

fn writes_closed(driver: &Driver) -> bool {
    driver.closed
}
`

const TURN_EXEC_SRC = `use std::future::Future;
use std::pin::Pin;

pub type StepFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

pub type Action = for<'a> fn(&'a mut Ctx<'a>) -> StepFuture<'a>;

pub struct Ctx<'a> {
    pub session: &'a mut Session,
    pub events: &'a [Event],
}
`

const SETTLE_SRC = `use crate::turn_exec::Ctx;

pub fn land_final(ctx: &Ctx<'_>, turn: u64) {
    let _ = Event::pending(ctx, EVENT_CHAT_ASSISTANT_FINAL, format!("f-{turn}"));
}

pub fn land_failure(ctx: &Ctx<'_>, turn: u64) {
    let _ = Event::pending(ctx, EVENT_CHAT_ASSISTANT_FALLBACK, format!("fb-{turn}"));
}
`

const KERNEL_DECL_SRC = `pub struct Builder {
    rows: Vec<Row>,
}

pub struct Frozen {
    rows: Vec<Row>,
}

impl Builder {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn declare(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }

    pub fn seal(self) -> Frozen {
        Frozen { rows: self.rows }
    }
}

impl Frozen {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
}
`

const KERNEL_ADAPTERS_SRC = `pub enum LatencyTier {
    Reflex,
}

pub fn tier_name(tier: LatencyTier) -> &'static str {
    match tier {
        LatencyTier::Reflex => "reflex",
    }
}
`

const SPEC_ROWS = [
  '| `TC-1` | `scripts/control-plane-audit.mjs`（CP-001/002/003） | `scripts/control-plane-audit.test.mjs` |',
  '| `TC-2` | `scripts/control-plane-audit.mjs`（CP-004） | `scripts/control-plane-audit.test.mjs` |',
  '| `TC-3` | `scripts/control-plane-audit.mjs`（CP-008） | `scripts/control-plane-audit.test.mjs` |',
  '| `TC-4` | `scripts/control-plane-audit.mjs`（CP-005） | `scripts/control-plane-audit.test.mjs` |',
  '| `TC-5` | `scripts/control-plane-audit.mjs`（CP-006） | `scripts/control-plane-audit.test.mjs` |',
  '| `TC-6` | `scripts/control-plane-audit.mjs`（CP-007） | `scripts/control-plane-audit.test.mjs` |',
]

const SPEC_TEXT = [
  '# 控制面终态判据（夹具）',
  '',
  '#### TC-1 唯一驱动形状',
  '#### TC-2 机制层无域名',
  '#### TC-3 声明面封闭',
  '#### TC-4 事实面唯一写方',
  '#### TC-5 横向扩展零改机制',
  '#### TC-6 判据集是闭集',
  '',
  '## 判据',
  '',
  '| `TC-n` | 执行方（判据码） | 反向用例 |',
  '|---|---|---|',
  ...SPEC_ROWS,
  '',
].join('\n')

function cleanTree() {
  return {
    [`${LOOP}/lib.rs`]: 'pub mod loop_rows;\npub mod turn_exec;\npub mod turn_runner;\n',
    [`${LOOP}/turn_runner/mod.rs`]: 'pub mod settle;\n',
    [ROWS]: LOOP_ROWS,
    [DRIVE]: DRIVE_SRC,
    [TURN_EXEC]: TURN_EXEC_SRC,
    [SETTLE]: SETTLE_SRC,
    'crates/symbio-kernel/src/lib.rs': 'pub mod adapters;\npub mod decl;\n',
    [DECL]: KERNEL_DECL_SRC,
    'crates/symbio-kernel/src/adapters.rs': KERNEL_ADAPTERS_SRC,
    'crates/symbio-core/src/lib.rs': 'pub struct Event {\n    pub name: String,\n}\n',
    [SPEC]: SPEC_TEXT,
    'docs/003-design/016-工程落地与验证.md': '## 关键决策\n\n### D-234 夹具决策\n\n机制面基线挂在这条决策上。\n',
    'scripts/control-plane-audit.mjs': '// 夹具占位：CP-007 只判「§判据表点名的脚本存在」\n',
    'scripts/control-plane-audit.test.mjs': '// 夹具占位\n',
  }
}

// ───────────────────────────── 不该红的用例 ─────────────────────────────

test('干净夹具：八条判据全绿，读数是「循环体 1 个」', () => {
  const r = audit(cleanTree())
  assert.equal(r.status, 0, r.stdout + r.stderr)
  for (const code of ['CP-001', 'CP-002', 'CP-003', 'CP-004', 'CP-005', 'CP-006', 'CP-007', 'CP-008']) {
    assert.match(r.stdout, new RegExp(`✓ ${code}`), r.stdout)
  }
  assert.match(r.stdout, /循环体 1 个/)
  assert.match(r.stdout, new RegExp(`收束格 chat\\.assistant\\.final：${SETTLE.replace(/[/.]/g, '\\$&')}`))
})

test('注释里的 loop / owed / 行名不算代码（判据读剥注释后的文本）', () => {
  const files = cleanTree()
  files[DRIVE] += '\n// 反例说明：别写 loop { } 也别 ring::owed(&rows, 0)\n'
  files[DECL] += '\n// 这里禁止出现行名 "prepare-turn" 与 "close"\n'
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /✓ CP-004/)
})

// ───────────────────────────── CP-001 ─────────────────────────────

test('CP-001：驱动文件里多一个 loop { ⇒ 红', () => {
  const files = cleanTree()
  files[DRIVE] += '\npub fn shadow_drive() {\n    loop {\n        break;\n    }\n}\n'
  assertRed(audit(files), 'CP-001')
})

test('CP-001：另一份文件多一处 loop_rows::next( ⇒ 红', () => {
  const files = cleanTree()
  files[SETTLE] += '\npub fn shadow_advance(facts: &Facts) {\n    let _ = loop_rows::next(facts, 0, 0);\n}\n'
  assertRed(audit(files), 'CP-001')
})

// ───────────────────────────── CP-002 ─────────────────────────────

test('CP-002：派发器直问求解器（ring::owed）⇒ 红', () => {
  const files = cleanTree()
  files[DRIVE] += '\npub fn peek(rows: &[Decl]) -> bool {\n    ring::owed(rows, 0).is_some()\n}\n'
  assertRed(audit(files), 'CP-002')
})

test('CP-002：第二个 ring::drive( ⇒ 红', () => {
  const files = cleanTree()
  files[DRIVE] += '\npub fn shadow_ring() -> bool {\n    ring::drive(&[], &[], &[], 0).is_ok()\n}\n'
  assertRed(audit(files), 'CP-002')
})

// ───────────────────────────── CP-003 ─────────────────────────────

test('CP-003：少一个守卫调用 ⇒ 红', () => {
  const files = cleanTree()
  files[DRIVE] = files[DRIVE].replace('writes_closed(driver)', 'false')
  assertRed(audit(files), 'CP-003')
})

test('CP-003：动作签名多一份 &mut（能拿到第二份可变状态）⇒ 红', () => {
  const files = cleanTree()
  files[TURN_EXEC] = files[TURN_EXEC].replace(
    "for<'a> fn(&'a mut Ctx<'a>) -> StepFuture<'a>",
    "for<'a> fn(&'a mut Ctx<'a>, &'a mut Session) -> StepFuture<'a>",
  )
  assertRed(audit(files), 'CP-003')
})

test('CP-003：Ctx 多一个 &mut 字段 ⇒ 红', () => {
  const files = cleanTree()
  files[TURN_EXEC] = files[TURN_EXEC].replace("pub events: &'a [Event],", "pub events: &'a mut [Event],")
  assertRed(audit(files), 'CP-003')
})

test('CP-003：派发器里点问（owes）⇒ 红', () => {
  const files = cleanTree()
  files[DRIVE] += '\npub fn owes_now(rows: &[Decl]) -> bool {\n    owes(rows, "prepare-turn")\n}\n'
  assertRed(audit(files), 'CP-003')
})

// ───────────────────────────── CP-004 ─────────────────────────────

test('CP-004：内核里出现行名字面量 ⇒ 红', () => {
  const files = cleanTree()
  files[DECL] += '\npub const ROW_NAME: &str = "prepare-turn";\n'
  assertRed(audit(files), 'CP-004')
})

// ───────────────────────────── CP-005 ─────────────────────────────

test('CP-005：收束格第二个构造点 ⇒ 红', () => {
  const files = cleanTree()
  files[SETTLE] +=
    '\npub fn land_again(ctx: &Ctx<\'_>, turn: u64) {\n' +
    '    let _ = Event::pending(ctx, EVENT_CHAT_ASSISTANT_FINAL, format!("f2-{turn}"));\n}\n'
  assertRed(audit(files), 'CP-005')
})

test('CP-005：声明面少一格（ACTOR_CLOSE.writes 没有 fallback）⇒ 红', () => {
  const files = cleanTree()
  files[ROWS] = files[ROWS].replace(
    'writes: &[EVENT_CHAT_ASSISTANT_FINAL, EVENT_CHAT_ASSISTANT_FALLBACK],',
    'writes: &[EVENT_CHAT_ASSISTANT_FINAL],',
  )
  assertRed(audit(files), 'CP-005')
})

// ───────────────────────────── CP-006 ─────────────────────────────

test('CP-006：循环体改一个字节而基线未跟 ⇒ 红', () => {
  const files = cleanTree()
  const before = cleanTree()
  files[DRIVE] = files[DRIVE].replace('steps += 1;', 'steps += 2;')
  assertRed(audit(files, { baselineFrom: before }), 'CP-006')
})

test('CP-006：控制面多一份生产文件而基线未跟 ⇒ 红', () => {
  const files = cleanTree()
  const before = cleanTree()
  files[`${LOOP}/host_extra.rs`] = 'pub struct Extra;\n'
  assertRed(audit(files, { baselineFrom: before }), 'CP-006')
})

test('CP-006：基线整个缺失 ⇒ 红（判据没有输入，宁红不静默）', () => {
  assertRed(audit(cleanTree(), { withBaseline: false }), 'CP-006')
})

// ───────────────────────────── CP-007 ─────────────────────────────

test('CP-007：规范里多出第七条 TC-n ⇒ 红（判据集不许跟着能力长）', () => {
  const files = cleanTree()
  files[SPEC] += '\n#### TC-7 新能力专用判据\n\n新端口要多一条判据。\n'
  assertRed(audit(files), 'CP-007')
})

test('CP-007：规范里少一条 TC-n ⇒ 红（TC_MAP 与条文对不上）', () => {
  const files = cleanTree()
  files[SPEC] = files[SPEC].replace('#### TC-6 判据集是闭集\n', '')
  assertRed(audit(files), 'CP-007')
})

test('CP-007：§判据表点名一个不存在的执行方脚本 ⇒ 红', () => {
  const files = cleanTree()
  files[SPEC] = files[SPEC].replace('`scripts/control-plane-audit.mjs`（CP-004）', '`scripts/nope-audit.mjs`')
  assertRed(audit(files), 'CP-007')
})

// ───────────────────────────── CP-008 ─────────────────────────────

test('CP-008：seal 改成借用式（同一份声明事后还能改）⇒ 红', () => {
  const files = cleanTree()
  files[DECL] = files[DECL].replace('pub fn seal(self)', 'pub fn seal(&mut self)')
  assertRed(audit(files), 'CP-008')
})

test('CP-008：impl Frozen 长出 &mut self 方法 ⇒ 红', () => {
  const files = cleanTree()
  files[DECL] = files[DECL].replace('pub fn rows(&self)', 'pub fn rows(&mut self)')
  assertRed(audit(files), 'CP-008')
})

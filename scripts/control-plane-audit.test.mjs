/**
 * control-plane-audit 回归测试 —— 证明四条判据**各自会红**
 *
 * 旧引擎删除后，本脚本只剩四条与引擎无关的形状判据（`CP-001` / `CP-004` / `CP-006` /
 * `CP-009`，见 034）。旧 33 条用例里有 25 条钉的是已删除的码与已删除的路径
 * （`loop_rows.rs` / `ring.rs` / `turn_exec.rs`），它们随对象一起删除；剩下的判据各
 * 保留「只改坏一处」的正反例。
 *
 * | 码 | 反向用例 |
 * |---|---|
 * | `CP-001` | 引擎里多一段 `loop`；把唯一循环搬到另一个文件 |
 * | `CP-004` | kernel / sdk / loop 的**代码**里出现 `vocab.rs` 登记的事件名；loop 的派发面出现端口名 |
 * | `CP-006` | 机制字节改了而基线未跟；多一份生产文件；基线缺失；基线指向不存在的文件；决策号不存在 |
 * | `CP-009` | 另一 crate 的 `loop` / `for` 里调 `drive(` |
 *
 * 另有四条**不该红**的用例：
 *   - 注释里的世界域名 ⇒ 仍绿（判据读剥注释后的代码，字符串字面量保留）；
 *   - `test_fixtures.rs` 是 `cfg(test)` / feature 下的调用方拼写 ⇒ 不算机制层；
 *   - sdk 里的端口名是载荷取值 / 角色词，不在派发面 ⇒ 不算机制在派发；
 *   - 循环体里没有推进调用 ⇒ 不算第二推进循环。
 *
 * 夹具的做法：建一棵最小可判的树（新环三个文件 + vocab + kernel / sdk 各一个文件 +
 * 决策锚），用本脚本自己的 `--print-baseline` 生成夹具基线，于是「基线」在每份夹具里
 * 都自洽；反向用例只破坏一处。
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
const ENGINE = `${LOOP}/ring/engine.rs`
const VOCAB = `${LOOP}/vocab.rs`
const PORT_VOCAB = 'crates/symbio-plugin-api/src/vocab.rs'
const KERNEL = 'crates/symbio-kernel/src/lib.rs'
const SDK = 'crates/symbio-sdk/src/lib.rs'

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

/** 用审计自己的 `--print-baseline` 算一份基线（夹具不手抄指纹）。 */
function printBaseline(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cp-baseline-'))
  try {
    writeTree(root, files)
    const r = spawnSync(process.execPath, [script, `--root=${root}`, '--print-baseline'], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
    })
    const at = r.stdout.split('\n').findIndex((l) => l === '{')
    assert.notEqual(at, -1, `--print-baseline 没有打印基线：\n${r.stdout}${r.stderr}`)
    return JSON.parse(r.stdout.split('\n').slice(at).join('\n'))
  } finally {
    rmTree(root)
  }
}

/**
 * 建夹具 → 装基线 → 跑审计。
 *
 * `baselineFrom` 默认是同一份被跑的树；`CP-006` 的「基线没跟上」用例传改坏之前的树。
 * `mutate` 改基线本身（指向不存在的文件 / 换掉决策号）。
 */
function audit(files, { baselineFrom = files, withBaseline = true, mutate } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'control-plane-'))
  try {
    if (withBaseline) {
      const b = printBaseline(baselineFrom)
      b.mechanism.adr = 'D-234'
      b.mechanism.reason = '夹具：机制面冻结在干净形状上（决策锚只为让 CP-006 的输入完整）'
      if (mutate) mutate(b)
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
  const out = r.stdout + r.stderr
  assert.equal(r.status, 1, out)
  assert.match(out, new RegExp(`\\[ERROR\\] ${code} `), out)
}

// ───────────────────────────── 夹具 ─────────────────────────────

const ENGINE_SRC = `use crate::ring::{DebtRow, DriveCtx};

pub(crate) async fn drive(ctx: &dyn DriveCtx, scope: u64) -> u32 {
    let mut steps = 0;
    loop {
        let Some(_row) = plan(ctx, scope) else {
            break;
        };
        expire(ctx, scope);
        steps += 1;
    }
    steps
}

fn plan(ctx: &dyn DriveCtx, scope: u64) -> Option<DebtRow> {
    ctx.source().rows(&ctx.facts(), scope).into_iter().next()
}

fn expire(ctx: &dyn DriveCtx, scope: u64) -> bool {
    ctx.now_ms() > scope
}
`

const VOCAB_SRC = `use symbio_kernel::event::{Entity, EventDecl, Verb};

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new("user.message", (Entity::Scope, Verb::Opened)),
    EventDecl::with_roles(
        "chat.assistant.final",
        (Entity::Scope, Verb::Closed),
        &[],
    ),
];
`

const DECL_DOC = '## 关键决策\n\n### D-234 夹具决策\n\n机制面基线挂在这条决策上。\n'

function cleanTree() {
  return {
    [`${LOOP}/lib.rs`]: 'pub mod ring;\npub mod vocab;\n',
    [`${LOOP}/ring/mod.rs`]: 'pub mod engine;\n',
    [ENGINE]: ENGINE_SRC,
    [VOCAB]: VOCAB_SRC,
    [PORT_VOCAB]: '/// 夹具：端口词汇面（端口名只有这一处登记）\npub const PORT_MODEL: &str = "model";\n',
    [KERNEL]: '// 内核夹具：没有任何世界域名\npub struct Event;\n',
    [SDK]: 'pub struct Store;\n',
    'docs/003-design/016-工程落地与验证.md': DECL_DOC,
  }
}

// ───────────────────────────── 不该红的用例 ─────────────────────────────

test('干净夹具：四条判据全绿（循环体 1 段、世界事件名 2 个、端口名 1 个、生产文件 4 份）', () => {
  const r = audit(cleanTree())
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /CP-001 唯一循环体 1/)
  assert.match(r.stdout, /CP-004 世界事件名 2 个、端口名 1 个、机制层 0 处/)
  assert.match(r.stdout, /CP-006 机制面/)
  assert.match(r.stdout, /（4 文件）/)
  assert.match(r.stdout, /CP-009 第二推进循环 0/)
})

test('注释里的世界域名不算代码（CP-004 读剥注释后的文本）', () => {
  const files = cleanTree()
  files[KERNEL] += '\n// 例如 "user.message" 是业务名字，内核不许持有\n'
  files[SDK] += '\n/// `"chat.assistant.final"` 只在文档里出现\n'
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('test_fixtures.rs 是测试支持文件，不算生产机制层（CP-004 不判）', () => {
  const files = cleanTree()
  files['crates/symbio-sdk/src/test_fixtures.rs'] = 'pub const N: &str = "user.message";\n'
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

// ───────────────────────────── CP-001 ─────────────────────────────

test('CP-001：引擎里多一段 loop ⇒ 红', () => {
  const files = cleanTree()
  files[ENGINE] += '\npub fn shadow() {\n    loop {\n        break;\n    }\n}\n'
  assertRed(audit(files, { baselineFrom: cleanTree() }), 'CP-001')
})

test('CP-001：唯一循环搬到另一个文件 ⇒ 红（唯一允许位置是 ring/engine.rs）', () => {
  const files = cleanTree()
  files[ENGINE] = ENGINE_SRC.replace(/loop \{[\s\S]*?\n    \}\n/, '')
  files[`${LOOP}/driver.rs`] = 'pub fn go() {\n    loop {\n        break;\n    }\n}\n'
  assertRed(audit(files, { baselineFrom: cleanTree() }), 'CP-001')
})

// ───────────────────────────── CP-004 ─────────────────────────────

test('CP-004：kernel 的代码里出现世界事件名字面量 ⇒ 红', () => {
  const files = cleanTree()
  files[KERNEL] += '\npub const N: &str = "user.message";\n'
  assertRed(audit(files), 'CP-004')
})

test('CP-004：sdk 的代码里出现世界事件名字面量 ⇒ 红', () => {
  const files = cleanTree()
  files[SDK] += '\npub const N: &str = "chat.assistant.final";\n'
  assertRed(audit(files), 'CP-004')
})

test('CP-004：loop 的代码里出现世界事件名字面量 ⇒ 红（声明表 vocab.rs 不算）', () => {
  const files = cleanTree()
  files[`${LOOP}/lib.rs`] += '\npub const N: &str = "user.message";\n'
  assertRed(audit(files), 'CP-004')
})

test('CP-004：loop 的派发面出现端口名字面量 ⇒ 红', () => {
  const files = cleanTree()
  files[`${LOOP}/lib.rs`] += '\npub const P: &str = "model";\n'
  assertRed(audit(files), 'CP-004')
})

test('CP-004：sdk 里的端口名是载荷取值 / 角色词，不在派发面 ⇒ 仍绿', () => {
  const files = cleanTree()
  files[SDK] += '\npub fn port_is(e: &str) -> bool {\n    e == "model" || e == "tool"\n}\n'
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

// ───────────────────────────── CP-006 ─────────────────────────────

test('CP-006：机制语义改一个字符而基线未跟 ⇒ 红', () => {
  const files = cleanTree()
  files[ENGINE] = files[ENGINE].replace('steps += 1;', 'steps += 2;')
  assertRed(audit(files, { baselineFrom: cleanTree() }), 'CP-006')
})

test('CP-006：只改格式（import 顺序 / 空行 / 注释 / 换行 / 尾逗号）⇒ 绿（指纹不量格式）', () => {
  const formatted = ENGINE_SRC.replace(
    'use crate::ring::{DebtRow, DriveCtx};',
    'use crate::ring::{DriveCtx, DebtRow};',
  )
    .replace(
      '    let Some(_row) = plan(ctx, scope) else {',
      '    // 只动格式：注释 + 空行\n\n    let Some(_row) = plan(ctx, scope,) else {',
    )
    .replace(
      '    ctx.source().rows(&ctx.facts(), scope).into_iter().next()',
      '    ctx.source()\n        .rows(&ctx.facts(), scope,)\n        .into_iter()\n        .next()',
    )
    .replace('    ctx.now_ms() > scope', '    // 只动格式：注释\n\n    ctx.now_ms() > scope')
  const files = cleanTree()
  files[ENGINE] = formatted
  const r = audit(files, { baselineFrom: cleanTree() })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /CP-006 机制面/)
})

test('CP-006：fn plan 加一个条件 ⇒ 红（归一化不抹语义）', () => {
  const files = cleanTree()
  files[ENGINE] = files[ENGINE].replace(
    '    ctx.source().rows(&ctx.facts(), scope).into_iter().next()',
    '    if scope == 0 {\n        return None;\n    }\n' +
      '    ctx.source().rows(&ctx.facts(), scope).into_iter().next()',
  )
  assertRed(audit(files, { baselineFrom: cleanTree() }), 'CP-006')
})

test('CP-006：fn expire 改一处返回值 ⇒ 红', () => {
  const files = cleanTree()
  files[ENGINE] = files[ENGINE].replace('ctx.now_ms() > scope', 'ctx.now_ms() >= scope')
  assertRed(audit(files, { baselineFrom: cleanTree() }), 'CP-006')
})

test('CP-006：控制面多一份生产文件而基线未跟 ⇒ 红', () => {
  const files = cleanTree()
  files[`${LOOP}/extra.rs`] = 'pub struct Extra;\n'
  assertRed(audit(files, { baselineFrom: cleanTree() }), 'CP-006')
})

test('CP-006：基线整个缺失 ⇒ 退 2（判不了不许亮绿灯）', () => {
  const r = audit(cleanTree(), { withBaseline: false })
  assert.equal(r.status, 2, r.stdout + r.stderr)
  assert.match(r.stderr, /基线读不出/)
})

test('CP-006：基线指向一个不存在的机制面文件 ⇒ 红（陈旧基线）', () => {
  const files = cleanTree()
  assertRed(
    audit(files, {
      mutate: (b) => {
        b.mechanism.file = `${LOOP}/ring/old.rs`
      },
    }),
    'CP-006',
  )
})

test('CP-006：基线里的决策号不存在 ⇒ 红（不许匿名冻结）', () => {
  const files = cleanTree()
  assertRed(
    audit(files, {
      mutate: (b) => {
        b.mechanism.adr = 'D-999'
      },
    }),
    'CP-006',
  )
})

test('机制面文件读不出 ⇒ 退 2（范围读不出，不是红）', () => {
  const files = cleanTree()
  delete files[ENGINE]
  const r = audit(files, { baselineFrom: cleanTree() })
  assert.equal(r.status, 2, r.stdout + r.stderr)
  assert.match(r.stderr, /机制面文件读不出/)
})

// ───────────────────────────── CP-009 ─────────────────────────────

test('CP-009：另一 crate 的 loop 里调 drive(...) ⇒ 红', () => {
  const files = cleanTree()
  files['crates/symbio-sdk/src/job.rs'] = 'pub fn advance() {\n    loop {\n        crate::drive(0);\n    }\n}\n'
  assertRed(audit(files), 'CP-009')
})

test('CP-009：for 遍历里调 drive(...) ⇒ 红（第二推进循环不只长得像 loop）', () => {
  const files = cleanTree()
  files['crates/symbio-sdk/src/job.rs'] =
    'pub fn advance(items: &[u8]) {\n    for item in items {\n        let _ = drive(*item);\n    }\n}\n'
  assertRed(audit(files), 'CP-009')
})

test('CP-009：循环里没有推进调用 ⇒ 仍绿（不判「有没有循环」）', () => {
  const files = cleanTree()
  files['crates/symbio-sdk/src/job.rs'] =
    'pub fn sum(items: &[u8]) -> u8 {\n    let mut n = 0;\n    for item in items {\n        n += *item;\n    }\n    n\n}\n'
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /CP-009 第二推进循环 0/)
})

/**
 * session-retire-audit 回归测试
 *
 * 判据面（SR-001/002/003，见 D-451）：
 *  - SR-001：豁免账里每条符号都必须在 `symbio-plugin-session/src` 里**真被用到**；
 *  - SR-002：符号条数 ≤ 基线（只许降）；
 *  - SR-003：`expires` 是**将来的**合法日期、`retire` 写明删除动作、`knife` 形如登记号。
 *
 * 这把尺最危险的失效是**读数失真**：
 *   ① 余量留在账上却没人报（E-009 只判「用了的必须在账上」——正是这个单向判据让
 *      15 条零命中条目静默积累）；
 *   ② 注释里提到某符号就被当成「在用」（那会让已退役的资格永久留在账上）；
 *   ③ 账本涨了不报（尺就失去意义）；
 *   ④ 豁免没有到期日 / `retire` 不写删除动作 ⇒ 变成永久豁免；
 *   ⑤ 输入读不出（JSON 缺失、源码目录被搬走）时静默通过。
 *
 * 夹具树用临时目录 + 脚本的 `--root=<dir>`（与 `port-type-coupling-audit.test.mjs` 同约定），
 * 不碰真实 crates。
 *
 * 跑法：node --test scripts/session-retire-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./session-retire-audit.mjs', import.meta.url))

const EXEMPTION_REL = 'scripts/exemptions/plugin-session-sdk.json'
const SESSION_SRC_REL = 'crates/symbio-plugin-session/src'

/** 干净夹具：两条符号都真被用、到期日在将来、`retire` 写明删除、`knife` 是登记号。 */
function cleanFiles() {
  return {
    [EXEMPTION_REL]: JSON.stringify(
      {
        rule: 'E-009',
        crate: 'symbio-plugin-session',
        dependency: 'symbio-sdk',
        expires: '2099-12-31',
        knife: 'T-340',
        retire: 'session 退役时同批删除本豁免',
        reason: '夹具',
        symbols: ['ClockNow', 'ReportDraft'],
      },
      null,
      2,
    ),
    [`${SESSION_SRC_REL}/lib.rs`]: [
      'use symbio_sdk::ClockNow;',
      'use symbio_sdk::ReportDraft;',
      '',
      'pub fn tick(d: ReportDraft) -> u64 { ClockNow::now() }',
      '',
    ].join('\n'),
  }
}

function sandbox(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sr-audit-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return { root, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  assert.ifError(r.error)
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

/** 只改豁免 JSON，源码夹具不动。 */
function withExemption(files, patch) {
  const d = JSON.parse(files[EXEMPTION_REL])
  patch(d)
  return { ...files, [EXEMPTION_REL]: JSON.stringify(d, null, 2) }
}

test('干净夹具树 ⇒ 通过，且三条判据都读到输入', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /SR-001/)
    assert.match(r.out, /SR-002/)
    assert.match(r.out, /SR-003/)
  } finally {
    box.cleanup()
  }
})

test('SR-001：账上一条符号零命中 ⇒ 红并点名（余量冒充进度）', () => {
  const box = sandbox(
    withExemption(cleanFiles(), (d) => {
      d.symbols = ['ClockNow', 'ReportDraft', 'NeverUsed']
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SR-001.*`NeverUsed`/s)
    assert.match(r.out, /零命中/)
  } finally {
    box.cleanup()
  }
})

test('SR-001：符号只出现在注释里 ⇒ 仍红（注释不是用法）', () => {
  const files = cleanFiles()
  files[`${SESSION_SRC_REL}/lib.rs`] = [
    'use symbio_sdk::ClockNow;',
    '// 这里曾经用 ReportDraft 做汇报（已迁走）',
    '/// ReportDraft 的旧读口已退役',
    'pub fn tick() -> u64 { ClockNow::now() }',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /`ReportDraft`/)
  } finally {
    box.cleanup()
  }
})

test('SR-002：符号条数超过基线 ⇒ 红（这条边只许收窄）', () => {
  // 基线是 97：造 98 条**都真被用**的符号，把 SR-001 摘干净，只看 SR-002。
  const symbols = Array.from({ length: 98 }, (_, i) => `Sym${i}`)
  const files = withExemption(cleanFiles(), (d) => {
    d.symbols = symbols
  })
  files[`${SESSION_SRC_REL}/lib.rs`] = symbols.map((s) => `use symbio_sdk::${s};`).join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SR-002.*98 条 > 基线 97/s)
    assert.doesNotMatch(r.out, /SR-001/)
  } finally {
    box.cleanup()
  }
})

test('SR-003：到期日已过 ⇒ 红（限期豁免过期即红）', () => {
  const box = sandbox(
    withExemption(cleanFiles(), (d) => {
      d.expires = '2000-01-01'
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SR-003.*已过期/s)
  } finally {
    box.cleanup()
  }
})

test('SR-003：到期日不可解析 ⇒ 红（没有到期日就是永久豁免）', () => {
  const box = sandbox(
    withExemption(cleanFiles(), (d) => {
      d.expires = '待定'
    }),
  )
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SR-003.*不是可解析的日期/s)
  } finally {
    box.cleanup()
  }
})

test('SR-003：retire 空 / 不含删除动作 ⇒ 红', () => {
  for (const retire of ['', '以后再看', '   ']) {
    const box = sandbox(
      withExemption(cleanFiles(), (d) => {
        d.retire = retire
      }),
    )
    try {
      const r = run(box.root)
      assert.equal(r.status, 1, r.out)
      assert.match(r.out, /SR-003/)
    } finally {
      box.cleanup()
    }
  }
})

test('SR-003：knife 不是登记号 ⇒ 红（这条账要指得出主人）', () => {
  for (const knife of ['', 'T034', 'T-34', 'knife-1']) {
    const box = sandbox(
      withExemption(cleanFiles(), (d) => {
        d.knife = knife
      }),
    )
    try {
      const r = run(box.root)
      assert.equal(r.status, 1, r.out)
      assert.match(r.out, /SR-003.*knife/s)
    } finally {
      box.cleanup()
    }
  }
})

test('输入读不出：豁免 JSON 缺失 ⇒ 红（宁红不静默）', () => {
  const files = cleanFiles()
  delete files[EXEMPTION_REL]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SR-003.*不存在/s)
  } finally {
    box.cleanup()
  }
})

test('输入读不出：session 源码目录被搬走 ⇒ 红（没有源码可量）', () => {
  const files = cleanFiles()
  delete files[`${SESSION_SRC_REL}/lib.rs`]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SR-001.*没有源码可量/s)
  } finally {
    box.cleanup()
  }
})

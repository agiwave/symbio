/**
 * port-surface-audit 回归测试
 *
 * 这条守卫的失效形态**全是恒真**：四格判据读不出输入（事件名字表读空、事实声明表
 * 读空、基线读空）时，最省事的实现是「没读到 ⇒ 没什么可判 ⇒ 通过」。而判据 1 是
 * PL-016 最硬的一条——它恒真就等于「加端口可以随便改引擎」这条最贵的走样没人管。
 *
 * 所以第一组用例拿**真仓当前状态**做非零对照（协议事实数 > 0、指纹非空），
 * 后面几组逐条注入违规，断言脚本**变红且只红该红的那条**。
 *
 * 跑法：node --test scripts/port-surface-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./port-surface-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const EVENT_REL = 'symbio/src/symbio_core/event/mod.rs'
const DECL_REL = 'symbio/src/symbio_core/decl/mod.rs'
const SOLVER_REL = 'symbio/src/symbio_core/decl/ring.rs'
const BASELINE_REL = 'scripts/baselines/port-surface.json'

/** 协议族常量（五个名字的定义，值取协议名的字面值） */
const EVENT_SRC = `pub const EVENT_REQUEST: &str = "request";
pub const EVENT_SENT: &str = "sent";
pub const EVENT_RESPONSE: &str = "response";
pub const EVENT_FAILED: &str = "failed";
pub const EVENT_RETRY: &str = "retry";
`

/** 声明层：两条带端口字段的事实 + 一条不带的 */
const DECL_SRC = `const BUILTIN_FACTS: &[FactDecl] = &[
    FactDecl {
        name: EVENT_REQUEST,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: &["port", "id", "attempt"],
    },
    FactDecl {
        name: EVENT_SENT,
        port: Some("port"),
        key: &["id", "attempt"],
        expand: &[],
        fields: &["port", "id", "attempt"],
    },
    FactDecl {
        name: "chat.assistant.final",
        port: None,
        key: &[],
        expand: &[],
        fields: &["text"],
    },
];
`

/** 求解器：判据 1 点名的那两个函数 */
const SOLVER_SRC = `pub(crate) fn owed(rows: &[Decl]) -> usize {
    rows.len()
}

pub(crate) fn drive(rows: &[Decl]) -> usize {
    owed(rows)
}
`

/**
 * 造一棵最小仓库。
 *
 * `facts` / `solver` / `event` 可覆盖，用来注入违规；基线**总是按夹具自己的求解器
 * 算出来**（基线是文件，随树走——这样「求解器变了」才是能被夹具表达的那件事）。
 */
function fixture({ event = EVENT_SRC, facts = DECL_SRC, solver = SOLVER_SRC, adr = 'ADR-115', reason = '夹具基线', skipBaseline = false, createAdr = true } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'port-surface-'))
  const write = (rel, body) => {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, body, 'utf8')
  }
  write(EVENT_REL, event)
  write(DECL_REL, facts)
  write(SOLVER_REL, solver)
  // 基线挂的 ADR 必须**能打开**（PTS-004）——夹具里给它一份空壳决策。
  if (createAdr) {
    const id = String(adr).match(/ADR-(\d+[a-z]?)/)?.[1]
    if (id) write(`docs/010-adr/${id}.md`, `---\nkind: adr\nid: ADR-${id}\ntitle: 夹具\n---\n`)
  }
  if (!skipBaseline) {
    const printed = spawnSync(process.execPath, [script, `--root=${root}`, '--print-baseline'], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.equal(printed.status, 0, printed.stdout + printed.stderr)
    const hash = JSON.parse(printed.stdout).hash
    write(BASELINE_REL, JSON.stringify({ solver: { file: SOLVER_REL, hash, adr, reason } }, null, 2))
  }
  return root
}

function run(root, args = []) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`, ...args], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30000,
  })
  assert.ifError(r.error)
  return r
}

function cleanup(root) {
  fs.rmSync(root, { recursive: true, force: true })
}

// ── ① 真仓当前状态：绿，且**数得出东西**（防恒真）─────────────────────

test('真仓当前状态通过，且四格判据都数得出非零输入', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 60000,
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  const facts = /带端口的事实 (\d+) 条/.exec(r.stdout)
  assert.ok(facts, '通过信息里没有「带端口的事实条数」⇒ 判据可能已恒真')
  assert.ok(Number(facts[1]) > 0, '真仓里带端口的事实数为 0 ⇒ 提取器与源码形状脱节')
  assert.match(r.stdout, /PTS-003.*sha256:/)
  assert.match(r.stdout, /PTS-004.*ADR-/)
})

// ── ② PTS-001：协议名闭集 ─────────────────────────────────────────────

test('PTS-001 带端口的事实名不在协议族里 ⇒ 红', () => {
  const root = fixture({
    facts: DECL_SRC.replace('name: EVENT_SENT,', 'name: "turn.sent",'),
  })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-001.*turn\.sent.*不在协议族/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-001 协议族常量缺一个 ⇒ 红（协议名必须单点定义）', () => {
  const root = fixture({ event: EVENT_SRC.replace('pub const EVENT_RETRY: &str = "retry";\n', '') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-001.*EVENT_RETRY/)
  } finally {
    cleanup(root)
  }
})

test('PTS-001 端口字段名不唯一 ⇒ 红', () => {
  const root = fixture({
    facts: DECL_SRC.replace('port: Some("port"),\n        key: &["id", "attempt"],\n        expand: &[],\n        fields: &["port", "id", "attempt"],\n    },\n    FactDecl {\n        name: "chat.assistant.final",',
      'port: Some("endpoint"),\n        key: &["id", "attempt"],\n        expand: &[],\n        fields: &["endpoint", "id", "attempt"],\n    },\n    FactDecl {\n        name: "chat.assistant.final",'),
  })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-001.*端口字段名必须唯一/)
  } finally {
    cleanup(root)
  }
})

test('豁免口：第 6 个协议名必须挂一条能打开的 ADR', () => {
  const root = fixture({
    facts: DECL_SRC.replace('name: EVENT_SENT,', 'name: "turn.sent",'),
  })
  try {
    // 先证明没有豁免时是红的
    assert.equal(run(root).status, 1)
    // 再在审计脚本自己的源码里写豁免（豁免必须挂得住决策）
    const self = fs.readFileSync(script, 'utf8')
    const patched = self.replace(
      'const WAIVER_RE =',
      '// port-surface-allow protocol turn.sent: ADR-115 —— 夹具：协议族扩一个成员\nconst WAIVER_RE =',
    )
    assert.notEqual(patched, self)
    fs.writeFileSync(script, patched, 'utf8')
    try {
      const r = run(root)
      assert.equal(r.status, 0, r.stdout + r.stderr)
    } finally {
      fs.writeFileSync(script, self, 'utf8')
    }
  } finally {
    cleanup(root)
  }
})

// ── ③ PTS-002：引擎对端口无感 ─────────────────────────────────────────

test('PTS-002 求解器里出现端口字段名 ⇒ 红', () => {
  const root = fixture({
    solver: SOLVER_SRC.replace(
      'pub(crate) fn drive(rows: &[Decl]) -> usize {\n    owed(rows)\n}',
      'pub(crate) fn drive(rows: &[Decl]) -> usize {\n    let _ = "port";\n    owed(rows)\n}',
    ),
  })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-002.*端口字段名/)
  } finally {
    cleanup(root)
  }
})

test('PTS-002 求解器里出现端口字段名（注释里不算）⇒ 绿', () => {
  const root = fixture({
    solver: `// 这里提到 port 只是散文，不是代码\n${SOLVER_SRC}`,
  })
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    cleanup(root)
  }
})

// ── ④ PTS-003 / PTS-004：基线 ─────────────────────────────────────────

test('PTS-003 求解器字节与基线不符 ⇒ 红（判据 1 的原话）', () => {
  const root = fixture()
  try {
    fs.writeFileSync(
      path.join(root, SOLVER_REL),
      SOLVER_SRC.replace('    rows.len()\n', '    rows.len() + 1\n'),
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-003.*字节与基线不符/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-003 基线缺失 ⇒ 红（没有基准就没有判据）', () => {
  const root = fixture({ skipBaseline: true })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-003.*基线/)
  } finally {
    cleanup(root)
  }
})

test('PTS-004 基线引用的 ADR 打不开 ⇒ 红', () => {
  const root = fixture({ adr: 'ADR-999', createAdr: false })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-004.*ADR-999/)
  } finally {
    cleanup(root)
  }
})

test('PTS-004 基线的理由为空 ⇒ 红', () => {
  const root = fixture({ reason: '   ' })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-004.*reason/)
  } finally {
    cleanup(root)
  }
})

// ── ⑤ 读不出输入时必须红，不得静默通过 ────────────────────────────────

test('事件名字表读不出 ⇒ 红（不得「没读到 ⇒ 没什么可判」）', () => {
  const root = fixture()
  try {
    fs.rmSync(path.join(root, EVENT_REL))
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-001.*读不出事件名字表/)
  } finally {
    cleanup(root)
  }
})

test('事实声明表读不出 ⇒ 红', () => {
  const root = fixture()
  try {
    fs.rmSync(path.join(root, DECL_REL))
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-001.*读不出事实声明表/)
  } finally {
    cleanup(root)
  }
})

test('求解器里没有 `fn owed` ⇒ 红（判据没有输入）', () => {
  const root = fixture()
  try {
    fs.writeFileSync(path.join(root, SOLVER_REL), 'pub(crate) fn nothing() {}\n', 'utf8')
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-00[23].*fn owed/)
  } finally {
    cleanup(root)
  }
})

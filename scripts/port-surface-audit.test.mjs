/**
 * port-surface-audit 回归测试
 *
 * 这条守卫的失效形态**全是恒真**：各格判据读不出输入（事件名字表读空、事实声明表
 * 读空、基线读空、写点扫空、契约表锚点漂走）时，最省事的实现是「没读到 ⇒ 没什么可判 ⇒ 通过」。而判据 1 是
 * DE-015 最硬的一条——它恒真就等于「加端口可以随便改引擎」这条最贵的走样没人管。
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
/** 端口表所在文件（脚本按「含 `PortDecl {` 的 .rs」扫取值名单，路径本身不钉死） */
const PORTS_REL = 'symbio/src/plugins/loop/loop_rows.rs'
/** 协议格写点所在文件（PTS-005 判**写方唯一**，不判这个路径——路径只是夹具的落点） */
const WRITER_REL = 'symbio/src/plugins/loop/port.rs'
/** 契约 7 的条文（PTS-006 的唯一输入） */
const PLAN_REL = 'docs/003-design/015-义务环.md'

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
 * 端口表（PTS-002 的**取值**名单来源）：脚本扫「含 `PortDecl {` 的 `.rs`」，
 * 所以夹具里也要有一份——没有它，取值一侧的判据在夹具树上永远没有输入，
 * 「注入一个 `if port == "tool"`」就红不起来。
 */
const PORTS_SRC = `pub(crate) static PORTS: &[PortDecl] = &[
    PortDecl { port: "turn", before: EVENT_SENT, lease_ms: 600_000, max_attempts: 1, concurrent: false },
    PortDecl { port: "tool", before: EVENT_SENT, lease_ms: 600_000, max_attempts: 1, concurrent: true },
];
`

/**
 * 协议格的写点（PTS-005 的输入）：`sent` 的名字是 `decl.before`（变量），
 * `response` 是常量——两种形态都要在夹具里出现，否则「按名字实参判」这条实现
 * 只测到了一半，把 `before` 那一支写坏也没人红。
 */
const WRITER_SRC = `pub(crate) fn begin<S>(store: &S, call: &Call, turn: u64) -> Result<(), AppendError> {
    store.append(Event::pending(
        call.attempt_key(decl.before),
        decl.before,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "system",
    ))
}

pub(crate) fn succeed<S>(store: &S, call: &Call, turn: u64) -> Result<(), AppendError> {
    store.append(Event::pending(
        call.attempt_key(EVENT_RESPONSE),
        EVENT_RESPONSE,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "system",
    ))
}
`

/**
 * 契约 7 的夹具条文：**合规的形状**（同时写「重试」与「未启用」）。
 *
 * 每份夹具都自带它——PTS-006 的输入不是这条判据的开关，而是仓库里本来就有的一份
 * 文档；夹具缺了它就等于「真仓的门在夹具上永远开着」，其余用过绿灯的用例也就不再
 * 证明它们各自的格。
 */
const PLAN_SRC = `### 2.6 执行契约

| # | 契约 | 不做的后果 |
|---|---|---|
| 6 | 无进展守卫 | 静默停摆 |
| 7 | 在途请求带租约；重试前提成立时落 retry 并重开下一笔（D-138），否则到顶即判死 | 永久在等 |
`

/**
 * 造一棵最小仓库。
 *
 * `facts` / `solver` / `event` / `writer` / `plan` 可覆盖，用来注入违规；基线**总是按夹具自己的求解器
 * 算出来**（基线是文件，随树走——这样「求解器变了」才是能被夹具表达的那件事）。
 */
function fixture({ event = EVENT_SRC, facts = DECL_SRC, solver = SOLVER_SRC, ports = PORTS_SRC, writer = WRITER_SRC, plan = PLAN_SRC, adr = 'D-115', reason = '夹具基线', skipBaseline = false, createAdr = true } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'port-surface-'))
  const write = (rel, body) => {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, body, 'utf8')
  }
  write(EVENT_REL, event)
  write(DECL_REL, facts)
  write(SOLVER_REL, solver)
  write(PORTS_REL, ports)
  write(WRITER_REL, writer)
  // `plan: null` = 夹具里没有这份文档（用来证明 PTS-006 读不到输入时判红）。
  if (plan !== null) write(PLAN_REL, plan)
  // 基线挂的决策必须**能打开**（PTS-004）——夹具在设计文档里写一条 `### D-NNN`。
  if (createAdr) {
    const id = String(adr).match(/D-(\d+[a-z]?)/)?.[1]
    if (id) write('docs/003-design/020-夹具决策.md', `# 夹具决策\n\n### D-${id} 夹具决策\n\n正文。\n`)
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

test('真仓当前状态通过，且各格判据都数得出非零输入', () => {
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
  assert.match(r.stdout, /PTS-004.*D-\d/)
  // PTS-005 也要数得出写点（扫空了就是判据静默变成「永远绿」）。
  const sites = /协议格写方唯一：(\S+)（命中 (\d+) 处构造）/.exec(r.stdout)
  assert.ok(sites, '通过信息里没有「协议格写方唯一」的实测名单与命中数 ⇒ PTS-005 可能没有输入')
  assert.ok(Number(sites[2]) > 0, `真仓里协议格写点命中数为 0（${sites[1]}）⇒ 提取器与源码形状脱节`)
  // 取值一侧也要数得出名单（否则「扫端口表」扫空了没人知道，判据静默变回只判字段名）。
  const values = /端口取值（名单 ([^）]*)）/.exec(r.stdout)
  assert.ok(values, '通过信息里没有「端口取值名单」⇒ 取值一侧可能没有输入')
  for (const v of ['turn', 'model', 'tool']) {
    assert.ok(values[1].includes(v), `端口取值名单里没有 ${v}：提取器与端口表脱节`)
  }
  // PTS-006 的输入也要真的存在（真仓里那份文档不是夹具自动补的）。
  assert.match(r.stdout, /PTS-006.*契约 7.*§2\.6/, '真仓里契约 7 的条文没被判到 ⇒ PTS-006 可能没有输入')
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

test('豁免口：第 6 个协议名必须挂一条能打开的决策', () => {
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
      '// port-surface-allow protocol turn.sent: D-115 —— 夹具：协议族扩一个成员\nconst WAIVER_RE =',
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

// 取值一侧（口径 ②）：`if port == "tool"` 不命名 `port` 字段，
// 但它是同一件事的另一种写法——引擎按端口值分支 ⇒ 加端口 = 改引擎。

test('PTS-002 求解器函数体里出现端口取值 ⇒ 红（不命名字段也渗进）', () => {
  const root = fixture({
    solver: SOLVER_SRC.replace(
      'pub(crate) fn drive(rows: &[Decl]) -> usize {\n    owed(rows)\n}',
      'pub(crate) fn drive(rows: &[Decl]) -> usize {\n    let _ = "tool";\n    owed(rows)\n}',
    ),
  })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-002.*端口取值 `tool`/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-002 端口取值只在注释里 ⇒ 绿（散文不是分支）', () => {
  const root = fixture({
    solver: '// 恢复时 tool 端口的那笔要重放——这是叙述，不是代码\n' + SOLVER_SRC,
  })
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    cleanup(root)
  }
})

test('PTS-002 端口取值在两个函数体之外 ⇒ 绿（自陈的边界：判据面是 owed / drive）', () => {
  // 边界要有名有姓：`ring.rs` 的信封字段名里就有 `"turn"`（轮号），它与端口
  // `turn` 的取值同名。把整份文件都算进来，这条真仓现状就成了假红。
  const root = fixture({
    solver: `const ENVELOPE: &[&str] = &["turn", "tool"];\n${SOLVER_SRC}`,
  })
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    cleanup(root)
  }
})

test('PTS-002 端口名单来自扫描而不是文件路径 ⇒ 换一张表也判得到', () => {
  // 脚本按「含 `PortDecl {` 的 .rs」收名单。把端口表搬到别的文件、原位置清空，
  // 注入的取值分支照样要红——否则「钉住某张表」会变成新的静默。
  const root = fixture({
    ports: '// 这张表搬走了\n',
    solver: SOLVER_SRC.replace(
      'pub(crate) fn drive(rows: &[Decl]) -> usize {\n    owed(rows)\n}',
      'pub(crate) fn drive(rows: &[Decl]) -> usize {\n    let _ = "tool";\n    owed(rows)\n}',
    ),
  })
  try {
    const moved = path.join(root, 'symbio/src/plugins/other/ports.rs')
    fs.mkdirSync(path.dirname(moved), { recursive: true })
    fs.writeFileSync(moved, PORTS_SRC, 'utf8')
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-002.*端口取值 `tool`/s)
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

test('PTS-004 基线引用的决策打不开 ⇒ 红', () => {
  const root = fixture({ adr: 'D-999', createAdr: false })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-004.*D-999/)
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

// ── ⑤ PTS-005：协议格写方唯一 ────────────────────────────────────────

test('PTS-005 第二个文件落协议格 ⇒ 红（引擎独占变成自觉）', () => {
  const root = fixture()
  try {
    fs.mkdirSync(path.join(root, 'symbio/src/plugins/other'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'symbio/src/plugins/other/extra.rs'),
      `pub(crate) fn cheat<S>(store: &S) {\n    let _ = store.append(Event::pending("x", EVENT_FAILED, Entity::Turn, Verb::Asserted, 1, "system"));\n}\n`,
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-005.*2 个写点文件/)
  } finally {
    cleanup(root)
  }
})

test('PTS-005 `before` 那一支也算写点（sent 的名字是变量，不是常量）', () => {
  const root = fixture()
  try {
    fs.mkdirSync(path.join(root, 'symbio/src/plugins/other'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'symbio/src/plugins/other/extra.rs'),
      `pub(crate) fn cheat<S>(store: &S, decl: &PortDecl) {\n    let _ = store.append(Event::pending("x", decl.before, Entity::Turn, Verb::Asserted, 1, "system"));\n}\n`,
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-005/)
  } finally {
    cleanup(root)
  }
})

test('PTS-005 读不出任何写点 ⇒ 红（协议格没人写是环不转，不是没什么可判）', () => {
  const root = fixture({ writer: 'pub(crate) fn nothing() {}\n' })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-005.*全仓找不到协议格的写点/)
  } finally {
    cleanup(root)
  }
})

test('PTS-005 行表里的 `source: EVENT_SENT` 不算写点（模式不是账）', () => {
  const root = fixture()
  try {
    fs.writeFileSync(
      path.join(root, PORTS_REL),
      PORTS_SRC +
        'const ROWS: &[Decl] = &[Decl { owes: Pattern::Fact { source: EVENT_SENT, ..P }, until: Pattern::Fact { source: EVENT_RESPONSE, ..P } }];\n',
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /PTS-005.*协议格写方唯一/)
  } finally {
    cleanup(root)
  }
})

// ── ⑦ PTS-006：契约 7 必须引 D-138 且不再写「未启用」 ──────────────────────

test('PTS-006 契约 7 仍写「未启用」⇒ 红（重试已启用，条文与机制分叉）', () => {
  const root = fixture({ plan: PLAN_SRC.replace('（D-138），否则到顶即判死', '（D-138），否则到顶即判死。重试通道未启用') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*未启用/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 契约 7 不引重试裁决 ⇒ 红（改判据不留决策）', () => {
  const root = fixture({ plan: PLAN_SRC.replace('（D-138）', '') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*重试进债键/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 契约表的第 7 行找不到 ⇒ 红（锚点漂了不是没什么可判）', () => {
  // 锚点漂的形状：第 7 行被改号（变成 8），判据找不到自己那一条。
  const root = fixture({ plan: PLAN_SRC.replace('| 7 |', '| 8 |') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*找到 0 条/)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 §2.6 小节不在了 ⇒ 红（判据没有输入）', () => {
  const root = fixture({ plan: PLAN_SRC.replace('### 2.6 执行契约', '### 别的标题') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*找不到「### 2\.6 /s)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 契约 7 文档读不出 ⇒ 红（不得「没读到 ⇒ 没什么可判」）', () => {
  const root = fixture({ plan: null })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*读不出/)
  } finally {
    cleanup(root)
  }
})

// ── ⑥ 读不出输入时必须红，不得静默通过 ────────────────────────────────
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

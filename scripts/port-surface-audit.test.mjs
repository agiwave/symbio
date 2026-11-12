/**
 * port-surface-audit 回归测试
 *
 * 这条守卫的失效形态**全是恒真**：各格判据读不出输入（事件名字表读空、事实声明表
 * 读空、基线读空、写点扫空、契约表锚点漂走）时，最省事的实现是「没读到 ⇒ 没什么可判 ⇒ 通过」。而判据 1 是
 * DE-051 最硬的一条——它恒真就等于「加端口可以随便改引擎」这条最贵的走样没人管。
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

const EVENT_REL = 'crates/symbio-kernel/src/event/mod.rs'
const DECL_REL = 'crates/symbio-kernel/src/decl/mod.rs'
const SOLVER_REL = 'crates/symbio-kernel/src/decl/ring.rs'
const BASELINE_REL = 'scripts/baselines/port-surface.json'
/** 端口表所在文件（脚本按「含 `PortDecl {` 的 .rs」扫取值名单，路径本身不钉死） */
const PORTS_REL = 'crates/symbio-plugin-loop/src/loop_rows.rs'
/** 协议格写点所在文件（PTS-005 判**写方唯一**，不判这个路径——路径只是夹具的落点） */
const WRITER_REL = 'crates/symbio-plugin-loop/src/port.rs'
/** 重试条文（PTS-006 的唯一输入） */
const PLAN_REL = 'docs/003-design/050-义务环事实词表与债务表.md'

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

/** 求解器：判据 1 冻结的那一个求解函数 */
const SOLVER_SRC = `pub(crate) fn owed(rows: &[Decl]) -> usize {
    rows.len()
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
    PortDecl { port: "model", before: EVENT_SENT, lease_ms: 600_000, max_attempts: 1, concurrent: false },
];
`

/**
 * PTS-007 的扫描面：三个读侧树各放一份**干净**样例。
 *
 * 这份样例本身就是一个判据：`stamp.role == Some("reader")` 必须**不红**
 * （`reader` 不在端口取值名单里）——否则「只认章」这条会被自己的守卫误伤。
 * 端口常量只作声明出现，比较留空。
 */
/** PTS-007 扫描的三个读侧树（与脚本里的 `PTS7_ROOTS` 同一份口径） */
const READ_SIDE_ROOTS = [
  'crates/symbio-sdk/src',
  'crates/symbio-plugin-api/src',
  'crates/symbio-plugin-session/src',
]

const READ_SIDE_SRC = `pub(crate) const PORT_TOOL: &str = "tool";

pub(crate) fn is_reader(payload: &serde_json::Value) -> bool {
    let role = payload.get("stamp").and_then(|s| s.get("role")).and_then(serde_json::Value::as_str);
    role == Some("reader")
}
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
 * 重试条文的夹具形状：**合规**（写出 `retry(attempt+1)`、不写「未启用」）。
 *
 * 每份夹具都自带它——PTS-006 的输入不是这条判据的开关，而是仓库里本来就有的一份
 * 文档；夹具缺了它就等于「真仓的门在夹具上永远开着」，其余用过绿灯的用例也就不再
 * 证明它们各自的格。
 */
const PLAN_SRC = `### 3.3 重放收敛

- 有 \`sent\`、无收束且租约已过 ⇒ 落 \`retry(attempt+1)\` 再派发；同一 \`(kind, key)\` 重复投递被账本吸收。
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
  // PTS-007 的扫描面：三个读侧树都要有文件，否则「判据没有输入」那一格会让
  // 每一条夹具用例一起红（那是夹具缺件，不是被测判据的红）。
  for (const rel of READ_SIDE_ROOTS) write(`${rel}/sample.rs`, READ_SIDE_SRC)
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
  assert.match(r.stdout, /PTS-006.*050-义务环事实词表与债务表\.md §3\.3/, '真仓里契约 7 的条文没被判到 ⇒ PTS-006 可能没有输入')
  // PTS-007 也要数得出扫描面（扫空了就变成「永远绿」）。
  const readSide = /PTS-007.*读侧不认端口名：(\d+) 个文件/.exec(r.stdout)
  assert.ok(readSide, '通过信息里没有「读侧不认端口名」的扫描文件数 ⇒ PTS-007 可能没有输入')
  assert.ok(Number(readSide[1]) > 0, '读侧扫描面为 0 ⇒ 三个读侧树读不出')
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
      '    rows.len()\n',
      '    let _ = "port";\n    rows.len()\n',
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
      '    rows.len()\n',
      '    let _ = "tool";\n    rows.len()\n',
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

test('PTS-002 端口取值在求解函数体之外 ⇒ 绿（自陈的边界：判据面是 owed）', () => {
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
      '    rows.len()\n',
      '    let _ = "tool";\n    rows.len()\n',
    ),
  })
  try {
    const moved = path.join(root, 'crates/symbio-plugin-other/src/ports.rs')
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
    fs.mkdirSync(path.join(root, 'crates/symbio-plugin-other/src'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'crates/symbio-plugin-other/src/extra.rs'),
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
    fs.mkdirSync(path.join(root, 'crates/symbio-plugin-other/src'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'crates/symbio-plugin-other/src/extra.rs'),
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

// ── ⑦ PTS-006：§3.3 必须写 retry(attempt+1) 且不再写「未启用」 ──────────────

test('PTS-006 §3.3 仍写「未启用」⇒ 红（重试已启用，条文与机制分叉）', () => {
  const root = fixture({ plan: `${PLAN_SRC}\n重试通道未启用。\n` })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*未启用/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 §3.3 不写 retry(attempt+1) ⇒ 红（条文不把重试写出来）', () => {
  const root = fixture({ plan: PLAN_SRC.replace('retry(attempt+1)', 'retry') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*retry\(attempt\+1\)/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 §3.3 小节不在了 ⇒ 红（判据没有输入）', () => {
  const root = fixture({ plan: PLAN_SRC.replace('### 3.3 重放收敛', '### 别的标题') })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*找不到「### 3\.3 /s)
  } finally {
    cleanup(root)
  }
})

test('PTS-006 重试条文文档读不出 ⇒ 红（不得「没读到 ⇒ 没什么可判」）', () => {
  const root = fixture({ plan: null })
  try {
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-006.*读不出/)
  } finally {
    cleanup(root)
  }
})

// ── ⑧ PTS-007：读侧不认端口名 ─────────────────────────────────────────

test('PTS-007 读侧出现 `if port == "model"` ⇒ 红（退化用例）', () => {
  const root = fixture()
  try {
    fs.mkdirSync(path.join(root, 'crates/symbio-plugin-session/src/answer'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'crates/symbio-plugin-session/src/answer/bad.rs'),
      'pub(crate) fn pick(port: &str) -> bool {\n    if port == "model" { true } else { false }\n}\n',
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-007.*bad\.rs:2.*`model`/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-007 读侧用端口常量比较（`port == PORT_TOOL`）⇒ 红', () => {
  const root = fixture()
  try {
    fs.mkdirSync(path.join(root, 'crates/symbio-sdk/src/projection'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'crates/symbio-sdk/src/projection/bad.rs'),
      'pub(crate) fn pick(port: &str) -> bool {\n    port == PORT_TOOL\n}\n',
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-007.*bad\.rs:2.*`tool`/s)
  } finally {
    cleanup(root)
  }
})

test('PTS-007 盖章比较（`stamp.role == Some("reader")`）不判 ⇒ 绿', () => {
  // 夹具的 `READ_SIDE_SRC` 就是这一条；单独再写一遍，免得它被别处的注入掩盖。
  const root = fixture()
  try {
    fs.mkdirSync(path.join(root, 'crates/symbio-sdk/src/projection'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'crates/symbio-sdk/src/projection/stamp.rs'),
      'pub(crate) fn is_reader(payload: &serde_json::Value) -> bool {\n    payload.get("stamp").and_then(|s| s.get("role")).and_then(serde_json::Value::as_str) == Some("reader")\n}\n',
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    cleanup(root)
  }
})

test('PTS-007 测试文件里的端口比较不判 ⇒ 绿（夹具不是读侧分支）', () => {
  const root = fixture()
  try {
    fs.mkdirSync(path.join(root, 'crates/symbio-plugin-session/src'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'crates/symbio-plugin-session/src/sample.test.rs'),
      'fn response(port: &str) {\n    if port == "model" {}\n}\n',
      'utf8',
    )
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    cleanup(root)
  }
})

test('PTS-007 三个读侧树读不出 ⇒ 红（不得「没什么可判 ⇒ 通过」）', () => {
  const root = fixture()
  try {
    for (const rel of READ_SIDE_ROOTS) {
      fs.rmSync(path.join(root, rel), { recursive: true, force: true })
    }
    const r = run(root)
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stderr, /PTS-007.*读侧判据没有输入/s)
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

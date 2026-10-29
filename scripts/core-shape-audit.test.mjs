/**
 * core-shape-audit 回归测试 —— 证明它**会红**，且**不误报**
 *
 * 这个守卫的失效形态有两头，两头都要钉住：
 * ① **漏数**（提取写宽了）：形状变宽而绿灯 ⇒ 它守的那条纪律（08 §0 纪律 3）回到「没人核对」；
 * ② **多数**（提取写严了）：把默认体里的局部 `fn`、测试里的 trait 都算成成员 ⇒ 一片误报，
 *    最后被人用放宽注释喂到失效——本仓为这种结局写过一整节（`core-export-audit` 的 W1 注）。
 *
 * 覆盖：
 *   1. 形状 == 基线 ⇒ 退出 0；
 *   2. CS-001 三种变宽各自变红：多一个 `pub trait` / 既有 trait 多一个方法 / 必需成员长出身；
 *   3. 反向：默认体里的局部 `fn` 不算成员；`#[cfg(test)]` 模块与 `*.test.rs` 不算形状；
 *   4. CS-002：放宽注释**没有 ADR 编号** / ADR **打不开** / 格的键名不认识 ⇒ 各自红；
 *      ADR 能打开 ⇒ 不红（判据不拦「有决策的放宽」，那是论证的事）；
 *   5. 范围读不出（没有 core）⇒ 红，不是静默通过；
 *   6. env 覆盖少给一格 ⇒ 红（那一格的判据不能凭空消失）。
 *
 * 用法：node --test scripts/core-shape-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./core-shape-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，见 no-direct-call-audit.test.mjs） */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

const CORE_MOD = 'crates/symbio-core/src/lib.rs'
const ALPHA = 'crates/symbio-core/src/alpha/mod.rs'
const KERNEL = 'crates/symbio-core/src/kernel.rs'
/** 一个成员、无默认体的最小夹具 */
const ONE = 'pub trait Store {\n    fn one(&self) -> u32;\n}\n'

/**
 * 建夹具 → 跑审计 → 返回 spawnSync 结果。
 *
 * `baseline` 就是 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>`：夹具只有几个 trait，
 * 不显式给基线就测不到棘轮（真仓的值是 20:72:24）。
 * `data` 是 `CORE_DATA_BASELINE=<structFields>:<enumVariants>`（CS-009），默认 `0:0`——
 * 绝大多数夹具里没有公共数据面，给 `0:0` 让提示噪音不出现在无关用例里。
 * `turnState` 是 `TURN_STATE_BASELINE=<字段数>`（CS-010），默认不给——夹具仓库里没有
 * 草稿文件时 CS-010 不适用，与真仓的默认基线无关。
 */
function audit(files, { baseline = '1:1:0', vocab, data = '0:0', turnState } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-shape-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const env = { ...process.env, NO_COLOR: '1', CORE_SHAPE_BASELINE: baseline, CORE_DATA_BASELINE: data }
    if (vocab !== undefined) env.VOCAB_BASELINE = vocab
    if (turnState !== undefined) env.TURN_STATE_BASELINE = turnState
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

const core = (traitSrc, extra = {}) => ({ [CORE_MOD]: 'mod alpha;\n', [ALPHA]: traitSrc, ...extra })

// ==================== CS-001 棘轮 ====================

test('形状等于基线 ⇒ 通过', () => {
  const r = audit(core(ONE), { baseline: '1:1:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('多一个 pub trait（新增接缝）⇒ CS-001 变红', () => {
  const r = audit(
    core(ONE, { 'crates/symbio-core/src/beta/mod.rs': 'pub trait Registry {\n    fn b(&self);\n}\n' }),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 1, '接缝变多必须红')
  assert.match(r.stderr, /CS-001/)
  assert.match(r.stderr, /接缝数.*实测 2 > 基线 1/)
})

test('既有 trait 多一个方法（契约变宽）⇒ CS-001 变红，且只有 CS-001 红', () => {
  const r = audit(
    core('pub trait Store {\n    fn one(&self) -> u32;\n    fn two(&self) -> u32;\n}\n'),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 1, '成员变多必须红')
  assert.match(r.stderr, /成员总数.*实测 2 > 基线 1/)
  assert.doesNotMatch(r.stderr, /CS-002/, '没有放宽痕迹时不该牵扯 CS-002')
})

test('必需成员长出默认体（接口上长实现）⇒ 只有 defaults 格变红', () => {
  // methods 不变（1 个成员），变的只有「它带不带体」——这一格必须单独看得见，
  // 否则「把实现塞进接口」就藏在成员数里，正是 C2 那批要钻的形状。
  const r = audit(
    core('pub trait Store {\n    fn one(&self) -> u32 {\n        1\n    }\n}\n'),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 1, '默认体变多必须红')
  assert.match(r.stderr, /带默认体的成员数.*实测 1 > 基线 0/)
  assert.doesNotMatch(r.stderr, /成员总数/)
})

test('反向：默认体里的局部 `fn` 不算成员（多数会误报）', () => {
  const r = audit(
    core(
      'pub trait Store {\n    fn one(&self) -> u32 {\n        fn helper() -> u32 { 7 }\n        helper()\n    }\n}\n',
    ),
    { baseline: '1:1:1' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('反向：`async fn` / 多行签名的成员照常数到（`->` 的尖括号不能拖歪计数）', () => {
  const r = audit(
    core(
      'pub trait Store {\n    async fn one(&self) -> Result<u32, Err>;\n    async fn two(&self) -> Result<u32, Err> {\n        self.one().await\n    }\n}\n',
    ),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 1, '两个成员 > 基线 1')
  assert.match(r.stderr, /成员总数.*实测 2 > 基线 1/)
  assert.match(r.stderr, /带默认体的成员数.*实测 1 > 基线 0/)
})

test('反向：测试里的 trait 不算形状（`#[cfg(test)]` 模块与 `*.test.rs`）', () => {
  const r = audit(
    core(
      ONE +
        '\n#[cfg(test)]\nmod tests {\n    pub trait Fake {\n        fn a(&self);\n        fn b(&self);\n    }\n}\n',
      { 'crates/symbio-core/src/alpha/inline.test.rs': 'pub trait Ghost {\n    fn x(&self);\n    fn y(&self);\n}\n' },
    ),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

// ==================== 内核文件集（A2：判据只对 kernel 生效） ====================

test('内核文件集：标记之外的同名 trait 不占形状（CS-001 只量内核）', () => {
  const r = audit(
    {
      [CORE_MOD]: 'mod alpha;\nmod beta;\nmod kernel;\n',
      [ALPHA]: ONE,
      'crates/symbio-core/src/beta/mod.rs': 'pub trait Registry {\n    fn b(&self);\n}\n',
      [KERNEL]: '// core-kernel-files: alpha/mod.rs\n',
    },
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('内核文件集指向不存在的文件 ⇒ 红（边界失焦不许静默）', () => {
  const r = audit(core(ONE, { [KERNEL]: '// core-kernel-files: alpha/mod.rs ghost/mod.rs\n' }), {
    baseline: '1:1:0',
  })
  assert.equal(r.status, 1, '文件集要可解析，否则判据在一个缩小了的范围上恒绿')
  assert.match(r.stderr, /CS-004/)
  assert.match(r.stderr, /不存在的文件/)
})

test('CS-004 只量内核文件集：域文件里的插件 id 字面量不判红（报告账）', () => {
  // 宿主侧插件域（`symbio-core/src/plugin/mod.rs`）存在 ⇒ 五格齐全检查生效，
  // 夹具把五个内核 trait 都造出来。
  const five =
    ONE +
    'pub trait Capability {\n    fn c(&self);\n}\n' +
    'pub trait ModelProvider {\n    fn m(&self);\n}\n' +
    'pub trait VdfsProvider {\n    fn v(&self);\n}\n' +
    'pub trait Registry {\n    fn r(&self);\n}\n'
  const files = {
    [CORE_MOD]: 'mod alpha;\nmod gamma;\nmod kernel;\n',
    [ALPHA]: five,
    'crates/symbio-core/src/gamma/mod.rs': 'pub const G: &str = "session";\n',
    [KERNEL]: '// core-kernel-files: alpha/mod.rs\n',
    'crates/symbio-core/src/plugin/mod.rs': '',
    'crates/symbio-plugin-session/src/.keep': '',
    // 这个夹具按真仓形状造（有宿主侧插件域）⇒ CS-010 的草稿文件必须同样在场。
    [TURN_STATE]: draft(['a']),
  }
  const ok = audit(files, { baseline: '5:5:0', turnState: '1' })
  assert.equal(ok.status, 0, ok.stderr + ok.stdout)

  const bad = audit(
    { ...files, [ALPHA]: five + 'pub const A: &str = "session";\n' },
    { baseline: '5:5:0', turnState: '1' },
  )
  assert.equal(bad.status, 1, '同一字面量落进内核文件就必须红')
  assert.match(bad.stderr, /CS-004/)
  assert.match(bad.stderr, /回流/)
})

// ==================== CS-002 放宽必须有能打开的决策 ====================

test('放宽注释里没有 ADR 编号 ⇒ CS-002 变红', () => {
  const r = audit(
    core(ONE, { 'scripts/core-shape-audit.mjs': '// core-shape-allow methods: 批次需要，先放宽一点\n' }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 1, '没有决策的放宽必须红')
  assert.match(r.stderr, /CS-002/)
  assert.match(r.stderr, /没有 ADR 编号/)
})

test('放宽注释引用了不存在的决策 ⇒ CS-002 变红', () => {
  const r = audit(
    core(ONE, { 'scripts/core-shape-audit.mjs': '// core-shape-allow methods: D-077 扩到 2 个成员的理由\n' }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 1, '指向打不开的决策 = 空引用')
  assert.match(r.stderr, /D-077.*条目/)
})

test('放宽注释挂着能打开的决策 ⇒ 通过（棘轮不判论证内容，只保证决策存在）', () => {
  const r = audit(
    core(ONE, {
      'scripts/core-shape-audit.mjs': '// core-shape-allow methods: D-077 扩到 2 个成员的理由\n',
      'crates/symbio-plugin-heartbeat/README.md': '# 心跳插件\n\n### D-077 夹具决策\n\n理由。\n',
    }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('放宽一个不存在的格 ⇒ CS-002 变红（拼错的键名不能当豁免）', () => {
  const r = audit(
    core(ONE, {
      'scripts/core-shape-audit.mjs': '// core-shape-allow methodz: D-077 打错了键名\n',
      'crates/symbio-plugin-heartbeat/README.md': '# 心跳插件\n\n### D-077 夹具决策\n\n理由。\n',
    }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 1)
  assert.match(r.stderr, /不是基线的格/)
})

// ==================== CS-009 数据面棘轮 ====================

/** 一个 2 字段 struct + 一个 2 变体 enum 的最小数据面夹具（合计 2:2）。 */
const DATA_TWO = 'pub struct Point {\n    pub x: u8,\n    pub y: u8,\n}\n\npub enum Axis {\n    X,\n    Y,\n}\n'

test('数据面等于基线 ⇒ 通过', () => {
  const r = audit(core(ONE, { 'crates/symbio-core/src/beta/mod.rs': DATA_TWO }), { data: '2:2' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('多一个枚举变体（封闭取值面变宽）⇒ CS-009 变红', () => {
  const r = audit(
    core(ONE, {
      'crates/symbio-core/src/beta/mod.rs': DATA_TWO.replace('    Y,\n', '    Y,\n    Z,\n'),
    }),
    { data: '2:2' },
  )
  assert.equal(r.status, 1, '加一个变体必须红')
  assert.match(r.stderr, /CS-009/)
  assert.match(r.stderr, /变体总数.*实测 3 > 基线 2/)
  assert.doesNotMatch(r.stderr, /字段总数/, '字段没变，不该牵连另一格')
})

test('多一个结构体字段（公共数据面变宽）⇒ 只有 structFields 格变红', () => {
  const r = audit(
    core(ONE, {
      'crates/symbio-core/src/beta/mod.rs': DATA_TWO.replace('    pub y: u8,\n', '    pub y: u8,\n    pub z: u8,\n'),
    }),
    { data: '2:2' },
  )
  assert.equal(r.status, 1, '加一个字段必须红')
  assert.match(r.stderr, /字段总数.*实测 3 > 基线 2/)
  assert.doesNotMatch(r.stderr, /变体总数/, '变体没变，不该牵连另一格')
})

test('反向：泛型实参 / 函数指针 / 属性里的逗号不算字段分隔', () => {
  // 若按「数逗号」实现，这个 struct 会被读成 5 个字段而不是 3 个——棘轮在方向上
  // 只会多数，但多数会把余量白送出去，等于放宽。
  const src =
    'pub struct Wide {\n    pub a: BTreeMap<u8, u8>,\n    pub b: fn(u8, u8) -> u8,\n    pub c: [u8; 2],\n}\n'
  const r = audit(core(ONE, { 'crates/symbio-core/src/beta/mod.rs': src }), { data: '3:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('反向：tuple struct 的位置字段计入，`pub(crate)` 与测试模块不计', () => {
  const src =
    'pub struct Pair(u8, u8);\n\npub(crate) struct Hidden {\n    a: u8,\n    b: u8,\n}\n\n#[cfg(test)]\nmod t {\n    pub struct Fake {\n        a: u8,\n    }\n}\n'
  const r = audit(core(ONE, { 'crates/symbio-core/src/beta/mod.rs': src }), { data: '2:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('CS-009 的放宽同样要挂能打开的决策（`core-shape-allow enumVariants`）', () => {
  const bumped = DATA_TWO.replace('    Y,\n', '    Y,\n    Z,\n')
  const withoutDecision = audit(core(ONE, { 'crates/symbio-core/src/beta/mod.rs': bumped }), {
    data: '2:3',
  })
  // 没有放宽注释时不该红：数字与基线一致。放宽注释只在「改数字」时才需要解释。
  assert.equal(withoutDecision.status, 0, withoutDecision.stderr + withoutDecision.stdout)

  const waived = audit(
    core(ONE, {
      'crates/symbio-core/src/beta/mod.rs': bumped,
      'scripts/core-shape-audit.mjs': '// core-shape-allow enumVariants: D-209 领域扩展位的理由\n',
      'docs/003-design/025-能力包与扩展面.md': '# DE-025\n\n### D-209 领域轴用一个扩展位\n\n理由。\n',
    }),
    { data: '2:3' },
  )
  assert.equal(waived.status, 0, waived.stderr + waived.stdout)
  assert.match(waived.stdout, /enumVariants←D-209/)

  const dangling = audit(
    core(ONE, {
      'crates/symbio-core/src/beta/mod.rs': bumped,
      'scripts/core-shape-audit.mjs': '// core-shape-allow enumVariants: D-999 指向不存在的决策\n',
    }),
    { data: '2:3' },
  )
  assert.equal(dangling.status, 1, '指向打不开的决策 = 空引用')
  assert.match(dangling.stderr, /D-999.*条目/)
})

// ==================== 判据自己不能哑火 ====================

test('读不出 core 范围 ⇒ 退出 1（宁红不静默）', () => {
  const r = audit({ 'README.md': '没有 symbio_core 的仓库\n' }, { baseline: '0:0:0' })
  assert.equal(r.status, 1, '没有输入时不许绿灯')
  assert.match(r.stderr, /读不出 core 范围/)
})

test('env 覆盖少给一格 ⇒ 红（那一格的判据不能凭空消失）', () => {
  const r = audit(core(ONE), { baseline: '1:1' })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /不是数字/)
})

// ==================== CS-005 事件词汇棘轮 ====================

const EVENT_MOD = 'crates/symbio-kernel/src/event/mod.rs'
const GRID = 'crates/symbio-kernel/src/event/grid.rs'

/**
 * 内核事件名表的最小夹具：协议五格照真仓写法，业务行由参数给。
 *
 * `cells` 只喂给**回潮用例**——`grid.rs` 已随 D-203 退役，真仓里它不该存在。
 * 保留这条通路是为了钉住「加回来会红」，不是为了描述现状。
 */
function vocab(sources = ['a.one'], cells = ['EVENT_A']) {
  return {
    [EVENT_MOD]: [
      'pub const EVENT_REQUEST: &str = "request";',
      'pub const EVENT_SENT: &str = "sent";',
      'pub const EVENT_RESPONSE: &str = "response";',
      'pub const EVENT_FAILED: &str = "failed";',
      'pub const EVENT_RETRY: &str = "retry";',
      'pub const EVENT_TRIGGER_META_KEY: &str = "trigger";',
      ...sources.map((v, i) => `pub const EVENT_${String.fromCharCode(65 + i)}: &str = "${v}";`),
    ].join('\n'),
    [GRID]: [
      'const CELLS: &[(&str, Cell)] = &[',
      '    (EVENT_REQUEST, (Entity::Turn, Verb::Asserted)),',
      '    (EVENT_SENT, (Entity::Turn, Verb::Asserted)),',
      '    (EVENT_RESPONSE, (Entity::Turn, Verb::Asserted)),',
      '    (EVENT_FAILED, (Entity::Turn, Verb::Asserted)),',
      '    (EVENT_RETRY, (Entity::Turn, Verb::Asserted)),',
      ...cells.map((n) => `    (${n}, (Entity::Turn, Verb::Asserted)),`),
      '];',
    ].join('\n'),
  }
}

test('CS-005：词汇等于基线 ⇒ 通过，且不把协议五格算进业务行', () => {
  const r = audit(core(ONE, vocab()), { baseline: '1:1:0', vocab: '1:1' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-005.*kernelEvents 1\/1.*cellsRows 1\/1/)
})

test('CS-005：新增一个业务事件常量 ⇒ 红（新事件该走 EventDecl）', () => {
  const r = audit(core(ONE, vocab(['a.one', 'b.two'])), { baseline: '1:1:0', vocab: '1:1' })
  assert.equal(r.status, 1, '内核业务事件常量变多必须红')
  assert.match(r.stderr, /CS-005/)
  assert.match(r.stderr, /kernelEvents 实测 2 > 冻结 1/)
})

test('CS-005：CELLS 新增一条业务行 ⇒ 红（写闸不再需要改这张表）', () => {
  const r = audit(core(ONE, vocab(['a.one', 'b.two'], ['EVENT_A', 'EVENT_B'])), {
    baseline: '1:1:0',
    vocab: '1:1',
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /cellsRows 实测 2 > 冻结 1/)
})

test('CS-005：grid.rs 已退役 ⇒ cellsRows 0 是目标态，不判「判据输入缺一半」', () => {
  const onlyEvents = { [EVENT_MOD]: vocab()['crates/symbio-kernel/src/event/mod.rs'] }
  const r = audit(core(ONE, onlyEvents), { baseline: '1:1:0', vocab: '1:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /cellsRows 0\/0/)
})

test('CS-005：退役的 grid.rs 被加回内核 ⇒ 红（名字重新写进语法就是回潮）', () => {
  const r = audit(core(ONE, vocab(['a.one'], ['EVENT_A'])), { baseline: '1:1:0', vocab: '1:0' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /cellsRows 实测 1 > 冻结 0/)
})

test('CS-005：收到基线下方 ⇒ 通过并提示同步数字', () => {
  const r = audit(core(ONE, vocab([], [])), { baseline: '1:1:0', vocab: '1:1' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /kernelEvents 0<1/)
})

// ==================== CS-008 core 生产代码零事件名字面量 ====================

const FIXTURES = 'crates/symbio-core/src/test_fixtures.rs'
const CORE_DELTA = 'crates/symbio-core/src/delta/mod.rs'

/**
 * CS-008 的输入：内核事件表（含 `Entity` 闭集）+ 一条业务名，**不带** `grid.rs`
 * （那张表已随 D-203 退役，夹具不该替它续命）。
 */
function vocabWithEntity(variants = ['Turn', 'Chat', 'Artifact']) {
  const files = vocab()
  delete files[GRID]
  files[EVENT_MOD] +=
    '\n' +
    ['pub enum Entity {', ...variants.map((v) => `    ${v},`), '}'].join('\n') +
    '\n'
  return files
}

test('CS-008：core 生产代码写死事件名 ⇒ 红（表归零不等于名字回不来）', () => {
  const src = 'pub fn is_final(k: &str) -> bool {\n    k == "chat.assistant.final"\n}\n'
  const r = audit(core(ONE, { ...vocabWithEntity(), [CORE_DELTA]: src }), {
    baseline: '1:1:0',
    vocab: '1:0',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /CS-008/)
  assert.match(r.stderr, /chat\.assistant\.final/)
})

test('CS-008：`#[cfg(test)] mod x;` 声明的夹具文件不算生产代码 ⇒ 不红', () => {
  const src = 'pub(crate) const EVENT_FINAL: &str = "chat.assistant.final";\n'
  const lib = 'mod alpha;\n#[cfg(test)]\nmod test_fixtures;\n'
  const r = audit(core(ONE, { ...vocabWithEntity(), [CORE_MOD]: lib, [FIXTURES]: src }), {
    baseline: '1:1:0',
    vocab: '1:0',
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-008.*剥掉 1 个/)
})

test('CS-008：首段不在 `Entity` 闭集里的点分串 ⇒ 不红（闸靠闭集收窄，不靠豁免）', () => {
  const src = 'pub fn f() -> (&\'static str, &\'static str) {\n    ("text.plain", "application/json")\n}\n'
  const r = audit(core(ONE, { ...vocabWithEntity(), [CORE_BETA]: src }), {
    baseline: '1:1:0',
    vocab: '1:0',
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

// ==================== CS-006 内核业务事件名的消费方分布 ====================

const ALPHA_CRATE = 'crates/symbio-plugin-alpha/src/lib.rs'
const BETA_CRATE = 'crates/symbio-plugin-beta/src/lib.rs'
const CORE_BETA = 'crates/symbio-core/src/beta/mod.rs'

test('CS-006：只有一个插件消费、core 已不读 ⇒ 红（这条名字该随 owner 的 EventDecl 走）', () => {
  const r = audit(
    core(ONE, { ...vocab(), [ALPHA_CRATE]: 'pub fn k() { let _ = EVENT_A; }\n' }),
    { baseline: '1:1:0', vocab: '1:1' },
  )
  assert.equal(r.status, 1, r.stderr + r.stdout)
  assert.match(r.stderr, /CS-006/)
  assert.match(r.stderr, /「a\.one」的唯一生产消费方是 symbio-plugin-alpha/)
})

test('CS-006：两个 crate 同时消费 ⇒ 绿（名字不能被任一插件独占，E-009）', () => {
  const r = audit(
    core(ONE, {
      ...vocab(),
      [ALPHA_CRATE]: 'pub fn k() { EVENT_A }\n',
      [BETA_CRATE]: 'pub fn j() { "a.one" }\n',
    }),
    { baseline: '1:1:0', vocab: '1:1' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-006.*1 条都不在「单一消费方且 core 不读」这一形态（≥2 crate 消费 1/)
})

test('CS-006：core 生产代码读它 ⇒ 不判红，但账记在「待迁存量」上（迁的移动作在读者那一侧，DE-024 D-203）', () => {
  const r = audit(
    core(ONE, {
      ...vocab(),
      [ALPHA_CRATE]: 'pub fn k() { EVENT_A }\n',
      [CORE_BETA]: 'pub const K: &str = "a.one";\n',
    }),
    { baseline: '1:1:0', vocab: '1:1' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-006.*core 读者 1/)
})

test('CS-006：core 里 `pub use` 转手一个名字不算读者 ⇒ 红（出口是账本，不是归属证据）', () => {
  const r = audit(
    core(ONE, {
      ...vocab(),
      [CORE_MOD]: 'mod alpha;\npub use crate::event::EVENT_A;\n',
      [ALPHA_CRATE]: 'pub use symbio_kernel::EVENT_A;\n',
    }),
    { baseline: '1:1:0', vocab: '1:1' },
  )
  assert.equal(r.status, 1, '转手不算读者，否则单消费方的名字永远迁不干净')
  assert.match(r.stderr, /「a\.one」的唯一生产消费方是 symbio-plugin-alpha/)
})

test('CS-006：谁都不读的业务名 ⇒ 红（内核表里躺着没人读的名字）', () => {
  const r = audit(
    core(ONE, { ...vocab(), [ALPHA_CRATE]: '// 曾经有人读 a.one，现在没有\n' }),
    { baseline: '1:1:0', vocab: '1:1' },
  )
  assert.equal(r.status, 1, r.stderr + r.stdout)
  assert.match(r.stderr, /「a\.one」在 core 与插件的生产代码里都没有读者/)
})

test('CS-006：没有插件 crate 的夹具仓库 ⇒ 不适用，不许静默当成已通过', () => {
  const r = audit(core(ONE, vocab()), { baseline: '1:1:0', vocab: '1:1' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-006.*不适用/)
})

// ==================== CS-010 控制面草稿字段棘轮 ====================

const TURN_STATE = 'crates/symbio-plugin-loop/src/chat_loop/state.rs'
/** 一个只有字段的草稿夹具（字段名随便起，棘轮数的是格数） */
const draft = (fields) =>
  `pub(crate) struct TurnState {\n${fields.map((f) => `    pub(crate) ${f}: u8,`).join('\n')}\n}\n`

test('CS-010：草稿字段数等于基线 ⇒ 通过', () => {
  const r = audit(core(ONE, { [TURN_STATE]: draft(['a', 'b']) }), {
    baseline: '1:1:0',
    turnState: '2',
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-010.*turnStateFields 2\/2/)
})

test('CS-010：草稿多一格 ⇒ 红（每多一格就多一份只有某条路径知道的状态）', () => {
  const r = audit(core(ONE, { [TURN_STATE]: draft(['a', 'b', 'c']) }), {
    baseline: '1:1:0',
    turnState: '2',
  })
  assert.equal(r.status, 1, '草稿变宽必须红')
  assert.match(r.stderr, /CS-010/)
  assert.match(r.stderr, /字段数实测 3 > 基线 2/)
})

test('CS-010：反向——泛型 / 元组 / 属性的逗号不算字段分隔', () => {
  const src =
    'pub(crate) struct TurnState {\n' +
    '    #[allow(dead_code)]\n' +
    '    a: BTreeMap<u8, u8>,\n' +
    '    b: Option<(String, String)>,\n' +
    '}\n'
  const r = audit(core(ONE, { [TURN_STATE]: src }), { baseline: '1:1:0', turnState: '2' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('CS-010：文件在但读不出 TurnState ⇒ 红（判据失去输入，宁红不静默）', () => {
  const r = audit(core(ONE, { [TURN_STATE]: 'pub(crate) struct Other {\n    a: u8,\n}\n' }), {
    baseline: '1:1:0',
    turnState: '2',
  })
  assert.equal(r.status, 1, '草稿被改名 ⇔ 棘轮失去输入')
  assert.match(r.stderr, /CS-010/)
  assert.match(r.stderr, /读不出/)
})

test('CS-010：真仓缺草稿文件 ⇒ 红（文件删了，判据不能跟着消失）', () => {
  const five =
    ONE +
    'pub trait Capability {\n    fn c(&self);\n}\n' +
    'pub trait ModelProvider {\n    fn m(&self);\n}\n' +
    'pub trait VdfsProvider {\n    fn v(&self);\n}\n' +
    'pub trait Registry {\n    fn r(&self);\n}\n'
  const r = audit(
    {
      [CORE_MOD]: 'mod alpha;\n',
      [ALPHA]: five,
      'crates/symbio-core/src/plugin/mod.rs': '',
    },
    { baseline: '5:5:0' },
  )
  assert.equal(r.status, 1)
  assert.match(r.stderr, /CS-010/)
  assert.match(r.stderr, /读不出/)
})

test('CS-010：夹具仓库没有草稿文件 ⇒ 不适用（不许静默当成已通过）', () => {
  const r = audit(core(ONE), { baseline: '1:1:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /CS-010.*不适用/)
})

/**
 * arch-shape-audit 回归测试 —— 证明它**会红**，且**不误报**
 *
 * 这个守卫的失效形态有两头，两头都要钉住：
 * ① **漏数**（提取写宽了）：形状变宽而绿灯 ⇒ 它守的那条纪律（SP-003 §5）回到「没人核对」；
 * ② **多数**（提取写严了）：把默认体里的局部 `fn`、测试里的 trait 都算成成员 ⇒ 一片误报，
 *    最后被人用放宽注释喂到失效——本仓为这种结局写过一整节（`core-export-audit` 的 W1 注）。
 *
 * 覆盖：
 *   1. 形状 == 基线 ⇒ 退出 0；
 *   2. CS-001 三种变宽各自变红：多一个 `pub trait` / 既有 trait 多一个方法 / 必需成员长出身；
 *   3. 反向：默认体里的局部 `fn` 不算成员；`#[cfg(test)]` 模块与 `*.test.rs` 不算形状；
 *   4. CS-002：放宽注释**没有决策编号** / 决策**打不开** / 格的键名不认识 ⇒ 各自红；
 *      决策能打开 ⇒ 不红（判据不拦「有决策的放宽」，那是论证的事）；
 *   5. 范围读不出（没有 core）⇒ 红，不是静默通过；
 *   6. env 覆盖少给一格 ⇒ 红（那一格的判据不能凭空消失）。
 *
 * 用法：node --test scripts/audits/arch-shape-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'
import { expectAudit, onStderr } from '../audit-assert.mjs'

const script = fileURLToPath(new URL('./arch-shape-audit.mjs', import.meta.url))


const CORE_MOD = 'crates/symbio-sdk/src/lib.rs'
const ALPHA = 'crates/symbio-sdk/src/alpha/mod.rs'
const KERNEL = 'crates/symbio-sdk/src/kernel.rs'
/** 一个成员、无默认体的最小夹具 */
const ONE = 'pub trait Store {\n    fn one(&self) -> u32;\n}\n'

/**
 * 建夹具 → 跑审计 → 返回 spawnSync 结果。
 *
 * `baseline` 就是 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>`：夹具只有几个 trait，
 * 不显式给基线就测不到棘轮（真仓的值是 20:72:24）。
 * `data` 是 `CORE_DATA_BASELINE=<structFields>:<enumVariants>`（CS-009），默认 `0:0`——
 * 绝大多数夹具里没有公共数据面，给 `0:0` 让提示噪音不出现在无关用例里。
 */
function audit(files, { baseline = '1:1:0', vocab, data = '0:0' } = {}) {
  return withTempTree(files, (root) => {
    const env = { ...process.env, NO_COLOR: '1', CORE_SHAPE_BASELINE: baseline, CORE_DATA_BASELINE: data }
    if (vocab !== undefined) env.VOCAB_BASELINE = vocab
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  }, { prefix: 'core-shape-' })
}

const core = (traitSrc, extra = {}) => ({ [CORE_MOD]: 'mod alpha;\n', [ALPHA]: traitSrc, ...extra })

// ==================== CS-001 棘轮 ====================

test('形状等于基线 ⇒ 通过', () => {
  const r = audit(core(ONE), { baseline: '1:1:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('多一个 pub trait（新增接缝）⇒ CS-001 变红', () => {
  const r = audit(
    core(ONE, { 'crates/symbio-sdk/src/beta/mod.rs': 'pub trait Registry {\n    fn b(&self);\n}\n' }),
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
      { 'crates/symbio-sdk/src/alpha/inline.test.rs': 'pub trait Ghost {\n    fn x(&self);\n    fn y(&self);\n}\n' },
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
      'crates/symbio-sdk/src/beta/mod.rs': 'pub trait Registry {\n    fn b(&self);\n}\n',
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

// ==================== CS-004：登记抵扣（2026-11-03 收窄） ====================

/** 五格齐全 + 宿主侧插件域 + 一张草稿，是 CS-004 夹具的最小合法形状。 */
const FIVE_TRAITS =
  ONE +
  'pub trait Capability {\n    fn c(&self);\n}\n' +
  'pub trait ModelProvider {\n    fn m(&self);\n}\n' +
  'pub trait VdfsProvider {\n    fn v(&self);\n}\n' +
  'pub trait Registry {\n    fn r(&self);\n}\n'

const NOUN_ALLOW_OK =
  '// arch-shape-allow CS-004 tool ×1: 用途=wire 词汇（家族名）；不是插件引用=内核不认插件实现；依据=D-303；登记=2026-11-03\n'

/** CS-004 夹具：内核文件 `alpha` 里可以放字面量；插件词表 = `tool`。 */
const nounFiles = (alphaSrc, extra = {}) => ({
  [CORE_MOD]: 'mod alpha;\nmod kernel;\n',
  [ALPHA]: alphaSrc,
  [KERNEL]: '// core-kernel-files: alpha/mod.rs\n',
  'crates/symbio-sdk/src/plugin/mod.rs': '',
  'crates/symbio-plugin-tool/src/.keep': '',
  'docs/003-design/047-义务环判据与归属推导.md': '# DE-047\n\n### D-303 夹具决策\n\n理由。\n',
  ...extra,
})

test('CS-004：与内核既有 wire 词同名的字面量可按登记抵扣（判耦合，不判字符串相同）', () => {
  const r = audit(nounFiles(FIVE_TRAITS + 'pub const T: &str = "tool";\n' + NOUN_ALLOW_OK), {
    baseline: '5:5:0',
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('CS-004：登记处数超过实测 ⇒ 红（登记了却没有的代码是空豁免）', () => {
  const r = audit(
    nounFiles(FIVE_TRAITS + 'pub const T: &str = "tool";\n' + NOUN_ALLOW_OK.replace('×1', '×2')),
    { baseline: '5:5:0', turnState: '1' },
  )
  assert.equal(r.status, 1, '多登记必须红，否则登记表可以拿来预先囤额度')
  assert.match(r.stderr, /登记超过实测 1 处/)
})

test('CS-004：登记缺 `依据=` ⇒ 红（豁免必须挂一条决策）', () => {
  const r = audit(
    nounFiles(
      FIVE_TRAITS + 'pub const T: &str = "tool";\n' + NOUN_ALLOW_OK.replace('依据=D-303；', ''),
    ),
    { baseline: '5:5:0', turnState: '1' },
  )
  assert.equal(r.status, 1)
  assert.match(r.stderr, /没有决策编号/)
})

test('CS-004：登记缺 `登记=` 日期 ⇒ 红', () => {
  const r = audit(
    nounFiles(
      FIVE_TRAITS + 'pub const T: &str = "tool";\n' + NOUN_ALLOW_OK.replace('登记=2026-11-03', ''),
    ),
    { baseline: '5:5:0', turnState: '1' },
  )
  assert.equal(r.status, 1)
  assert.match(r.stderr, /缺日期/)
})

test('CS-004：登记写在内核文件集之外 ⇒ 红（判据面之外没有豁免权）', () => {
  const r = audit(
    nounFiles(FIVE_TRAITS, {
      [CORE_MOD]: 'mod alpha;\nmod gamma;\nmod kernel;\n',
      'crates/symbio-sdk/src/gamma/mod.rs': 'pub const G: &str = "tool";\n' + NOUN_ALLOW_OK,
    }),
    { baseline: '5:5:0', turnState: '1' },
  )
  assert.equal(r.status, 1)
  assert.match(r.stderr, /豁免只能登记在 CS-004 判定的内核文件集里/)
})

test('CS-004：登记扣不完的部分仍是真红', () => {
  const r = audit(
    nounFiles(FIVE_TRAITS + 'pub const A: &str = "tool";\npub const B: &str = "tool";\n' + NOUN_ALLOW_OK),
    { baseline: '5:5:0', turnState: '1' },
  )
  assert.equal(r.status, 1, '扣 1 处后仍有 1 处，必须按回流判红')
  assert.match(r.stderr, /有 1 处 > 冻结 0/)
})

test('CS-004 只量内核文件集：域文件里的插件 id 字面量不判红（报告账）', () => {
  // 宿主侧插件域（`symbio-sdk/src/plugin/mod.rs`）存在 ⇒ 五格齐全检查生效，
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
    'crates/symbio-sdk/src/gamma/mod.rs': 'pub const G: &str = "session";\n',
    [KERNEL]: '// core-kernel-files: alpha/mod.rs\n',
    'crates/symbio-sdk/src/plugin/mod.rs': '',
    'crates/symbio-plugin-session/src/.keep': '',
  }
  const ok = audit(files, { baseline: '5:5:0' })
  assert.equal(ok.status, 0, ok.stderr + ok.stdout)

  const bad = audit(
    { ...files, [ALPHA]: five + 'pub const A: &str = "session";\n' },
    { baseline: '5:5:0' },
  )
  assert.equal(bad.status, 1, '同一字面量落进内核文件就必须红')
  assert.match(bad.stderr, /CS-004/)
  assert.match(bad.stderr, /回流/)
})

// ==================== CS-002 放宽必须有能打开的决策 ====================

test('放宽注释里没有决策编号 ⇒ CS-002 变红', () => {
  const r = audit(
    core(ONE, { 'scripts/audits/arch-shape-audit.mjs': '// core-shape-allow methods: 批次需要，先放宽一点\n' }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 1, '没有决策的放宽必须红')
  assert.match(r.stderr, /CS-002/)
  assert.match(r.stderr, /没有决策编号/)
})

test('放宽注释引用了不存在的决策 ⇒ CS-002 变红', () => {
  const r = audit(
    core(ONE, { 'scripts/audits/arch-shape-audit.mjs': '// core-shape-allow methods: D-077 扩到 2 个成员的理由\n' }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 1, '指向打不开的决策 = 空引用')
  assert.match(r.stderr, /D-077.*条目/)
})

test('放宽注释挂着能打开的决策 ⇒ 通过（棘轮不判论证内容，只保证决策存在）', () => {
  const r = audit(
    core(ONE, {
      'scripts/audits/arch-shape-audit.mjs': '// core-shape-allow methods: D-077 扩到 2 个成员的理由\n',
      'crates/symbio-plugin-heartbeat/README.md': '# 心跳插件\n\n### D-077 夹具决策\n\n理由。\n',
    }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('放宽一个不存在的格 ⇒ CS-002 变红（拼错的键名不能当豁免）', () => {
  const r = audit(
    core(ONE, {
      'scripts/audits/arch-shape-audit.mjs': '// core-shape-allow methodz: D-077 打错了键名\n',
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
  const r = audit(core(ONE, { 'crates/symbio-sdk/src/beta/mod.rs': DATA_TWO }), { data: '2:2' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('多一个枚举变体（封闭取值面变宽）⇒ CS-009 变红', () => {
  const r = audit(
    core(ONE, {
      'crates/symbio-sdk/src/beta/mod.rs': DATA_TWO.replace('    Y,\n', '    Y,\n    Z,\n'),
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
      'crates/symbio-sdk/src/beta/mod.rs': DATA_TWO.replace('    pub y: u8,\n', '    pub y: u8,\n    pub z: u8,\n'),
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
  const r = audit(core(ONE, { 'crates/symbio-sdk/src/beta/mod.rs': src }), { data: '3:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('反向：tuple struct 的位置字段计入，`pub(crate)` 与测试模块不计', () => {
  const src =
    'pub struct Pair(u8, u8);\n\npub(crate) struct Hidden {\n    a: u8,\n    b: u8,\n}\n\n#[cfg(test)]\nmod t {\n    pub struct Fake {\n        a: u8,\n    }\n}\n'
  const r = audit(core(ONE, { 'crates/symbio-sdk/src/beta/mod.rs': src }), { data: '2:0' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('CS-009 的放宽同样要挂能打开的决策（`core-shape-allow enumVariants`）', () => {
  const bumped = DATA_TWO.replace('    Y,\n', '    Y,\n    Z,\n')
  const withoutDecision = audit(core(ONE, { 'crates/symbio-sdk/src/beta/mod.rs': bumped }), {
    data: '2:3',
  })
  // 没有放宽注释时不该红：数字与基线一致。放宽注释只在「改数字」时才需要解释。
  assert.equal(withoutDecision.status, 0, withoutDecision.stderr + withoutDecision.stdout)

  const waived = audit(
    core(ONE, {
      'crates/symbio-sdk/src/beta/mod.rs': bumped,
      'scripts/audits/arch-shape-audit.mjs': '// core-shape-allow enumVariants: D-209 领域扩展位的理由\n',
      'docs/003-design/025-能力包与扩展面.md': '# DE-025\n\n### D-209 领域轴用一个扩展位\n\n理由。\n',
    }),
    { data: '2:3' },
  )
  assert.equal(waived.status, 0, waived.stderr + waived.stdout)
  assert.match(waived.stdout, /enumVariants←D-209/)

  const dangling = audit(
    core(ONE, {
      'crates/symbio-sdk/src/beta/mod.rs': bumped,
      'scripts/audits/arch-shape-audit.mjs': '// core-shape-allow enumVariants: D-999 指向不存在的决策\n',
    }),
    { data: '2:3' },
  )
  assert.equal(dangling.status, 1, '指向打不开的决策 = 空引用')
  assert.match(dangling.stderr, /D-999.*条目/)
})

// ==================== 判据自己不能哑火 ====================

test('读不出 core 范围 ⇒ 退出 1（宁红不静默）', () => {
  const r = audit({ 'README.md': '没有 symbio_kernel 的仓库\n' }, { baseline: '0:0:0' })
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
  expectAudit(r, 0, [/CS-005.*kernelEvents 1\/1.*cellsRows 1\/1/])
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
  expectAudit(r, 0, [/cellsRows 0\/0/])
})

test('CS-005：退役的 grid.rs 被加回内核 ⇒ 红（名字重新写进语法就是回潮）', () => {
  const r = audit(core(ONE, vocab(['a.one'], ['EVENT_A'])), { baseline: '1:1:0', vocab: '1:0' })
  expectAudit(r, 1, [onStderr(/cellsRows 实测 1 > 冻结 0/)])
})

test('CS-005：收到基线下方 ⇒ 通过并提示同步数字', () => {
  const r = audit(core(ONE, vocab([], [])), { baseline: '1:1:0', vocab: '1:1' })
  expectAudit(r, 0, [/kernelEvents 0<1/])
})

// ==================== CS-008 core 生产代码零事件名字面量 ====================

const FIXTURES = 'crates/symbio-sdk/src/test_fixtures.rs'
const CORE_DELTA = 'crates/symbio-sdk/src/delta/mod.rs'

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
  expectAudit(r, 1, [onStderr(/CS-008/), onStderr(/chat\.assistant\.final/)])
})

test('CS-008：`#[cfg(test)] mod x;` 声明的夹具文件不算生产代码 ⇒ 不红', () => {
  const src = 'pub(crate) const EVENT_FINAL: &str = "chat.assistant.final";\n'
  const lib = 'mod alpha;\n#[cfg(test)]\nmod test_fixtures;\n'
  const r = audit(core(ONE, { ...vocabWithEntity(), [CORE_MOD]: lib, [FIXTURES]: src }), {
    baseline: '1:1:0',
    vocab: '1:0',
  })
  expectAudit(r, 0, [/CS-008.*剥掉 1 个/])
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
const CORE_BETA = 'crates/symbio-sdk/src/beta/mod.rs'

test('CS-006：只有一个插件消费、core 已不读 ⇒ 红（这条名字该随 owner 的 EventDecl 走）', () => {
  const r = audit(
    core(ONE, { ...vocab(), [ALPHA_CRATE]: 'pub fn k() { let _ = EVENT_A; }\n' }),
    { baseline: '1:1:0', vocab: '1:1' },
  )
  expectAudit(r, 1, [onStderr(/CS-006/), onStderr(/「a\.one」的唯一生产消费方是 symbio-plugin-alpha/)])
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
  expectAudit(r, 0, [/CS-006.*1 条都不在「单一消费方且 core 不读」这一形态（≥2 crate 消费 1/])
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
  expectAudit(r, 0, [/CS-006.*core 读者 1/])
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
  expectAudit(r, 1, [onStderr(/「a\.one」在 core 与插件的生产代码里都没有读者/)])
})

test('CS-006：没有插件 crate 的夹具仓库 ⇒ 不适用，不许静默当成已通过', () => {
  const r = audit(core(ONE, vocab()), { baseline: '1:1:0', vocab: '1:1' })
  expectAudit(r, 0, [/CS-006.*不适用/])
})

// ==================== CS-010 内核不含业务名 ====================

/** 一份最小的 `PLUGIN.yml`：`principal_<角色>: <owner>|<能力名+…>|<可见性>`。 */
const PLUGIN_YML = 'principal_main: session/main|judge.intent+reply|thread_private\n'

/**
 * CS-010 夹具：内核文件集（`alpha`）+ 一个声明了 owner / 能力名的插件。
 *
 * `alpha` 在文件集里 ⇒ 它是判据面；`beta` 不在 ⇒ 域文件，同一个字面量不该红。
 */
const bizFiles = (alphaSrc, extra = {}) => ({
  [CORE_MOD]: 'mod alpha;\nmod beta;\nmod kernel;\n',
  [ALPHA]: alphaSrc,
  [KERNEL]: '// core-kernel-files: alpha/mod.rs\n',
  'crates/symbio-plugin-biz/PLUGIN.yml': PLUGIN_YML,
  ...extra,
})

test('CS-010：内核文件集写死能力名 ⇒ 红（能力坐标回流）', () => {
  const r = audit(bizFiles(ONE + 'pub const C: &str = "judge.intent";\n'), {
    baseline: '1:1:0',
  })
  expectAudit(r, 1, [onStderr(/CS-010/), onStderr(/judge\.intent/)])
})

test('CS-010：内核文件集写死 owner ⇒ 红（主体坐标回流）', () => {
  const r = audit(bizFiles(ONE + 'pub const O: &str = "session/main";\n'), {
    baseline: '1:1:0',
  })
  expectAudit(r, 1, [onStderr(/CS-010/), onStderr(/session\/main/)])
})

test('CS-010：同名子串（不带引号）与注释里的名字 ⇒ 不红（判业务坐标，不判措辞）', () => {
  const r = audit(
    bizFiles(
      ONE +
        '// 见 `session/main` 的能力清单：judge.intent。\n' +
        'pub fn f(owner: &str) -> bool {\n    owner.starts_with("session")\n}\n',
    ),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('CS-010 只量内核文件集：域文件里的业务名字面量不判红', () => {
  const r = audit(
    bizFiles(ONE, { 'crates/symbio-sdk/src/beta/mod.rs': 'pub const N: &str = "session/main";\n' }),
    { baseline: '1:1:0' },
  )
  expectAudit(r, 0, [/CS-010.*14 个|CS-010.*1 个 owner/])
})

test('CS-010：没有 `principal_*` 声明的夹具仓库 ⇒ 不适用（不是「扫 0 条 = 全绿」）', () => {
  const r = audit(core(ONE), { baseline: '1:1:0' })
  expectAudit(r, 0, [/CS-010.*不适用/])
})


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
 */
function audit(files, { baseline = '1:1:0', vocab } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-shape-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const env = { ...process.env, NO_COLOR: '1', CORE_SHAPE_BASELINE: baseline }
    if (vocab !== undefined) env.VOCAB_BASELINE = vocab
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
  }
  const ok = audit(files, { baseline: '5:5:0' })
  assert.equal(ok.status, 0, ok.stderr + ok.stdout)

  const bad = audit({ ...files, [ALPHA]: five + 'pub const A: &str = "session";\n' }, { baseline: '5:5:0' })
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
  assert.match(r.stderr, /不是形状基线的格/)
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

/** 两张手写表的最小夹具：协议五格照真仓写法，业务行由参数给。 */
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

test('CS-005：两张表只在一半 ⇒ 红（判据输入缺一半不许静默）', () => {
  const onlyEvents = { [EVENT_MOD]: vocab()['crates/symbio-kernel/src/event/mod.rs'] }
  const r = audit(core(ONE, onlyEvents), { baseline: '1:1:0', vocab: '1:1' })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /缺一半|只找到/)
})

test('CS-005：收到基线下方 ⇒ 通过并提示同步数字', () => {
  const r = audit(core(ONE, vocab([], [])), { baseline: '1:1:0', vocab: '1:1' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /kernelEvents 0<1/)
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

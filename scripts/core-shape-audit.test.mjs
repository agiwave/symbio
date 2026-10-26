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

const CORE_MOD = 'symbio/src/symbio_core/mod.rs'
const ALPHA = 'symbio/src/symbio_core/alpha/mod.rs'
/** 一个成员、无默认体的最小夹具 */
const ONE = 'pub trait Store {\n    fn one(&self) -> u32;\n}\n'

/**
 * 建夹具 → 跑审计 → 返回 spawnSync 结果。
 *
 * `baseline` 就是 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>`：夹具只有几个 trait，
 * 不显式给基线就测不到棘轮（真仓的值是 20:72:24）。
 */
function audit(files, { baseline = '1:1:0' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-shape-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1', CORE_SHAPE_BASELINE: baseline },
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
    core(ONE, { 'symbio/src/symbio_core/beta/mod.rs': 'pub trait Registry {\n    fn b(&self);\n}\n' }),
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
      { 'symbio/src/symbio_core/alpha/inline.test.rs': 'pub trait Ghost {\n    fn x(&self);\n    fn y(&self);\n}\n' },
    ),
    { baseline: '1:1:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
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
      'symbio/src/plugins/heartbeat/README.md': '# 心跳插件\n\n### D-077 夹具决策\n\n理由。\n',
    }),
    { baseline: '1:2:0' },
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('放宽一个不存在的格 ⇒ CS-002 变红（拼错的键名不能当豁免）', () => {
  const r = audit(
    core(ONE, {
      'scripts/core-shape-audit.mjs': '// core-shape-allow methodz: D-077 打错了键名\n',
      'symbio/src/plugins/heartbeat/README.md': '# 心跳插件\n\n### D-077 夹具决策\n\n理由。\n',
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

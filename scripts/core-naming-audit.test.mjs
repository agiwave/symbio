// core-naming-audit 回归测试
//
// 本脚本是**判定型**（有违规即退出码 1），所以它最危险的失效形态是
// **永远不命中**：规则写错了、前缀比对写宽了、或规则源根本没读到——
// 三种情况下它都会安静地亮绿灯。一个只会亮绿灯的守卫等于没有守卫。
//
// 因此下面每条规则都配一个**注入真实违规**的反例，断言脚本确实变红；
// 另外配正例钉住四件「不是违规」的事（限定词 / 子模块 / 词级匹配 / 函数带前缀），
// 免得守卫靠误报活着——一个只会误报的守卫最后一定会被人用豁免喂到失效。
//
// 最后一条测试跑**真实仓库**：夹具只能证明规则引擎自洽，证明不了 README 的那张表
// 与当前公开面一致——而后者才是这份守卫存在的理由。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const script = path.join(scriptDir, 'core-naming-audit.mjs')
const repoRoot = path.resolve(scriptDir, '..')

/** 表头：**五列**，第四列必须是「函数 / 自由函数」（守卫会校验这一点） */
const HEADER = '| 域 | 类型 / trait | 常量 / 静态量 | 函数 / 自由函数 | 备注 |'

/** 造一份 README：`table` 是表格行（不含表头），`modifiers` 是限定词标记内容 */
function readme(table, modifiers = 'Dyn,Default', header = HEADER) {
  return [
    '# symbio_core —— 内核契约层：命名与结构规范',
    '',
    '### 域前缀对照表（全 N 域 —— 新增符号照此取名）',
    '',
    `<!-- core-naming:modifiers ${modifiers} -->`,
    '',
    header,
    '|---|---|---|---|---|',
    table,
    '',
    '> 表结束。',
    '',
  ].join('\n')
}

/** 造一棵最小仓库：`domains` 是 `{ 域名: mod.rs 内容 }`，根 mod.rs 默认通配重导出各域 */
function repo({ table, domains = {}, rootMod, modifiers, header }) {
  const files = {
    'symbio/src/symbio_core/README.md': readme(table, modifiers, header),
    'symbio/src/symbio_core/mod.rs':
      rootMod ?? Object.keys(domains).map((d) => `pub use ${d}::*;\n`).join(''),
  }
  for (const [d, src] of Object.entries(domains)) {
    files[`symbio/src/symbio_core/${d}/mod.rs`] = src
  }
  return files
}

/** 把文件写进临时仓库并跑 core-naming-audit */
function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-naming-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 10000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const out = (r) => r.stdout + r.stderr

// ── N-001：前缀必须落在所属域登记的前缀里 ────────────────────────────────

test('N-001 反例：类型不匹配本域前缀 ⇒ 红', () => {
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: 'pub struct WrongName;\n' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-001/, out(r))
  assert.match(out(r), /WrongName/, out(r))
})

test('N-001 反例：常量不匹配本域前缀 ⇒ 红', () => {
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: 'pub const WRONG: &str = "x";\n' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-001/, out(r))
})

test('N-001 反例：函数不匹配本域前缀 ⇒ 红（§1.2 收紧：函数与类型 / 常量同规）', () => {
  // `now_ms` 这类名字的由来：根平铺导入后，`use crate::symbio_core::{now_ms}`
  // 读不出它属于哪个域——「导入行已带着模块路径」这个豁免理由是假的。
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: 'pub fn whatever() {}\n' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-001/, out(r))
  assert.match(out(r), /whatever/, out(r))
})

test('N-001 反例：该域登记为「—」却出现了常量 ⇒ 红', () => {
  // `—` 的语义是「本域没有这一类符号」，不是「不用前缀」——前者是可判定的，后者不是。
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | — | — | |',
      domains: { alpha: 'pub const ALPHA_X: &str = "x";\n' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-001/, out(r))
})

// ── N-003：用了别的域的前缀 ⇒ 报「错放」，并说出前缀属于谁 ───────────────

test('N-003 反例：符号用了别的域的前缀 ⇒ 红，且指明该前缀属于谁', () => {
  // 这正是真实事故的形状：`KEY_PROVIDER` 住在 `plugin` 却用着 `keys` 域的前缀。
  const r = audit(
    repo({
      table: [
        '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
        '| `beta` | `Beta` | `BETA_` | `beta_` | |',
      ].join('\n'),
      domains: { beta: 'pub struct AlphaThing;\n' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-003/, out(r))
  assert.match(out(r), /AlphaThing/, out(r))
  assert.match(out(r), /`alpha` 域的前缀/, out(r))
})

// ── N-002：一域一前缀（表级）────────────────────────────────────────────

test('N-002 反例：两个域登记同一前缀 ⇒ 红', () => {
  // 「一域一前缀，前缀不跨域复用」——`PLUGIN_` 曾同时属于 plugin 与 keys。
  const r = audit(
    repo({
      table: [
        '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
        '| `beta` | `Alpha` | `BETA_` | `beta_` | |',
      ].join('\n'),
      domains: { alpha: '', beta: '' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-002/, out(r))
})

// ── N-004：域目录 ↔ 表 双向一一对应 ─────────────────────────────────────

test('N-004 反例：域目录存在但表里没登记 ⇒ 红', () => {
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: '', beta: '' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-004/, out(r))
  assert.match(out(r), /beta/, out(r))
})

test('N-004 反例：表里登记了不存在的域 ⇒ 红（表在说假话）', () => {
  const r = audit(
    repo({
      table: [
        '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
        '| `ghost` | `Ghost` | `GHOST_` | `ghost_` | |',
      ].join('\n'),
      domains: { alpha: '' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-004/, out(r))
  assert.match(out(r), /ghost/, out(r))
})

// ── N-005：公开面符号必须有归属域 ───────────────────────────────────────

test('N-005 反例：根 mod.rs 直接声明的常量没有归属域 ⇒ 红', () => {
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: '' },
      rootMod: 'pub use alpha::*;\npub const ROOT_THING: &str = "x";\n',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-005/, out(r))
  assert.match(out(r), /ROOT_THING/, out(r))
})

// ── 正例：四件「不是违规」的事 ──────────────────────────────────────────

test('正例：限定词前缀不算越界（Dyn / Default）', () => {
  // `DynVdfsProvider` = `Dyn` + `VdfsProvider`——限定词由 README 的标记持有。
  const r = audit(
    repo({
      table: '| `vdfs` | `Vdfs` | `VDFS_` | `vdfs_` | |',
      domains: { vdfs: 'pub struct DynVdfsProvider;\npub struct VdfsNode;\n' },
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('正例：函数带本域前缀 ⇒ 绿（收紧后的正常形态）', () => {
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: 'pub fn alpha_whatever() {}\n' },
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('正例：子模块（命名空间）不判前缀 —— 它是主题名，不是符号', () => {
  // `pub use capability::failure_kind;` 是**模块**重导出，snake_case 会被 `kindOf`
  // 读成「函数」。模块是命名空间，前缀规则对它不适用（§1.2 的「子命名空间」手段）。
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
      domains: { alpha: 'pub mod failure_kind {\n    pub const X: &str = "x";\n}\n' },
      rootMod: 'pub use alpha::*;\npub use alpha::failure_kind;\n',
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('正例：前缀按「词」比对，含别域前缀词不误判', () => {
  // `VDFS_PLUGIN_PROVIDER_FIELD` 含 `PLUGIN_`，但首词是 `VDFS` ⇒ 属于 vdfs。
  // 若按字面 contains 比对，它会同时命中 plugin 域 —— 一个只会误报的守卫。
  const r = audit(
    repo({
      table: [
        '| `vdfs` | `Vdfs` | `VDFS_` | `vdfs_` | |',
        '| `plugin` | `Plugin` | `PLUGIN_` | `plugin_` | |',
      ].join('\n'),
      domains: { vdfs: 'pub const VDFS_PLUGIN_PROVIDER_FIELD: &str = "x";\n', plugin: '' },
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('正例：`keys` 域类型用后缀、实例用裸名', () => {
  const r = audit(
    repo({
      table: '| `keys` | `…Key`（**后缀**） | **裸名**（实例） | — | |',
      domains: { keys: 'pub struct PathKey;\npub const PATH: PathKey = PathKey;\n' },
    }),
  )
  assert.equal(r.status, 0, out(r))
})

test('反例：`keys` 域类型不带 `Key` 后缀 ⇒ 红', () => {
  // `KeyPath` 会读成「键的路径」，语义反了 —— 后缀规则存在的理由就是这个。
  const r = audit(
    repo({
      table: '| `keys` | `…Key`（**后缀**） | **裸名**（实例） | — | |',
      domains: { keys: 'pub struct KeyPath;\n' },
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /N-001/, out(r))
  assert.match(out(r), /KeyPath/, out(r))
})

// ── 规则源不可解析 ⇒ 必须红，不能静默放过 ───────────────────────────────

test('反例：README 缺限定词标记 ⇒ 红（绿灯只说明没检查）', () => {
  const files = repo({
    table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
    domains: { alpha: '' },
  })
  files['symbio/src/symbio_core/README.md'] =
    '# x\n\n### 域前缀对照表（全 1 域）\n\n' +
    `${HEADER}\n|---|---|---|---|---|\n| \`alpha\` | \`Alpha\` | \`ALPHA_\` | \`alpha_\` | |\n`
  const r = audit(files)
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /规则源不可解析/, out(r))
})

test('反例：README 找不到域前缀对照表 ⇒ 红', () => {
  const files = repo({
    table: '| `alpha` | `Alpha` | `ALPHA_` | `alpha_` | |',
    domains: { alpha: '' },
  })
  files['symbio/src/symbio_core/README.md'] =
    '# x\n\n<!-- core-naming:modifiers Dyn -->\n\n没有那张表。\n'
  const r = audit(files)
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /规则源不可解析/, out(r))
})

test('反例：表头缺「函数」列 ⇒ 红（读歪了不能报得理直气壮）', () => {
  // 只有四列时 `cells[3]` 落到「备注」上 ⇒ 每个函数都被判成「该域没有这一类符号」，
  // 报得理直气壮而规则其实没读到。所以表头列数是**规则源的一部分**，必须校验。
  const r = audit(
    repo({
      table: '| `alpha` | `Alpha` | `ALPHA_` | |',
      domains: { alpha: '' },
      header: '| 域 | 类型 / trait | 常量 / 静态量 | 备注 |',
    }),
  )
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /规则源不可解析/, out(r))
  assert.match(out(r), /五列/, out(r))
})

// ── 真实仓库：夹具证明不了「表与当前公开面一致」─────────────────────────

test('真实仓库：README §1.2 的表与公开面一致（5 条规则全过）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: repoRoot,
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 20000,
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /5 条规则全部通过/, out(r))
})

/**
 * capability-migration-audit 回归测试 —— 证明 EX-003 / EX-007 **改输入后结论必须变**
 *
 * 这条守卫的失效形态都是「看起来在判」：
 *
 * - 有 `Migration:` 就放行 ⇒ 写上 trailer 即可在架构面夹带，D-247 的两类分家作废；
 * - 只判新增、不判删除 ⇒ 插件落地而旧实现留着（两份同时生效）照样绿；
 * - 只认 `*.gen.rs` ⇒ `plugin-api` 的 `events.rs` / `routes.rs` 被当成
 *   机制增量，补件几乎条条假红，豁免写满之后没人再看；
 * - 决策号只判形状不查存在 ⇒ 拼一个 `D-999` 就能进架构；
 * - 注释里的插件 id / 工具名算成代码 ⇒ 「补件不是通用的」这条判据自伤。
 *
 * 本文件逐条注入并断言红，另有一条真仓冒烟证明读 git 那一半也真的在跑。
 *
 * 用法：node --test scripts/capability-migration-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import {
  ARCH_TREES,
  BEFORE_GUARD,
  DEBT_EXEMPTIONS,
  DEBT_SCHEDULE,
  FIXTURE,
  NET_GUARD,
  activeDebtExemption,
  archNetChange,
  classifyCommit,
  crossTreeMoves,
  debtAllowance,
  generatedPaths,
  isCapabilitySource,
  isGenerated,
  migrationErrors,
  movedLines,
  parsePatch,
  parseRelocationHeader,
  relocationErrors,
  supplementErrors,
  supplementRatioReading,
  validateBeforeGuard,
  validateDebtSchedule,
} from './capability-migration-audit.mjs'

const script = fileURLToPath(new URL('./capability-migration-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(script), '..')

const CORE = 'crates/symbio-sdk/src/a.rs'
const LOOP = 'crates/symbio-plugin-loop/src/loop_rows.rs'
const SESSION = 'crates/symbio-plugin-session/src/lib.rs'

/** 一份最小的 `git show --unified=0` 补丁文本。 */
function patchText(files) {
  return files
    .map((f) => {
      const header = `diff --git a/${f.path} b/${f.path}\nindex 111..222 100644\n`
      if (f.new) {
        return `${header}--- /dev/null\n+++ b/${f.path}\n@@ -0,0 +1,${f.added.length} @@\n${f.added.map((l) => `+${l}`).join('\n')}`
      }
      if (f.deleted) {
        return `${header}--- a/${f.path}\n+++ /dev/null\n@@ -1,${f.removed.length} +0,0 @@\n${f.removed.map((l) => `-${l}`).join('\n')}`
      }
      return `${header}--- a/${f.path}\n+++ b/${f.path}\n@@ -1,${f.removed.length} +1,${f.added.length} @@\n${[
        ...f.removed.map((l) => `-${l}`),
        ...f.added.map((l) => `+${l}`),
      ].join('\n')}`
    })
    .join('\n')
}

const patchOf = (files) => parsePatch(patchText(files))
const decisions = new Set(['D-219', 'D-220', 'D-207'])

// ── parsePatch ────────────────────────────────────────────────────────────

test('parsePatch：多文件的增删各归各文件', () => {
  const p = patchOf([
    { path: CORE, added: ['pub fn a() {}'], removed: ['fn old() {}'] },
    { path: LOOP, added: [], removed: ['row!();'] },
  ])
  assert.deepEqual(
    p.map((f) => [f.path, f.added.length, f.removed.length]),
    [
      [CORE, 1, 1],
      [LOOP, 0, 1],
    ],
  )
})

test('parsePatch：新增文件的行记在自己名下，不挂到上一个文件', () => {
  const p = patchOf([
    { path: CORE, added: ['pub fn a() {}'], removed: ['fn old() {}'] },
    { path: 'crates/symbio-plugin-api/src/context.rs', new: true, added: ['pub struct Ctx;', 'pub fn b() {}'] },
  ])
  const ctx = p.find((f) => f.path.endsWith('context.rs'))
  assert.deepEqual(ctx.added, ['pub struct Ctx;', 'pub fn b() {}'])
  assert.equal(p.find((f) => f.path === CORE).added.length, 1)
})

test('parsePatch：删除文件按旧路径记账，并带 `deleted` 标记', () => {
  // `deleted` 是「搬家行」判据（`movedLines`）的唯一依据：只有整份文件消失，
  // 它的行才轮不到继续当通道。少了这个标记，`+++ /dev/null` 与「清空文件」长得一样。
  const p = patchOf([{ path: LOOP, deleted: true, removed: ['row!();'] }])
  assert.deepEqual(p, [{ path: LOOP, deleted: true, added: [], removed: ['row!();'] }])
})

test('parsePatch：hunk 里以 `--- ` 开头的删除行不是文件头', () => {
  const text = [
    'diff --git a/x.rs b/x.rs',
    '--- a/x.rs',
    '+++ b/x.rs',
    '@@ -1,1 +1,1 @@',
    '--- 一条被删掉的注释横线',
    '+/// 换成一条文档注释',
  ].join('\n')
  const [file] = parsePatch(text)
  assert.equal(file.path, 'x.rs')
  assert.deepEqual(file.removed, ['-- 一条被删掉的注释横线'])
  assert.deepEqual(file.added, ['/// 换成一条文档注释'])
})

// ── 文件类别 ──────────────────────────────────────────────────────────────

test('generatedPaths：描述符声明的输出都在清单里（含不带 .gen.rs 后缀的两份）', () => {
  const g = generatedPaths(repoRoot)
  for (const p of [
    'crates/symbio-plugin-api/src/events.rs',
    'crates/symbio-plugin-api/src/routes.rs',
    'crates/symbio-sdk/src/authz/rows.gen.rs',
  ]) {
    assert.ok(g.has(p), `生成物清单漏了 ${p}`)
  }
})

test('generatedPaths：读不出目录时返回空集，不报错也不假装有输入', () => {
  assert.equal(generatedPaths(path.join(repoRoot, '不存在的目录')).size, 0)
})

test('isCapabilitySource：测试与生成物都不算可迁移源码', () => {
  assert.equal(isCapabilitySource(LOOP), true)
  assert.equal(isCapabilitySource('crates/symbio-plugin-loop/src/loop_rows.test.rs'), false)
  assert.equal(isCapabilitySource('crates/symbio-sdk/src/authz/rows.gen.rs'), false)
  assert.equal(isCapabilitySource('crates/symbio-plugin-api/src/events.rs'), false)
  assert.equal(isCapabilitySource('crates/symbio-plugin-api/src/context.rs'), true)
})

test('isGenerated：后缀与描述符两条路都认', () => {
  const g = new Set(['x/y/z.rs'])
  assert.equal(isGenerated('a/b.gen.rs', g), true)
  assert.equal(isGenerated('x/y/z.rs', g), true)
  assert.equal(isGenerated(CORE, g), false)
})

// ── 分类 ──────────────────────────────────────────────────────────────────

test('classifyCommit：Migration trailer 一律是迁移批次（不因新增公开项改判）', () => {
  const patch = patchOf([{ path: CORE, added: ['pub fn leak() {}'], removed: [] }])
  assert.equal(classifyCommit({ message: 'feat: 搬走一条\n\nMigration: L1', patch }), 'migration')
})

test('classifyCommit：没有 trailer、但在架构面新增公开项 ⇒ 补件', () => {
  const patch = patchOf([{ path: CORE, added: ['pub fn port() {}'], removed: [] }])
  assert.equal(classifyCommit({ message: 'feat: 加一条端口\n\nCore-Change: D-219', patch }), 'supplement')
})

test('classifyCommit：只改注释的架构面改动两类都不是', () => {
  const patch = patchOf([{ path: CORE, added: ['// 说明'], removed: ['// 旧说明'] }])
  assert.equal(classifyCommit({ message: 'docs: 注释', patch }), null)
})

// 判定面是 sdk / kernel：`plugin-api` 是插件面的通用契约，为端口执行体加读面、
// 把驱动契约定在这里，都不改内核的公开面与语义——判成补件等于让每次契约演进都填表。
test('classifyCommit：只在 plugin-api 新增公开项 ⇒ 不算补件（判定面是 sdk / kernel）', () => {
  const patch = patchOf([
    { path: 'crates/symbio-plugin-api/src/drive.rs', added: ['pub struct Drive;'], removed: [] },
  ])
  assert.equal(classifyCommit({ message: 'feat(port): 通用驱动契约', patch }), null)
})

test('classifyCommit：同批再动 sdk ⇒ 仍按补件判', () => {
  const patch = patchOf([
    { path: 'crates/symbio-plugin-api/src/drive.rs', added: ['pub struct Drive;'], removed: [] },
    { path: CORE, added: ['pub fn port() {}'], removed: [] },
  ])
  assert.equal(classifyCommit({ message: 'feat: 补件', patch }), 'supplement')
})

// 删符号时 `pub use` 名单必然被重写。它是**接线**不是机制面（与本文件 `isMechanismLine`
// 同一条口径，头注释也写明「`use` 行（含 `pub use` 转发）两边都不计」）——把它算成
// 「新增公开项」，每一个纯删除的收口提交都会被记一笔补件欠账。
test('classifyCommit：只重写 `pub use` 名单（删符号）⇒ 两类都不是', () => {
  const patch = patchOf([
    {
      path: CORE,
      added: ['pub use self::spec::{one, two};'],
      removed: ['pub use self::spec::{one, two, three, four};'],
    },
  ])
  assert.equal(classifyCommit({ message: 'refactor: 删掉零调用判据', patch }), null)
})

// ── 迁移批次 ──────────────────────────────────────────────────────────────

const IDS = ['memory', 'local', 'notes']
const TOOLS = ['todo_write', 'content_search']
const mig = (files) => migrationErrors({ sha: 'x', patch: patchOf(files), pluginIds: IDS, toolNames: TOOLS }).errors

test('migrationErrors：干净的迁移（只删 loop 旧实现）⇒ 绿', () => {
  assert.deepEqual(mig([{ path: LOOP, added: [], removed: ['row!("memory")'] }]), [])
})

test('migrationErrors：只搬 session 的实现也判得动', () => {
  assert.deepEqual(mig([{ path: SESSION, added: [], removed: ['fn old() {}'] }]), [])
})

test('migrationErrors：迁移提交在 SDK 机制面新增源码 ⇒ EX-007 红并点名 sdk（变宽一分未松）', () => {
  const [e] = mig([
    { path: CORE, added: ['pub fn helper() {}'], removed: [] },
    { path: LOOP, added: [], removed: ['row!("memory")'] },
  ])
  assert.match(e, /EX-007/)
  assert.match(e, /crates\/symbio-sdk/)
})

test('migrationErrors：机制面只加注释（如 arch-shape-allow 登记）⇒ 不算源码变宽', () => {
  assert.deepEqual(
    mig([
      { path: 'crates/symbio-kernel/src/capability/family.rs', added: ['// arch-shape-allow CS-004 tool ×1: 用途=wire 词汇'], removed: [] },
      { path: LOOP, added: [], removed: ['row!("memory")'] },
    ]),
    [],
  )
})

test('migrationErrors：机制面重写一条私有 use（删符号的必然结果）⇒ 不算源码变宽', () => {
  assert.deepEqual(
    mig([
      {
        path: 'crates/symbio-plugin-api/src/port.rs',
        added: ['use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};'],
        removed: ['use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt};'],
      },
      { path: LOOP, added: [], removed: ['row!("memory")'] },
    ]),
    [],
  )
})

test('migrationErrors：控制面新增一行只有字符串字面量（日志署名）⇒ 不算新通道', () => {
  assert.deepEqual(
    mig([{ path: LOOP, added: ['"session",'], removed: ['row!("old_memory")'] }]),
    [],
  )
})

test('migrationErrors：折叠整文件搬出机制面、落进能力包 ⇒ 绿（D-283 判的是变宽）', () => {
  assert.deepEqual(
    mig([
      { path: 'crates/symbio-sdk/src/projection/cost.rs', added: [], removed: ['pub fn cost_ledger() {}'] },
      { path: 'crates/symbio-plugin-cost/src/decl.rs', added: ['name: "cost.ledger"'], removed: [] },
      { path: LOOP, added: ['reports::read_all(snapshot)'], removed: ['cost_ledger()'] },
    ]),
    [],
  )
})

test('migrationErrors：控制面新增的行带能力名 ⇒ EX-007 红（两份可实现）', () => {
  const [e] = mig([{ path: LOOP, added: ['row!("memory")'], removed: ['row!("old_memory")'] }])
  assert.match(e, /EX-007/)
  assert.match(e, /memory/)
})

test('migrationErrors：控制面新增的行带工具名 ⇒ EX-007 红', () => {
  const [e] = mig([{ path: LOOP, added: ['if tool == "todo_write" {}'], removed: ['row!("memory")'] }])
  assert.match(e, /EX-007/)
  assert.match(e, /todo_write/)
})

test('migrationErrors：绑定切换的通用胶水不算新增能力（删旧实现 + 加通用行）⇒ 绿', () => {
  assert.deepEqual(
    mig([{ path: LOOP, added: ['plugin_row!(decl);'], removed: ['row!("memory")'] }]),
    [],
  )
})

test('migrationErrors：新增行只在注释里提能力名 ⇒ 绿（判的是代码）', () => {
  assert.deepEqual(
    mig([{ path: LOOP, added: ['// 原 todo_write 那一行已交给插件'], removed: ['row!("memory")'] }]),
    [],
  )
})

test('migrationErrors：迁移提交没有删除旧实现 ⇒ EX-003 红（改测试不算搬走）', () => {
  const [e] = mig([{ path: 'crates/symbio-plugin-loop/src/loop_rows.test.rs', added: [], removed: ['assert!(old);'] }])
  assert.match(e, /EX-003/)
})

test('migrationErrors：迁移中重生成 events.rs 不算碰机制面', () => {
  assert.deepEqual(
    mig([
      { path: 'crates/symbio-plugin-api/src/events.rs', added: ['const X: &str = "notes.recent";'], removed: [] },
      { path: LOOP, added: [], removed: ['row!("memory")'] },
    ]),
    [],
  )
})

test('migrationErrors：迁移中改测试与 .gen.rs 不判（断言与镜像跟着搬）', () => {
  assert.deepEqual(
    mig([
      { path: 'crates/symbio-plugin-loop/src/loop_rows.test.rs', added: ['#[test]'], removed: [] },
      { path: 'crates/symbio-sdk/src/authz/rows.gen.rs', added: ['X', 'Y'], removed: ['Z'] },
      { path: LOOP, added: [], removed: ['row!("memory")'] },
    ]),
    [],
  )
})

// ── 搬迁口径：净变化（D-404）─────────────────────────────────────────────

const API_TEXT = 'crates/symbio-plugin-api/src/text.rs'
const SDK_TEXT = 'crates/symbio-sdk/src/text.rs'
const MOVED = ['pub fn count(s: &str) -> usize { s.len() }', 'pub struct Tokenizer;']

/** 一份「实现从 plugin-api 搬进 sdk」的补丁；`stub` 为 false 时故意不留 `pub use`。 */
function relocationPatch({ toLines = MOVED.length, stub = true } = {}) {
  return [
    {
      path: API_TEXT,
      removed: MOVED,
      added: stub ? ['pub use symbio_sdk::text::*;'] : [],
    },
    {
      path: SDK_TEXT,
      added: [...MOVED, 'pub fn extra() {}'].slice(0, toLines),
      removed: [],
    },
  ]
}

const relocationMessage = (from = MOVED.length, to = MOVED.length) =>
  `feat: 搬 text 进 sdk\n\nMigration: T-053\nfrom: ${API_TEXT} (${from})\nto: ${SDK_TEXT} (${to})`

const reloc = (message, files) => relocationErrors({ message, patch: patchOf(files) }).errors

test('archNetChange：`use` 转发不算机制面，净零搬迁读作 0', () => {
  const net = archNetChange(patchOf(relocationPatch()))
  assert.deepEqual([net.added, net.removed, net.net], [MOVED.length, MOVED.length, 0])
})

test('parseRelocationHeader：两侧都写才算完整；缺一侧不算；多对按序配对', () => {
  const one = parseRelocationHeader(relocationMessage())
  assert.deepEqual(one.pairs, [{ from: { path: API_TEXT, lines: 2 }, to: { path: SDK_TEXT, lines: 2 } }])
  assert.equal(one.complete, true)
  assert.equal(one.declared, true)

  const half = parseRelocationHeader(`Migration: 搬 text\nfrom: ${API_TEXT} (2)`)
  assert.equal(half.complete, false)
  assert.equal(half.declared, true)

  const two = parseRelocationHeader(
    `Migration: 搬 text\nfrom: ${API_TEXT} (2)\nto: ${SDK_TEXT} (2)\nfrom: a/b.test.rs (9)\nto: c/d.test.rs (9)`,
  )
  assert.deepEqual(two.pairs, [
    { from: { path: API_TEXT, lines: 2 }, to: { path: SDK_TEXT, lines: 2 } },
    { from: { path: 'a/b.test.rs', lines: 9 }, to: { path: 'c/d.test.rs', lines: 9 } },
  ])
  assert.equal(two.complete, true)
})

test('classifyCommit：净零搬迁是 relocation（不记欠账），净减仍是迁移（抵 1 笔）', () => {
  const zero = patchOf(relocationPatch())
  assert.equal(classifyCommit({ message: relocationMessage(), patch: zero }), 'relocation')
  const shrink = patchOf([
    { path: API_TEXT, removed: MOVED, added: ['pub use symbio_sdk::text::*;'] },
    { path: SDK_TEXT, added: [MOVED[0]], removed: [] },
  ])
  assert.equal(classifyCommit({ message: relocationMessage(2, 1), patch: shrink }), 'migration')
})

test('classifyCommit：旧口径（NET_GUARD 之前）没有 relocation 这一类', () => {
  assert.equal(
    classifyCommit({ message: relocationMessage(), patch: patchOf(relocationPatch()), net: false }),
    'migration',
  )
})

test('relocationErrors：净零 + 完整声明 + api 留转发 ⇒ 绿', () => {
  assert.deepEqual(reloc(relocationMessage(), relocationPatch()), [])
})

test('relocationErrors 红 1：`to` 比 `from` 多搬 1 行机制面 ⇒ 红（不许借搬迁加面）', () => {
  const errors = reloc(relocationMessage(2, 3), relocationPatch({ toLines: 3 }))
  assert.ok(errors.some((e) => /多搬了 1 行/.test(e)), errors.join('\n'))
})

test('relocationErrors 红 2：`from` 行数写错 ⇒ 红（写错等于没写）', () => {
  const errors = reloc(relocationMessage(3, 2), relocationPatch())
  assert.ok(errors.some((e) => /`from:` 写了 3 行，实际/.test(e)), errors.join('\n'))
})

test('relocationErrors 红 2：没有 `Migration:` 却跨树搬同一份文本 ⇒ 红', () => {
  const errors = reloc('feat: 顺手挪了个文件', relocationPatch())
  assert.ok(errors.some((e) => /没有 `Migration:`/.test(e)), errors.join('\n'))
  assert.equal(classifyCommit({ message: 'feat: 顺手挪了个文件', patch: patchOf(relocationPatch()) }), 'relocation')
})

test('relocationErrors 红 3：搬走 plugin-api 实现却不留 `pub use` ⇒ E-009 红', () => {
  const errors = reloc(relocationMessage(), relocationPatch({ stub: false }))
  assert.ok(errors.some((e) => /E-009/.test(e) && /pub use/.test(e)), errors.join('\n'))
})

test('relocationErrors 红 3：零扩展消费方的模块搬走不要求空转转发（D-416）', () => {
  const from = 'crates/symbio-plugin-api/src/zz_fixture.rs'
  const to = 'crates/symbio-sdk/src/zz_fixture.rs'
  const message = `feat: 搬内部枚举进 sdk\n\nMigration: D-416\nfrom: ${from} (1)\nto: ${to} (1)`
  const files = [
    { path: from, removed: ['pub fn helper() {}'], added: [] },
    { path: to, added: ['pub fn helper() {}'], removed: [] },
  ]
  assert.deepEqual(reloc(message, files), [])
})

test('crossTreeMoves：同树内删了又加不算搬家', () => {
  const same = patchOf([
    { path: 'crates/symbio-sdk/src/a.rs', added: ['pub fn helper() {}'], removed: [] },
    { path: 'crates/symbio-sdk/src/b.rs', added: [], removed: ['pub fn helper() {}'] },
  ])
  assert.equal(crossTreeMoves(same).count, 0)
})

test('NET_GUARD：口径起点有提交号与理由，且不是判据起点本身', () => {
  assert.deepEqual(validateBeforeGuard(NET_GUARD), [])
  assert.match(NET_GUARD.sha, /^[0-9a-f]{7,40}$/)
  assert.notEqual(NET_GUARD.sha, BEFORE_GUARD.sha)
})

// ── 补件 ──────────────────────────────────────────────────────────────────

const supp = (message, files, { moved } = {}) =>
  supplementErrors({
    sha: 'x',
    message,
    patch: patchOf(files),
    decisions,
    pluginIds: ['notes', 'local'],
    toolNames: ['todo_write', 'content_search'],
    moved,
  }).errors

test('supplementErrors：决策号存在且新增面通用 ⇒ 绿', () => {
  assert.deepEqual(
    supp('feat: 端口\n\nCore-Change: D-219', [{ path: CORE, added: ['pub fn port() {}'], removed: [] }]),
    [],
  )
})

test('supplementErrors：没有决策号 ⇒ 红', () => {
  const [e] = supp('feat: 加了个口子', [{ path: CORE, added: ['pub fn port() {}'], removed: [] }])
  assert.match(e, /没有决策号/)
})

test('supplementErrors：决策号打不开 ⇒ 红（写错的号等于没写）', () => {
  const [e] = supp('feat: 端口\n\nCore-Change: D-999', [{ path: CORE, added: ['pub fn port() {}'], removed: [] }])
  assert.match(e, /打不开/)
})

test('supplementErrors：`Core-Change:` 段之外的决策号不顶用', () => {
  const [e] = supp('feat: 端口（D-219 讨论过）\n\nCore-Change: D-999', [
    { path: CORE, added: ['pub fn port() {}'], removed: [] },
  ])
  assert.match(e, /打不开/)
})

test('supplementErrors：新增面写死插件 id ⇒ 红', () => {
  const [e] = supp('feat: 端口\n\nCore-Change: D-219', [
    { path: CORE, added: ['if meta == "notes" { return }'], removed: [] },
  ])
  assert.match(e, /不是通用的/)
  assert.match(e, /notes/)
})

test('supplementErrors：新增面写死工具名 ⇒ 红', () => {
  const [e] = supp('feat: 端口\n\nCore-Change: D-219', [
    { path: CORE, added: ['tool: "local/todo_write".into(),'], removed: [] },
  ])
  assert.match(e, /todo_write/)
})

test('supplementErrors：注释里提到插件 id 与工具名 ⇒ 绿（判的是代码）', () => {
  assert.deepEqual(
    supp('feat: 端口\n\nCore-Change: D-219', [
      { path: CORE, added: ['// notes 的 todo_write 曾经在这里分支，已迁走', '/// `"local"` 只是举例', 'pub fn port() {}'], removed: [] },
    ]),
    [],
  )
})

test('supplementErrors：生成物里的插件 id 不算补件定制', () => {
  assert.deepEqual(
    supp('feat: 端口\n\nCore-Change: D-219', [
      { path: 'crates/symbio-plugin-api/src/events.rs', added: ['const X: &str = "notes.recent";'], removed: [] },
      { path: CORE, added: ['pub fn port() {}'], removed: [] },
    ]),
    [],
  )
})

test('supplementErrors：搬家行不算补件定制（D-279）', () => {
  // 机制面收缩时的真实形状：一条带插件 id 的常量（本仓是 `SKILL_TAG = "skill"`）
  // 从 core 搬进 sdk。`--no-renames` 的 diff 说「旧文件全删 + 新文件全增」，
  // 那一行字没变、面也没变宽。夹具里把 id 换成本文件的 `notes`，道理相同。
  const line = 'pub const NOTES_TAG: &str = "notes";'
  const moved = movedLines(
    patchOf([{ path: 'crates/symbio-sdk/src/actors/skill.rs', deleted: true, removed: [line] }]),
  )
  assert.deepEqual(supp('feat: 搬迁\n\nCore-Change: D-219', [{ path: CORE, added: [line], removed: [] }], { moved }), [])
})

test('supplementErrors：搬家豁免只抵一次（同一行新增两处仍红）', () => {
  const line = 'pub const NOTES_TAG: &str = "notes";'
  const moved = movedLines(
    patchOf([{ path: 'crates/symbio-sdk/src/actors/skill.rs', deleted: true, removed: [line] }]),
  )
  const errors = supp(
    'feat: 搬迁\n\nCore-Change: D-219',
    [{ path: CORE, added: [line, line], removed: [] }],
    { moved },
  )
  assert.equal(errors.length, 1)
  assert.match(errors[0], /不是通用的/)
})

test('supplementErrors：从 loop 删掉同一行再加进 sdk ⇒ 仍红（反向拆分，D-247）', () => {
  // 文件没被删掉，只是删了一行：那不是搬家，是把定制通道塞进机制面。
  const line = 'if meta == "notes" { return }'
  const moved = movedLines(patchOf([{ path: LOOP, added: [], removed: [line] }]))
  const [e] = supp('feat: 端口\n\nCore-Change: D-219', [{ path: CORE, added: [line], removed: [] }], { moved })
  assert.match(e, /不是通用的/)
})

test('movedLines：只收「整份文件被删掉」的那些行', () => {
  const moved = movedLines(
    patchOf([
      { path: 'crates/symbio-sdk/src/a.rs', deleted: true, removed: ['pub fn a() {}', ''] },
      { path: LOOP, added: [], removed: ['pub fn b() {}'] },
    ]),
  )
  assert.deepEqual([...moved.keys()], ['pub fn a() {}'])
  assert.equal(moved.get('pub fn a() {}'), 1)
})

// ── 判据起点 ──────────────────────────────────────────────────────────────

test('validateBeforeGuard：起点必须有提交号与理由', () => {
  assert.deepEqual(validateBeforeGuard({ sha: '811b2e7d', why: '判据落地' }), [])
  assert.match(validateBeforeGuard({ sha: 'xyz', why: 'x' })[0], /提交号/)
  assert.match(validateBeforeGuard({ sha: '811b2e7d', why: '  ' })[0], /理由/)
})

test('常量：起点与夹具各自带理由，且不是同一个提交', () => {
  assert.deepEqual(validateBeforeGuard(BEFORE_GUARD), [])
  assert.match(FIXTURE.sha, /^[0-9a-f]{7,40}$/)
  assert.notEqual(FIXTURE.sha, BEFORE_GUARD.sha)
})

// ── 累计付账线（D-372）────────────────────────────────────────────────────

test('supplementRatioReading：欠账按「新增补件 − 新增迁移」算，不按比值算', () => {
  const stock = { supplements: 21, migrations: 15 }
  assert.deepEqual(supplementRatioReading({ supplements: 21, migrations: 15, stock }), {
    dSupp: 0,
    dMig: 0,
    debt: 0,
    ratio: 1.4,
  })
  // 只补不搬 ⇒ 欠账 +1（红）
  assert.equal(supplementRatioReading({ supplements: 22, migrations: 15, stock }).debt, 1)
  // 补一搬一 ⇒ 欠账不变（绿），比值仍高于 1:1 也不假红
  assert.equal(supplementRatioReading({ supplements: 22, migrations: 16, stock }).debt, 0)
  // 搬两补一 ⇒ 提前抵账（绿）
  assert.equal(supplementRatioReading({ supplements: 22, migrations: 17, stock }).debt, -1)
  // 累计比值降到 1:1 以下是另一条读数
  assert.equal(supplementRatioReading({ supplements: 16, migrations: 16, stock }).debt, -6)
})

test('supplementRatioReading：没有迁移时比值为 ∞（不是 NaN）', () => {
  assert.equal(supplementRatioReading({ supplements: 3, migrations: 0, stock: { supplements: 0, migrations: 0 } }).ratio, Infinity)
})

// ── 绝对欠账的期限与台阶（D-375）──────────────────────────────────────────

test('debtAllowance：起算日宽限到本金，检查点当天降档，末点归零', () => {
  const initialDebt = 6
  assert.deepEqual(debtAllowance({ today: '2026-11-07', initialDebt }), {
    today: '2026-11-07',
    allowed: 6,
    next: { by: '2026-11-10', max: 5 },
    deadline: '2026-11-25',
  })
  // 检查点前一天仍按上一档：宽限到日期当天，不是提前判红
  assert.equal(debtAllowance({ today: '2026-11-09', initialDebt }).allowed, 6)
  // 检查点当天降档（日期一过即按新上限判）
  assert.equal(debtAllowance({ today: '2026-11-10', initialDebt }).allowed, 5)
  assert.equal(debtAllowance({ today: '2026-11-22', initialDebt }).allowed, 1)
  // 到期清零，且没有下一台阶
  const atDeadline = debtAllowance({ today: '2026-11-25', initialDebt })
  assert.equal(atDeadline.allowed, 0)
  assert.equal(atDeadline.next, null)
  assert.equal(debtAllowance({ today: '2026-11-26', initialDebt }).allowed, 0)
})

test('validateDebtSchedule：真表合法；平台阶 / 末点非 0 / 日期倒序都判红', () => {
  assert.deepEqual(validateDebtSchedule(DEBT_SCHEDULE, 6), [])
  // 空的台阶 = 没有期限
  assert.ok(validateDebtSchedule({ start: '2026-11-07', checkpoints: [] }, 6).length > 0)
  // 平的台阶 = 绝对线退回增量线
  const flat = {
    start: '2026-11-07',
    checkpoints: [
      { by: '2026-12-31', max: 5 },
      { by: '2027-01-31', max: 5 },
      { by: '2027-06-30', max: 0 },
    ],
  }
  assert.match(validateDebtSchedule(flat, 6).join('；'), /至少降 1/)
  // 末点不归零 = 到期还能欠
  const noZero = { start: '2026-11-07', checkpoints: [{ by: '2027-06-30', max: 1 }] }
  assert.match(validateDebtSchedule(noZero, 6).join('；'), /压到 0/)
  // 日期不递增
  const back = {
    start: '2026-11-07',
    checkpoints: [
      { by: '2027-01-31', max: 5 },
      { by: '2026-12-31', max: 0 },
    ],
  }
  assert.match(validateDebtSchedule(back, 6).join('；'), /严格递增/)
})

// ── 到期豁免（D-403）──────────────────────────────────────────────────────

test('activeDebtExemption：期限当天仍有效、次日失效', () => {
  const decisions = new Set(['D-403'])
  const entry = { decision: 'D-403', through: '2026-11-12', reason: '欠账 7 来自 T-053 待搬' }
  assert.equal(activeDebtExemption({ today: '2026-11-09', decisions, exemptions: [entry] }), entry)
  assert.equal(activeDebtExemption({ today: '2026-11-12', decisions, exemptions: [entry] }), entry)
  assert.equal(activeDebtExemption({ today: '2026-11-13', decisions, exemptions: [entry] }), null)
})

test('activeDebtExemption：决策号打不开或理由为空 ⇒ 不生效（匿名豁免等于没写）', () => {
  const entry = { decision: 'D-403', through: '2026-11-12', reason: '欠账 7 来自 T-053 待搬' }
  assert.equal(activeDebtExemption({ today: '2026-11-09', decisions: new Set(), exemptions: [entry] }), null)
  assert.equal(
    activeDebtExemption({
      today: '2026-11-09',
      decisions: new Set(['D-403']),
      exemptions: [{ ...entry, reason: '  ' }],
    }),
    null,
  )
})

test('DEBT_EXEMPTIONS：真仓条目有决策号 / 可读日期 / 非空理由', () => {
  assert.ok(DEBT_EXEMPTIONS.length >= 1)
  for (const e of DEBT_EXEMPTIONS) {
    assert.match(e.decision, /^D-\d+$/)
    assert.match(e.through, /^\d{4}-\d{2}-\d{2}$/)
    assert.ok(e.reason.trim().length >= 8)
  }
})

// ── 真仓冒烟 ──────────────────────────────────────────────────────────────

test('真仓冒烟：夹具读得出、--last 跑绿且点名夹具', () => {
  const r = spawnSync(process.execPath, [script, '--last'], { cwd: repoRoot, encoding: 'utf8', shell: false })
  assert.equal(r.status, 0, r.stderr || r.stdout)
  assert.match(r.stdout, new RegExp(FIXTURE.sha.slice(0, 8)))
  assert.match(r.stdout, /全部只搬运/)
  // 期限线必须每轮打印（不手抄数字）：欠账 / 今日允许 / 下一台阶。
  // 欠账可为负：`--last` 只看最后一笔时，若它本身是迁移，净补件为 −1 —— 输出照实打印。
  assert.match(r.stdout, /累计欠账 -?\d+ 笔/)
  assert.match(r.stdout, /今日（\d{4}-\d{2}-\d{2}）允许 ≤\d+/)
})

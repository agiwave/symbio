/**
 * capability-migration-audit 回归测试 —— 证明 EX-003 / EX-007 **改输入后结论必须变**
 *
 * 这条守卫的失效形态都是「看起来在判」：
 *
 * - 有 `Migration:` 就放行 ⇒ 写上 trailer 即可在架构面夹带，D-247 的两类分家作废；
 * - 只判新增、不判删除 ⇒ 插件落地而旧实现留着（两份同时生效）照样绿；
 * - 只认 `*.gen.rs` ⇒ `plugin-api` 的 `view_names.rs` / `events.rs` / `routes.rs` 被当成
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
  FIXTURE,
  classifyCommit,
  generatedPaths,
  isCapabilitySource,
  isGenerated,
  migrationErrors,
  parsePatch,
  supplementErrors,
  validateBeforeGuard,
} from './capability-migration-audit.mjs'

const script = fileURLToPath(new URL('./capability-migration-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(script), '..')

const CORE = 'crates/symbio-core/src/a.rs'
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

test('parsePatch：删除文件按旧路径记账', () => {
  const p = patchOf([{ path: LOOP, deleted: true, removed: ['row!();'] }])
  assert.deepEqual(p, [{ path: LOOP, added: [], removed: ['row!();'] }])
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

test('generatedPaths：描述符声明的输出都在清单里（含不带 .gen.rs 后缀的三份）', () => {
  const g = generatedPaths(repoRoot)
  for (const p of [
    'crates/symbio-plugin-api/src/view_names.rs',
    'crates/symbio-plugin-api/src/events.rs',
    'crates/symbio-plugin-api/src/routes.rs',
    'crates/symbio-core/src/authz/rows.gen.rs',
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
  assert.equal(isCapabilitySource('crates/symbio-core/src/authz/rows.gen.rs'), false)
  assert.equal(isCapabilitySource('crates/symbio-plugin-api/src/view_names.rs'), false)
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

test('migrationErrors：迁移提交改 core ⇒ EX-007 红并点名 core', () => {
  const [e] = mig([
    { path: CORE, added: ['pub fn helper() {}'], removed: [] },
    { path: LOOP, added: [], removed: ['row!("memory")'] },
  ])
  assert.match(e, /EX-007/)
  assert.match(e, /crates\/symbio-core/)
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

test('migrationErrors：迁移中重生成 view_names.rs 不算碰机制面', () => {
  assert.deepEqual(
    mig([
      { path: 'crates/symbio-plugin-api/src/view_names.rs', added: ['const X: &str = "notes.recent";'], removed: [] },
      { path: LOOP, added: [], removed: ['row!("memory")'] },
    ]),
    [],
  )
})

test('migrationErrors：迁移中改测试与 .gen.rs 不判（断言与镜像跟着搬）', () => {
  assert.deepEqual(
    mig([
      { path: 'crates/symbio-plugin-loop/src/loop_rows.test.rs', added: ['#[test]'], removed: [] },
      { path: 'crates/symbio-core/src/authz/rows.gen.rs', added: ['X', 'Y'], removed: ['Z'] },
      { path: LOOP, added: [], removed: ['row!("memory")'] },
    ]),
    [],
  )
})

// ── 补件 ──────────────────────────────────────────────────────────────────

const supp = (message, files) =>
  supplementErrors({
    sha: 'x',
    message,
    patch: patchOf(files),
    decisions,
    pluginIds: ['notes', 'local'],
    toolNames: ['todo_write', 'content_search'],
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
      { path: 'crates/symbio-plugin-api/src/view_names.rs', added: ['const X: &str = "notes.recent";'], removed: [] },
      { path: CORE, added: ['pub fn port() {}'], removed: [] },
    ]),
    [],
  )
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

// ── 真仓冒烟 ──────────────────────────────────────────────────────────────

test('真仓冒烟：夹具读得出、--last 跑绿且点名夹具', () => {
  const r = spawnSync(process.execPath, [script, '--last'], { cwd: repoRoot, encoding: 'utf8', shell: false })
  assert.equal(r.status, 0, r.stderr || r.stdout)
  assert.match(r.stdout, new RegExp(FIXTURE.sha.slice(0, 8)))
  assert.match(r.stdout, /全部只搬运/)
})

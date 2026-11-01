// `gate.d/_scope.mjs` 的回归测试。
//
// ## 为什么这一份特别要紧
//
// 它是**唯一能决定「某个阶段这一轮不跑」的模块**，而它判错的两种方向代价极不对称：
//
//   - 判成「改了」而其实没改 ⇒ 只是白跑几分钟（可接受，且范围求值取的是保守方向）；
//   - 判成「没改」而其实改了 ⇒ **整轮门禁 40 秒绿着跑完，什么都没检查**。这是本仓
//     反复出现的那一类失效（`D-224`：判据产出零项与「跑过且绿」同形），只不过这次
//     伪装成「优化」——它让门禁**更快**，而更快本身是没人会怀疑的信号。
//
// 所以这里钉的不是「省了多少秒」，是**四条不许退让的性质**：
//   ① 拿不到范围 ⇒ `kind: 'all'`（全跑），不是空集；
//   ② 显式 `--base` 拼错 ⇒ 全跑，不是静默换成别的基准；
//   ③ 工作区**未提交**的改动必须被算进范围（含中文路径与带空格的路径）；
//   ④ 门禁自己（`scripts/gate.mjs` / `scripts/gate.d/`）变了 ⇒ `forceAll`。
//
// 每条都注入一次坏形态来证伪：如果哪天有人把 `git status -z` 换成默认格式、或者把
// 「解析失败」当成「没有改动」，这里必须先红。
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import assert from 'node:assert/strict'

import { resolveScope, skipReason, ALL_SURFACES } from './_scope.mjs'

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'gate-scope-'))

function git(cwd, args) {
  const r = spawnSync('git', args, { cwd, encoding: 'utf8', shell: false })
  assert.equal(r.status, 0, `git ${args.join(' ')} 失败：${r.stderr}`)
  return r.stdout
}

/** 建一个有初始提交的临时仓（`origin/v2` 指在初始提交上，模拟「远端主分支」）。 */
function repoWith({ files = { 'README.md': '# x\n' } } = {}) {
  const root = tmp()
  git(root, ['init', '-q', '-b', 'v2'])
  git(root, ['config', 'user.email', 't@example.com'])
  git(root, ['config', 'user.name', 't'])
  git(root, ['config', 'commit.gpgsign', 'false'])
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, body)
  }
  git(root, ['add', '-A'])
  git(root, ['commit', '-q', '-m', 'init'])
  git(root, ['branch', 'origin/v2'])
  return root
}

function commit(root, files, msg = 'c') {
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, body)
  }
  git(root, ['add', '-A'])
  git(root, ['commit', '-q', '-m', msg])
}

test('没有基准可用 ⇒ 全跑（不是「什么都没改」）', () => {
  const root = tmp()
  git(root, ['init', '-q'])
  const scope = resolveScope({ repoRoot: root })
  assert.equal(scope.kind, 'all')
  assert.equal(scope.hits(['rust']), true)
  assert.equal(scope.hits(['scripts']), true)
})

test('不在 git 仓里 ⇒ 全跑', () => {
  const root = tmp()
  const scope = resolveScope({ repoRoot: root })
  assert.equal(scope.kind, 'all')
  assert.equal(scope.hits(['frontend']), true)
})

test('显式 --base 拼错 ⇒ 全跑，而不是静默换一个基准', () => {
  const root = repoWith()
  commit(root, { 'docs/a.md': '# a\n' })
  const scope = resolveScope({ repoRoot: root, base: 'no-such-ref' })
  assert.equal(scope.kind, 'all', '拼错的基准必须退化成全跑')
  assert.match(scope.reason, /解析不出来/)
})

test('只有 docs 改动 ⇒ 命中 docs、不命中 rust/e2e/scripts', () => {
  const root = repoWith()
  commit(root, { 'docs/003-design/001-x.md': '# x\n' })
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.kind, 'diff')
  assert.deepEqual(scope.surfaces, ['docs'])
  assert.equal(scope.hits(['rust']), false)
  assert.equal(scope.hits(['e2e']), false)
  assert.equal(scope.hits(['scripts']), false)
  assert.equal(scope.hits(['docs']), true)
  assert.equal(scope.hits([]), true, '没声明 needs 的阶段恒跑')
})

test('crates 下的清单与源码归 rust；tauri 前端源归 frontend', () => {
  const root = repoWith()
  commit(root, {
    'crates/symbio-sdk/src/lib.rs': 'pub fn a() {}\n',
    'crates/symbio-plugin-memory/PLUGIN.yml': 'id: memory\n',
    'tauri/src/App.vue': '<template/>\n',
  })
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.hits(['rust']), true, 'PLUGIN.yml 在运行时被读 ⇒ 算 rust')
  assert.equal(scope.hits(['frontend']), true)
  assert.equal(scope.hits(['docs']), false)
})

test('crates 下的 README 只算 docs，不许把整个 cargo 链拽起来', () => {
  const root = repoWith()
  commit(root, { 'crates/symbio-sdk/README.md': '# x\n' })
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.hits(['rust']), false)
  assert.equal(scope.hits(['docs']), true)
})

test('工作区未提交改动必须看见——含中文路径与带空格的路径', () => {
  const root = repoWith()
  fs.mkdirSync(path.join(root, 'docs', '说明 目录'), { recursive: true })
  fs.writeFileSync(path.join(root, 'docs', '说明 目录', '一个 文档.md'), '# y\n')
  // `git status` 的默认格式会给这类路径加引号并转义，按行切就全落空；
  // 那正好是「改了却判成没改」⇒ 假绿。
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.kind, 'diff')
  assert.ok(scope.files.includes('docs/说明 目录/一个 文档.md'), `实际：${JSON.stringify(scope.files)}`)
  assert.equal(scope.hits(['docs']), true)
})

test('未跟踪文件也算改动（新增的守卫脚本不许被当成没改）', () => {
  const root = repoWith()
  fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(root, 'scripts', 'new-audit.mjs'), '// x\n')
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.hits(['scripts']), true)
})

test('门禁自身变了 ⇒ forceAll（改门禁的那一次必须按新门禁全跑）', () => {
  const root = repoWith({ files: { 'scripts/gate.mjs': '// gate\n' } })
  fs.writeFileSync(path.join(root, 'scripts', 'gate.mjs'), '// gate v2\n')
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.forceAll, true)
  assert.equal(scope.hits(['rust']), true)
  assert.equal(scope.hits(['frontend']), true)
})

test('--full ⇒ 全跑，且不依赖 git', () => {
  const root = tmp()
  const scope = resolveScope({ repoRoot: root, full: true })
  assert.equal(scope.kind, 'all')
  assert.deepEqual(scope.surfaces, [...ALL_SURFACES].sort())
  assert.equal(scope.hits(['rust']), true)
})

test('没有改动 ⇒ 空集，但仍是一次明确结论（不是「不可知」）', () => {
  const root = repoWith()
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  assert.equal(scope.kind, 'diff')
  assert.deepEqual(scope.files, [])
  assert.equal(scope.hits(['rust']), false, '确实什么都没改 ⇒ 每一面都不命中')
  assert.equal(scope.brief(), '无改动')
})

test('skipReason 必须同时说出「哪个面」与「怎么强制全跑」', () => {
  const root = repoWith()
  commit(root, { 'docs/a.md': '# a\n' })
  const scope = resolveScope({ repoRoot: root, base: 'origin/v2' })
  const reason = skipReason(scope, ['rust'])
  assert.match(reason, /rust/)
  assert.match(reason, /--full/)
})

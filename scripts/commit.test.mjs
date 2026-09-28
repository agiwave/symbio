// `scripts/commit.mjs` 的回归测试。
//
// 这个脚本的失效方式与守卫相反：它**不会亮假绿灯，而是卡住**——默认交互时在
// 无 TTY 的调用方（子进程 / CI / 自动化）里挂在 `readline.question` 上，
// 表现为「命令跑不完」而不是「命令报错」。三件事因此值得钉住：
//
//   1. **默认非交互**：无 `--interactive` 时绝不读 stdin（读不到 TTY 会挂）；
//   2. **默认不跑门禁**：提交与门禁解耦——耦合时提交要等几分钟，且门禁的
//      autoWork 会在提交过程中改写索引；
//   3. **推断可用**：type / scope / 标题 / 分节从暂存内容得出，不留空壳默认值
//      （`type=chore` + `标题不能为空` 这种「能跑但没用」的结果同样是失效）。
//
// 用真实临时 git 仓库而非打桩：被测行为全部建立在 `git diff --cached` 的语义上。
//
// 跑法：node --test scripts/commit.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'

const repoRoot = path.resolve(import.meta.dirname, '..')
const commitScript = path.join(repoRoot, 'scripts', 'commit.mjs')

// 每个用例一个独立仓库：`commit.mjs` 会读索引，共用一个仓库必然互相污染。
function makeRepo() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-commit-'))
  const git = (args, opts = {}) =>
    spawnSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024, ...opts })
  git(['-c', 'init.defaultBranch=main', 'init', '-q'])
  git(['config', 'user.email', 'test@example.com'])
  git(['config', 'user.name', 'test'])
  git(['config', 'core.autocrlf', 'false'])
  fs.writeFileSync(path.join(root, 'README.md'), '# init\n')
  git(['add', '.'])
  git(['commit', '-q', '-m', 'init'])
  return { root, git }
}

/** 跑 commit.mjs：**stdin 显式给 'ignore'**——真交互时子进程会挂，测试要能暴露它。 */
function runCommit(root, args = []) {
  return spawnSync(process.execPath, [commitScript, ...args], {
    cwd: root,
    encoding: 'utf8',
    timeout: 60_000,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
}

const repos = []
function repo() {
  const r = makeRepo()
  repos.push(r.root)
  return r
}
after(() => {
  for (const r of repos) fs.rmSync(r, { recursive: true, force: true })
})

// ================= 1. 默认非交互 =================

test('★ 无 --interactive 时不读 stdin：挂起的交互会让自动化卡死', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'a.md'), 'x\n')
  git(['add', 'a.md'])

  // stdio[0]='ignore' ⇒ 任何 readline.question 都拿不到输入、必然挂到 timeout。
  // 默认非交互则应当在 timeout 前正常走完。
  const r = runCommit(root, ['--dry-run'])
  assert.notEqual(r.error?.code, 'ETIMEDOUT', '默认模式读了 stdin ⇒ 无 TTY 的调用方会卡死')
  assert.equal(r.status, 0, `应正常结束：${r.stdout}\n${r.stderr}`)
})

test('--interactive 是显式选择：有它才允许逐项询问', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'b.md'), 'x\n')
  git(['add', 'b.md'])

  // 给了 --interactive 且 stdin 关闭 ⇒ 停在询问上是**预期**行为（timeout 或非 0）。
  // 这条钉住「开关真的接上了」，而不是接了个永远不生效的假开关。
  const r = runCommit(root, ['--dry-run', '--interactive'])
  assert.ok(
    r.error?.code === 'ETIMEDOUT' || r.status !== 0,
    '--interactive 未生效（它应当去问问题）',
  )
})

// ================= 2. 默认不跑门禁 =================

test('★ 默认不跑门禁：提交不该等门禁，也不该让 autoWork 改写索引', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'c.md'), 'x\n')
  git(['add', 'c.md'])

  const r = runCommit(root, ['--dry-run'])
  assert.equal(r.status, 0, `应正常结束：${r.stdout}\n${r.stderr}`)
  // 门禁会打印阶段表头；默认路径不该出现它。
  assert.ok(!/── 阶段 \d\/\d/.test(r.stdout), `默认跑了门禁：\n${r.stdout}`)
  assert.match(r.stdout, /门禁：本次跳过/, 'gateSummary 应明示跳过')
})

test('--gate 显式要求时才进入门禁阶段（真仓库里会红，重点是「它开始跑了」）', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'd.md'), 'x\n')
  git(['add', 'd.md'])

  // 临时仓库不是 symbio：门禁必然失败/找不到阶段，但只要它**开始跑**，
  // 输出里就会出现阶段表头或门禁产物（.workbuddy-ai/）。
  const r = runCommit(root, ['--dry-run', '--gate'])
  const started = /── 阶段 \d\/\d/.test(r.stdout) || fs.existsSync(path.join(root, '.workbuddy-ai'))
  assert.ok(started, `--gate 没有真的去跑门禁：\n${r.stdout}\n${r.stderr}`)
})

// ================= 3. 推断可用 =================

test('★ type 从路径推断：docs-only 改动判 docs，不留 chore 空壳', () => {
  const { root, git } = repo()
  fs.mkdirSync(path.join(root, 'docs'), { recursive: true })
  fs.writeFileSync(path.join(root, 'docs', 'guide.md'), 'x\n')
  git(['add', 'docs/guide.md'])

  const r = runCommit(root, ['--dry-run'])
  assert.match(r.stdout, /^docs\(/m, `md-only 改动应判 docs：\n${r.stdout}`)
})

test('★ 标题不为空：不留「标题不能为空」以外的空壳，也不要求人补', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'e.md'), 'x\n')
  git(['add', 'e.md'])

  const r = runCommit(root, ['--dry-run'])
  assert.equal(r.status, 0, `不应因缺标题而失败：${r.stdout}\n${r.stderr}`)
  // 消息首行形如 `<type>(<scope>): <标题>`，冒号后必须有内容
  const firstLine = r.stdout.split('\n').find((l) => /^[a-z]+\([^)]+\): /.test(l))
  assert.ok(firstLine, `没找到消息标题行：\n${r.stdout}`)
  assert.ok(firstLine.replace(/^[a-z]+\([^)]+\): /, '').trim().length > 0, '标题为空')
})

test('★ 标题维度统一：不把「目录」和「文件名」混在同一句里', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'CONTRIBUTING.md'), 'x\n')
  fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(root, 'scripts', 'a.mjs'), 'y\n')
  fs.writeFileSync(path.join(root, 'scripts', 'b.mjs'), 'z\n')
  git(['add', '.'])

  const r = runCommit(root, ['--dry-run'])
  const title = r.stdout.split('\n').find((l) => /^[a-z]+\([^)]+\): /.test(l)) ?? ''
  // 旧实现把无目录的根文件按**文件名**入列 ⇒ 出现 `scripts/ 2 文件、CONTRIBUTING.md 1 文件`：
  // 同一句里一半是目录一半是文件，像是两套分类混在一起。
  assert.ok(!/[a-z0-9-]+\.md \d+ 文件/.test(title), `标题混入了文件名：${title}`)
  assert.match(title, /scripts\/\d+ 文件/, `应报出目录维度：${title}`)
  assert.match(title, /\(根\) \d+ 文件/, `根文件应收进统一的「(根)」维度：${title}`)
})

test('★ 分节自动归纳：未给 --section 时正文不是「留空」占位', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'f1.md'), 'x\n')
  fs.writeFileSync(path.join(root, 'f2.md'), 'y\n')
  git(['add', 'f1.md', 'f2.md'])

  const r = runCommit(root, ['--dry-run'])
  assert.ok(!/正文分节留空/.test(r.stdout), `仍落了空壳分节：\n${r.stdout}`)
  assert.match(r.stdout, /文件（\+\d+\/-\d+）/, `分节应含增删行数：\n${r.stdout}`)
})

test('★ 分节按顶层目录聚合：不把同目录文件拆成并列条目', () => {
  const { root, git } = repo()
  fs.mkdirSync(path.join(root, 'scripts', 'gate.d'), { recursive: true })
  fs.writeFileSync(path.join(root, 'scripts', 'a.mjs'), 'x\n')
  fs.writeFileSync(path.join(root, 'scripts', 'gate.d', 'b.mjs'), 'y\n')
  fs.writeFileSync(path.join(root, 'scripts', 'gate.d', 'c.mjs'), 'z\n')
  git(['add', '.'])

  const r = runCommit(root, ['--dry-run'])
  const body = r.stdout
  // 旧实现按 `slice(0, 2)` 切片 ⇒ 出现 `scripts/gate.d：2 文件` 与 `scripts/a.mjs：1 文件`
  // 两条**并列同级**条目，读起来像文件清单而非归纳。
  assert.match(body, /scripts\/（[^）]*）：3 文件/, `分节未按顶层聚合：\n${body}`)
  assert.ok(!/（?scripts\/gate\.d）：\d+ 文件/.test(body), `同目录文件仍被拆成独立条目：\n${body}`)
})

test('★ 根文件在分节里报出文件名：`(根)` 自身没有信息量', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'CHANGELOG.md'), 'x\n')
  fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(root, 'scripts', 't.mjs'), 'y\n')
  git(['add', '.'])

  const r = runCommit(root, ['--dry-run'])
  assert.match(r.stdout, /\(根\)（CHANGELOG\.md）：1 文件/, `根文件未报名字：\n${r.stdout}`)
})

test('★ 工具改动夹带根级文档仍判 chore：不掉进 refactor 的语义黑洞', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'CONTRIBUTING.md'), 'x\n')
  fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(root, 'scripts', 'tools.mjs'), 'y\n')
  git(['add', '.'])

  const r = runCommit(root, ['--dry-run'])
  // 「改工具 + 补一句说明」是常规动作。若严格要求全部落在 scripts/，
  // 这类提交会掉进 refactor，与「重构产品代码」混为一谈。
  assert.match(r.stdout, /^chore\(/m, `工具+根文档应判 chore：\n${r.stdout}`)
})

test('★ 新增回归测试时判 test：补测试与改机制在历史里应可区分', () => {
  const { root, git } = repo()
  fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(root, 'scripts', 'foo.test.mjs'), 'x\n')
  git(['add', '.'])

  const r = runCommit(root, ['--dry-run'])
  assert.match(r.stdout, /^test\(/m, `新增 *.test.mjs 应判 test：\n${r.stdout}`)
})

test('显式 --title / --section 覆盖推断（作者要写「为什么」时的出口）', () => {
  const { root, git } = repo()
  fs.writeFileSync(path.join(root, 'g.md'), 'x\n')
  git(['add', 'g.md'])

  const r = runCommit(root, [
    '--dry-run',
    '--type=feat',
    '--scope=core',
    '--title=显式标题不许被推断覆盖',
    '--section=显式分节同样优先',
  ])
  assert.equal(r.status, 0, `应正常结束：${r.stdout}\n${r.stderr}`)
  assert.match(r.stdout, /^feat\(core\): 显式标题不许被推断覆盖$/m, '显式标题被覆盖了')
  assert.match(r.stdout, /显式分节同样优先/, '显式分节被覆盖了')
})

test('索引为空时明确报错（而不是跑完门禁再被 git 拒掉）', () => {
  const { root } = repo()
  const r = runCommit(root)
  assert.notEqual(r.status, 0, '空索引应当失败')
  // 报错走 stderr（die 用 console.error）——两路合起来查。
  assert.match(r.stdout + r.stderr, /没有可提交的改动|索引为空/)
})

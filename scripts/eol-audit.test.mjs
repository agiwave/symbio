/**
 * eol-audit 回归测试
 *
 * 这份守卫判的是**判据的输入前提**（D-142），它自己的失效形态比别的守卫更隐蔽：
 * 判据读的是 `git ls-files` / `git check-attr`，一旦这两个命令的解析写错，
 * 结果不是"报错"而是**一条都不命中**——一条永远不红的行尾守卫，看起来和
 * 「仓里行尾很干净」完全一样。所以三条码各配一条反向用例，另加两条边界：
 * 二进制不判（不该逼人写豁免）、不是 git 仓库时**必须红**（判不了不许亮绿灯）。
 *
 * 跑法：node --test scripts/eol-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./eol-audit.mjs', import.meta.url))

/** 在临时目录造一个**真的 git 仓库**：行尾判定只能由 git 回答，夹具不能是假装的 */
function repo(files = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'eol-audit-'))
  const git = (args, cwd = root) => {
    const r = spawnSync('git', args, { cwd, encoding: 'utf8' })
    assert.equal(r.status, 0, `git ${args.join(' ')} 失败：${r.stderr}`)
    return r
  }
  git(['init', '-q'])
  // 夹具必须关掉 autocrlf：本机的全局配置不该决定「入库时会不会被归一化」，
  // 否则同一个用例在 Windows 与 Linux 上结论不同（本仓 `.gitattributes` 头注释
  // 记的正是这个坑）。
  git(['config', 'core.autocrlf', 'false'])
  git(['config', 'user.email', 'test@example.com'])
  git(['config', 'user.name', 'test'])
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return { root, git, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  assert.ifError(r.error)
  return r
}

/** 造一个已暂存的仓库并跑判定（多数用例的形状：写文件 → git add → 判） */
function audit(files, { beforeRun } = {}) {
  const r = repo(files)
  try {
    r.git(['add', '-A'])
    if (beforeRun) beforeRun(r)
    return run(r.root)
  } finally {
    r.cleanup()
  }
}

const ATTR = '* text=auto eol=lf\n'

test('干净仓库（声明 lf、工作树 lf）→ 通过', () => {
  const r = audit({ '.gitattributes': ATTR, 'a.md': '# A\n', 'b.ts': 'export const x = 1\n' })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /eol-audit 通过/)
})

test('EOL-001：声明了 lf，工作树却是 CRLF → 红（`git diff` 对此一个字符都不显示）', () => {
  const r = audit({ '.gitattributes': ATTR, 'a.md': '# A\r\n\r\n正文\r\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /EOL-001/)
  assert.match(r.stdout, /a\.md/)
  // 反向确认：这条红的**不是**别的两面——入库 blob 已被 git 归一化成 LF
  assert.match(r.stdout, /EOL-003 入库 blob：带 \\r 0 个/)
})

test('EOL-002：文本文件没有任何 `eol` 声明 → 红（新类型从声明网里漏掉）', () => {
  // 整个仓库**没有** `.gitattributes`：attr 为 `unspecified`，行尾从此「看谁先写」。
  // 这正是新类型进来时的形状——没人会记得去补那一条声明。
  const r = audit({ 'b.weird': 'hello\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /EOL-002/)
  assert.match(r.stdout, /b\.weird/)
})

test('EOL-003：先提交 CRLF、后加 `eol=lf` 声明 → 入库 blob 仍带 `\\r`，红', () => {
  const r = repo({ 'c.md': '# C\r\n' })
  try {
    r.git(['add', '-A'])
    r.git(['commit', '-q', '-m', 'no attrs yet'])
    // 声明来得比提交晚——这正是 `.gitattributes` 头注释记的那次事故形状：
    // 声明之后 git 才会在下次 add 时归一化，已入库的 blob 不会自己变。
    fs.writeFileSync(path.join(r.root, '.gitattributes'), ATTR)
    r.git(['add', '.gitattributes'])
    const out = run(r.root)
    assert.equal(out.status, 1, out.stdout)
    assert.match(out.stdout, /EOL-003/)
    assert.match(out.stdout, /c\.md/)
  } finally {
    r.cleanup()
  }
})

test('二进制不判：前 8KB 含 NUL 的文件既不算未声明也不算 CRLF（不逼人写豁免）', () => {
  const r = audit({ '.gitattributes': ATTR, 'logo.bin': Buffer.from([0x89, 0x50, 0x00, 0x0d, 0x0a]) })
  assert.equal(r.status, 0, r.stdout)
  // 受版本控制 2 个、其中只有 `.gitattributes` 算文本——二进制被内容判据挡在统计外
  assert.match(r.stdout, /1 个文本文件（受版本控制 2 个）· 带 \\r 0 个/)
})

test('不是 git 仓库 → 红（判不了的判据不许亮绿灯）', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'eol-audit-norepo-'))
  try {
    fs.writeFileSync(path.join(dir, 'a.md'), '# A\n')
    const out = run(dir)
    assert.equal(out.status, 1, out.stdout + out.stderr)
    assert.match(out.stderr, /无法运行/)
  } finally {
    fs.rmSync(dir, { recursive: true, force: true })
  }
})

test('真实仓库当前状态通过（防本守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 60_000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout)
})

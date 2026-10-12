// `gen-doc-refs` 的回归测试（引用只写编号、地址由机制生成/更新：RF-001 / RF-002）。
//
// 这套机制的失效形态有两种，各自必须钉住：
//   ① **该改的不改**（地址漂移不被重算）⇒ 文档搬家后入链静默失效，机制退化成装饰；
//   ② **不该改的乱改**（把外链 / 非编号标签的链接也重写）⇒ 守卫开始改坏正文，最后被人整体关掉。
// 另加两条最容易被忽略的：**未登记编号必须红且不写盘**（改一半 = 两份半截状态）、
// **幂等**（第二次运行零改动——否则门禁的 autoWork 天天重写同一批文件）。
//
// 跑法：node --test scripts/gen-doc-refs.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { stripAnsi } from './color.mjs'
import { runGenDocRefs } from './gen-doc-refs.mjs'

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-refs-'))
let caseNo = 0
after(() => {
  try {
    fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3 })
  } catch {
    /* 沙箱的 safe-delete 垫片可能拒删，交 OS 回收 */
  }
})

const fm = (meta) =>
  ['---', ...Object.entries(meta).map(([k, v]) => `${k}: ${v}`), '---', ''].join('\n')

/** 台账类条目（住 docs/<kind>/，编号 = 文件名） */
const taskDoc = (id, body = '## 内容\n说明。\n') =>
  fm({ kind: 'task', id, title: `${id} 任务`, status: 'todo', created: '2026-10-11', track: 'meta' }) + body

/** 文档类（在位存放，编号 = 仓根相对路径，title = 首个 H1） */
const planDoc = (rel, body) =>
  fm({ kind: 'plan', id: rel.replace(/\.md$/, ''), title: (body.match(/^#\s+(.+)$/m) || [])[1] ?? 'x', status: 'active', created: '2026-10-11' }) + body

/** 一座夹具仓：键为仓根相对路径 */
function caseRoot(files) {
  const base = path.join(tmp, `case${++caseNo}`)
  for (const [rel, content] of Object.entries(files)) {
    const p = path.join(base, rel)
    fs.mkdirSync(path.dirname(p), { recursive: true })
    fs.writeFileSync(p, content)
  }
  return base
}

const read = (base, rel) => fs.readFileSync(path.join(base, rel), 'utf8')
const out = (r) => stripAnsi(r.lines.join('\n'))

test('记号展开：`[[编号]]` → `[编号](相对地址)`，锚点原样带上（反向：第二次零改动=幂等）', () => {
  const base = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [[T-001]] 与 [[T-001#处理过程]]。\n'),
  })
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 0, out(r))
  const text = read(base, 'docs/plan/x.md')
  assert.match(text, /见 \[T-001\]\(\.\.\/task\/T-001\.md\) 与 \[T-001\]\(\.\.\/task\/T-001\.md#处理过程\)。/)
  assert.equal(runGenDocRefs([`--root=${base}`]).changed.length, 0, '第二次必须零改动（幂等）')
})

test('地址重算（RF-002）：标签是编号、地址漂移 ⇒ 自愈；已解析到同一文件的等价写法不动', () => {
  const base = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [T-001](../task/OLD.md) 和 [`T-001`](../task/T-001.md)。\n'),
  })
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 0, out(r))
  const text = read(base, 'docs/plan/x.md')
  assert.match(text, /\[T-001\]\(\.\.\/task\/T-001\.md\) 和 \[`T-001`\]\(\.\.\/task\/T-001\.md\)/, '漂移地址被重算，带反引号的等价写法也认')
  assert.equal(r.changed.length, 1)
})

test('RF-001：未登记的编号引用 ⇒ 红，且**一个文件都不改**（改一半 = 两份半截状态）', () => {
  const base = caseRoot({
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [[T-999]]。\n'),
  })
  const before = read(base, 'docs/plan/x.md')
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 1)
  assert.match(out(r), /docs\/plan\/x\.md：RF-001 引用了未登记的编号 `\[\[T-999\]\]`/)
  assert.equal(read(base, 'docs/plan/x.md'), before, '有未登记编号时不得写盘')
})

test('外链 / 纯锚点 / 指向非登记文件 / 围栏里的链接一律不动（反向）', () => {
  const body = [
    '# X',
    '',
    '外链 [T-001](https://example.com/T-001) · 锚点 [T-001](#x) · 代码 [工具](../scripts/tool.mjs)',
    '',
    '```md',
    '见 [T-001](../task/OLD.md)',
    '```',
    '',
  ].join('\n')
  const base = caseRoot({ 'docs/task/T-001.md': taskDoc('T-001'), 'docs/plan/x.md': planDoc('docs/plan/x.md', body) })
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 0, out(r))
  assert.equal(r.changed.length, 0, '这些都不该被改')
  assert.match(read(base, 'docs/plan/x.md'), /https:\/\/example\.com\/T-001/)
})

test('带显示文字的引用：`[[编号|文字]]` → `[文字](地址 "编号")`（编号进标题=身份载体）', () => {
  const base = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [[T-001|接管协议那一条]] 与 [[T-001#处理过程|过程]]。\n'),
  })
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 0, out(r))
  const text = read(base, 'docs/plan/x.md')
  assert.match(text, /见 \[接管协议那一条\]\(\.\.\/task\/T-001\.md "T-001"\) 与 \[过程\]\(\.\.\/task\/T-001\.md#处理过程 "T-001"\)/)
  assert.equal(runGenDocRefs([`--root=${base}`]).changed.length, 0, '第二次零改动（幂等）')
})

test('标题载体的地址重算（RF-002）：标签是自由文字、地址漂移 ⇒ 自愈且标题保留', () => {
  const base = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [接管那一条](../task/OLD.md "T-001")。\n'),
  })
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 0, out(r))
  assert.match(read(base, 'docs/plan/x.md'), /\[接管那一条\]\(\.\.\/task\/T-001\.md "T-001"\)/)
})

test('RF-003：指向已登记文档的手工地址（无编号标签 / 无编号标题）⇒ 红且不写盘', () => {
  const base = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [接管协议](../task/T-001.md)。\n'),
  })
  const before = read(base, 'docs/plan/x.md')
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 1)
  assert.match(out(r), /RF-003 手工地址 `\[接管协议\]\(\.\.\/task\/T-001\.md\)` 指向已登记文档 `T-001`/)
  assert.equal(read(base, 'docs/plan/x.md'), before, '有 RF-003 时不得写盘')
  // 例外：**自指**地址随文件一起搬，不判红（否则每篇引用自己小节的文档都得改）
  const self = caseRoot({
    'docs/plan/y.md': planDoc('docs/plan/y.md', '# Y\n\n见 [本章](y.md)。\n'),
  })
  const r2 = runGenDocRefs([`--root=${self}`])
  assert.equal(r2.code, 0, out(r2))
  assert.equal(r2.changed.length, 0)
})

test('行内代码段是字面量：里面的记号与链接一律不动（机制文档要能写 `[[编号]]`）', () => {
  const body = [
    '# X',
    '',
    '写法是 `[[T-001]]`，展开后成 `[T-001](../task/OLD.md)`。',
    '',
    '段外的真引用自当照改：见 [T-001](../task/OLD.md)。',
    '',
  ].join('\n')
  const base = caseRoot({ 'docs/task/T-001.md': taskDoc('T-001'), 'docs/plan/x.md': planDoc('docs/plan/x.md', body) })
  const r = runGenDocRefs([`--root=${base}`])
  assert.equal(r.code, 0, out(r))
  const text = read(base, 'docs/plan/x.md')
  assert.match(text, /`\[\[T-001\]\]`/, '段内的记号不得被展开')
  assert.match(text, /`\[T-001\]\(\.\.\/task\/OLD\.md\)`/, '段内的旧地址不得被重算')
  assert.match(text, /段外的真引用自当照改：见 \[T-001\]\(\.\.\/task\/T-001\.md\)/, '段外的旧地址照改')
  // 反引号标签的链接（标签在段内、链接起点在段外）仍按真引用重算
  const b2 = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/y.md': planDoc('docs/plan/y.md', '# Y\n\n见 [`T-001`](../task/OLD.md)。\n'),
  })
  const r2 = runGenDocRefs([`--root=${b2}`])
  assert.equal(r2.code, 0, out(r2))
  assert.match(read(b2, 'docs/plan/y.md'), /\[`T-001`\]\(\.\.\/task\/T-001\.md\)/)
  assert.equal(runGenDocRefs([`--root=${b2}`]).changed.length, 0, '幂等')
})

test('范围读不出 ⇒ exit 2（不退化成「什么都没检查」）', () => {
  const empty = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-refs-none-'))
  assert.equal(runGenDocRefs([`--root=${empty}`]).code, 2)
  assert.equal(runGenDocRefs(['--bogus']).code, 2)
  fs.rmSync(empty, { recursive: true, force: true })
})

test('--check：漂移即非零退出（CI 用），且不写盘', () => {
  const base = caseRoot({
    'docs/task/T-001.md': taskDoc('T-001'),
    'docs/plan/x.md': planDoc('docs/plan/x.md', '# X\n\n见 [T-001](../task/OLD.md)。\n'),
  })
  const before = read(base, 'docs/plan/x.md')
  const r = runGenDocRefs([`--root=${base}`, '--check'])
  assert.equal(r.code, 1)
  assert.match(out(r), /RF-002 docs\/plan\/x\.md/)
  assert.equal(read(base, 'docs/plan/x.md'), before, '--check 不得写盘')
})

// `gate-codes-audit` 的回归测试。
//
// 判据码是判据的**地址**，而这条守卫判的是地址本身会不会撞车。它的失效形态与
// `35-baseline` 同族：**判据写错了就永远不命中，日志照样一片绿**。所以每条判据都
// 配一个注入的真实违规（必须变红）与一个反向用例（不该误报）——尤其 GC-003，
// 它的实现曾经恒真（声明行里当然包含那个码），只有注入才暴露得出来。
//
// 跑法：node --test scripts/gate-codes-audit.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { checkDecls, collect } from './gate-codes-audit.mjs'
import { parseGateCodeDecls, stringLiteralsOf } from './gate.d/_shared.mjs'

const repoRoot = path.resolve(import.meta.dirname, '..')

// ================= 夹具 =================
//
// 用**真实文件系统**而不是内存 Map：`collect` 判的就是「scripts/ 与 scripts/gate.d/
// 这两个目录里有哪些脚本」，打桩等于把被测的扫描范围换掉。

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-gate-codes-'))

const decl = (ns, codes, extra = '') =>
  `// @ns ${ns}\n// @codes ${codes}\n${extra}\nconsole.log('done')\n`

/**
 * 一个用例一座目录：`collect` 扫的是整个 `scripts/`，夹具若共用一个目录，
 * 上一个用例留下的脚本会串进这一个的断言里（而失败形态是「多了一条没预期的红」）。
 */
let caseNo = 0
function errors(files) {
  const base = path.join(root, `case${++caseNo}`)
  for (const [rel, body] of Object.entries(files)) {
    const p = path.join(base, 'scripts', rel)
    fs.mkdirSync(path.dirname(p), { recursive: true })
    fs.writeFileSync(p, body)
  }
  return checkDecls(collect(base)).error
}

after(() => {
  try {
    fs.rmSync(root, { recursive: true, force: true, maxRetries: 3 })
  } catch {
    /* 沙箱的 safe-delete 垫片可能拒删，交 OS 回收 */
  }
})

// ================= 声明解析 =================

test('parseGateCodeDecls：@ns 与 @codes 各归各，前缀取自码本身', () => {
  const { ns, codes } = parseGateCodeDecls(decl('D 文档正文与链接', 'D-001 D-002 D-006'))
  assert.deepEqual([...ns], [['D', '文档正文与链接']])
  assert.deepEqual([...codes], [['D-001', 'D'], ['D-002', 'D'], ['D-006', 'D']])
})

test('parseGateCodeDecls：形状不对的声明行不算声明', () => {
  const { ns, codes } = parseGateCodeDecls(
    ['// @ns TOOLONG 前缀超过四个字母', '// @codes D-1', '// @codes d-001', '//@codes D-002 注释符后无空格'].join('\n'),
  )
  assert.equal(ns.size, 0, '前缀是 1–4 个大写字母（`NDC` 合法，`TOOLONG` 不是）')
  assert.equal(codes.size, 0, '码必须是 <前缀>-<三位数字>，且声明行要写成 `// @codes`')
})

// ================= stringLiteralsOf：注释不算、字符串里的 // 不算结束 =================

test('stringLiteralsOf：行注释与块注释里的内容一律不取', () => {
  const src = `// 原 E-010 已退役\n/* 见 D-005 */\nconst a = 'S-001'\nconsole.log("S-002")\n`
  assert.deepEqual(stringLiteralsOf(src).filter((s) => /[A-Z]+-\d{3}/.test(s)), ['S-001', 'S-002'])
})

test('stringLiteralsOf：字符串里的 `//` 不得把这一行当注释截掉', () => {
  // 这是最容易被写反的一条：朴素实现「删掉 // 之后的内容」会连 URL 一起吃掉，
  // 于是同一行后半句里的判据码**静默消失**——漏报的形态与「没有违规」完全一样。
  const src = `const u = 'https://example.com/x' // 说明\nconsole.log('R-002 已登记')\n`
  const found = stringLiteralsOf(src).filter((s) => /[A-Z]+-\d{3}/.test(s))
  assert.deepEqual(found, ['R-002 已登记'])
})

test('stringLiteralsOf：正则字面量里的引号不得开启字符串', () => {
  // `/['"]/` 一旦被判成字符串起点，扫描器会一路吞到下一个引号，中间的**注释**
  // 就被当成字符串内容 ⇒ 历史叙述（「原 E-010 已退役」）变成假违规。
  const src = `const re = /['"]/g // 原 E-010 已退役\nconsole.log('N-001')\n`
  assert.deepEqual(stringLiteralsOf(src).filter((s) => /[A-Z]+-\d{3}/.test(s)), ['N-001'])
})

test('stringLiteralsOf：模板字面量整段取（含 ${} 里的插值文本）', () => {
  const src = 'console.log(`${red("[ERROR]")} D-005 ${file}`)\n'
  assert.ok(
    stringLiteralsOf(src).some((s) => s.includes('D-005')),
    '模板串里的判据码必须看得见，否则 GC-004 对最常见的输出形态失效',
  )
})

// ================= 六条判据：注入必须变红 =================

test('合法夹具 ⇒ 零违规（守卫不误报，且真的读到了声明）', () => {
  const errs = errors({
    'a-audit.mjs': decl(
      'D 文档正文与链接',
      'D-001 D-002',
      'console.log("D-001 坏链")\nconsole.log("D-002 该归档")',
    ),
    'gate.d/b-audit.mjs': decl('D 文档正文与链接', 'D-005', 'console.log("D-005 符号指认")'),
  })
  assert.deepEqual(errs, [])
})

test('GC-001 撞号：两个脚本声明同一码 ⇒ 红，且报出两个归属', () => {
  const errs = errors({
    'a-audit.mjs': decl('D 文档正文与链接', 'D-001', 'console.log("D-001")'),
    'b-audit.mjs': decl('D 文档正文与链接', 'D-001', 'console.log("D-001")'),
  })
  assert.equal(errs.length, 1)
  assert.match(errs[0], /GC-001 撞号：D-001/)
  assert.match(errs[0], /a-audit\.mjs.*b-audit\.mjs/)
})

test('GC-002 未登记前缀：@codes 用了本文件没声明的前缀 ⇒ 红', () => {
  const errs = errors({ 'a-audit.mjs': decl('D 文档正文与链接', 'D-001 X-001', 'console.log("D-001 X-001")') })
  assert.ok(errs.some((e) => /GC-002 未登记前缀：.*X-001.*@ns X/.test(e)), errs.join('\n'))
})

test('GC-003 登记错位：码只出现在声明行里 ⇒ 红（本文件其实没判它）', () => {
  const errs = errors({ 'a-audit.mjs': decl('D 文档正文与链接', 'D-001 D-099', 'console.log("D-001")') })
  assert.equal(errs.length, 1, 'D-099 只在 @codes 行出现过')
  assert.match(errs[0], /GC-003 登记错位：.*D-099/)
})

test('GC-004 输出了未登记的码 ⇒ 红；加判定要先登记', () => {
  const errs = errors({
    'a-audit.mjs': decl('S 源码 grep 形态', 'S-001', 'console.log("S-001")'),
    'b-audit.mjs': `// 没有声明 S 前缀之外的事\nconsole.log('S-002 命中')\n`,
  })
  assert.ok(errs.some((e) => /GC-004.*S-002.*没有任何脚本声明/.test(e)), errs.join('\n'))
})

test('GC-004 反向：前缀未登记的码形如 ADR-023 ⇒ 不判（它不是判据码）', () => {
  const errs = errors({
    'a-audit.mjs': decl('S 源码 grep 形态', 'S-001', 'console.log("S-001 ADR-023")'),
    'b-audit.mjs': `console.log('依据 ADR-023')\n`,
  })
  assert.deepEqual(errs, [])
})

test('GC-005 空命名空间：声明了 @ns 却没有任何 @codes 用它 ⇒ 红', () => {
  const errs = errors({
    'a-audit.mjs': decl('S 源码 grep 形态', 'S-001', 'console.log("S-001")'),
    'b-audit.mjs': "// @ns Q 只有前缀、没有号\nconsole.log('Q 还没被任何判据用')\n",
  })
  assert.equal(errs.length, 1, errs.join('\n'))
  assert.match(errs[0], /GC-005 空命名空间：Q/)
})

test('GC-006 同一前缀两种说法 ⇒ 红（共用命名空间必须先对「它是什么」达成一致）', () => {
  const errs = errors({
    'a-audit.mjs': decl('D 文档正文与链接', 'D-001', 'console.log("D-001")'),
    'b-audit.mjs': decl('D 文档链接', 'D-005', 'console.log("D-005")'),
  })
  assert.ok(errs.some((e) => /GC-006 同一前缀两种说法：D/.test(e)), errs.join('\n'))
})

test('GC-006 反向：描述逐字相同的共用 ⇒ 不判（D 前缀本就两个脚本共用）', () => {
  const errs = errors({
    'a-audit.mjs': decl('D 文档正文与链接', 'D-001', 'console.log("D-001")'),
    'b-audit.mjs': decl('D 文档正文与链接', 'D-005', 'console.log("D-005")'),
  })
  assert.deepEqual(errs, [])
})

// ================= 本仓真实状态（dogfood） =================

// 守卫对自己也要成立：本仓 9 个前缀、52 余条码必须**零违规**。
// 前缀数与码数一起断言，是为了让「解析口径失效 ⇒ 什么都没读到 ⇒ 全绿」这条路走不通。
test('本仓当前声明自洽，且解析真的读出了全部命名空间', () => {
  const scripts = collect(repoRoot)
  assert.deepEqual(checkDecls(scripts).error, [])
  const prefixes = new Set(scripts.flatMap((s) => [...s.ns.keys()]))
  const total = scripts.reduce((n, s) => n + s.codes.size, 0)
  assert.ok(prefixes.size >= 9, `只读到 ${prefixes.size} 个前缀——声明的行形状变了？`)
  assert.ok(total >= 50, `只读到 ${total} 条码——解析口径与脚本一起失效了`)
  const ownerOf = (code) => scripts.find((s) => s.codes.has(code))?.rel
  assert.equal(ownerOf('D-005'), 'scripts/doc-symbol-audit.mjs')
  assert.equal(ownerOf('R-002'), 'scripts/dead-code-audit.mjs')
  assert.equal(ownerOf('E-010'), undefined, 'E-010 已退役，不该被任何脚本重新认领')
})

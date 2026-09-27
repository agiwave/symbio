/**
 * `rust-scan.mjs` 回归测试
 *
 * 这些扫描器的共同特征是**写错不报错，只静默漏报**。所以每个用例都要钉住一个
 * 「朴素实现在这里会翻车」的具体输入，而不是复述 happy path。
 *
 * 跑法：node --test scripts/rust-scan.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  skipString,
  stripComments,
  blankComments,
  matchBrace,
  testModuleSpans,
  stripTestModules,
  blankTestModules,
} from './rust-scan.mjs'

// ── skipString ────────────────────────────────────────────────────────────

test('skipString 吃掉转义引号，停在真正的闭引号', () => {
  const src = String.raw`"a\"b" rest`
  assert.equal(skipString(src, 0), src.indexOf('" rest'))
})

test('skipString 认原始字符串 r#"…"#（内含 " 不闭）', () => {
  const src = 'r#"has " inside"# tail'
  const end = skipString(src, src.indexOf('"'))
  assert.equal(src.slice(0, end + 1), 'r#"has " inside"#')
})

test('skipString 认 br###"…"###（内含 " 与 "## 不闭，直到 "###）', () => {
  const src = 'br###"x"##y"### tail'
  // start 必须是**开引号**下标（`br###"` 的 `"` 在 5），不是 `r` 或 `b` 的下标
  const end = skipString(src, src.indexOf('"'))
  assert.equal(src.slice(0, end + 1), 'br###"x"##y"###')
})

test('skipString 恒向前：返回值 ≥ start（否则外层扫描原地打转）', () => {
  const src = '"unterminated'
  assert.ok(skipString(src, 0) >= 0)
})

// ── stripComments ─────────────────────────────────────────────────────────

test('stripComments 不把字符串里的 // 当注释起点', () => {
  const src = 'let u = "https://api.example.com/v1"; // 真注释\nlet x = 1;'
  const out = stripComments(src)
  assert.ok(out.includes('"https://api.example.com/v1"'), `字符串被截断：${out}`)
  assert.ok(!out.includes('真注释'))
  assert.ok(out.includes('let x = 1;'))
})

test('stripComments 剥块注释，且不吞掉块注释后的代码', () => {
  const src = 'let a = 1; /* 多行\n注释 */ let b = 2;'
  const out = stripComments(src)
  assert.ok(out.includes('let a = 1;'))
  assert.ok(out.includes('let b = 2;'))
  assert.ok(!out.includes('多行'))
})

// ── blankComments ─────────────────────────────────────────────────────────

test('blankComments 保留行数——行号必须与原始文件一致', () => {
  // 'a // c\nb\n/*\nc\n*/\nd' 共 6 行；块注释跨第 3、4 行，第 5 行是 `*/` 收尾
  // （被块内状态吃掉，只剩空行），第 6 行 'd' 原样保留。
  const src = 'a // c\nb\n/*\nc\n*/\nd'
  const lines = blankComments(src)
  assert.equal(lines.length, src.split('\n').length)
  assert.equal(lines[0].trim(), 'a')
  assert.equal(lines[1], 'b')
  assert.equal(lines[2].trim(), '', '块注释起始行应被抹空')
  assert.equal(lines[3].trim(), '', '块注释内容行应被抹空')
  assert.equal(lines[4].trim(), '', '块注释收尾行应被抹空')
  assert.equal(lines[5], 'd')
})

test('blankComments 换行时重置引号状态——未闭合引号不吞后续代码', () => {
  // Rust 字符串不跨行：第一行的孤立引号不能把第二行整段判成「在字符串里」
  const src = 'let a = "oops\nlet b = 2; // c'
  const lines = blankComments(src)
  assert.ok(lines[1].includes('let b = 2;'), `第二行被吞：${JSON.stringify(lines[1])}`)
  assert.ok(!lines[1].includes('// c'))
})

test('blankComments 默认不吃 HTML 注释', () => {
  const src = '<!-- 路径 docs/x.md -->\nreal'
  const lines = blankComments(src)
  assert.ok(lines[0].includes('docs/x.md'))
})

test('blankComments html:true 剥 HTML 注释（.vue 顶部文档注释）', () => {
  const src = '<!-- 路径 docs/x.md -->\nreal'
  const lines = blankComments(src, { html: true })
  assert.ok(!lines[0].includes('docs/x.md'))
  assert.equal(lines[1], 'real')
})

// ── matchBrace ────────────────────────────────────────────────────────────

test('matchBrace 在字面量括号上不错位（format!("{{}}")）', () => {
  const src = 'fn f() { let s = format!("{{}}"); }'
  const open = src.indexOf('{')
  assert.equal(matchBrace(src, open), src.length - 1)
})

test('matchBrace 不被字符串里的 } 提前收尾', () => {
  const src = 'fn f() { let s = "}"; }'
  const open = src.indexOf('{')
  assert.equal(matchBrace(src, open), src.length - 1)
})

test('matchBrace 未配平时返回 txt.length（越界一位，便于 slice 取到末尾）', () => {
  const src = 'fn f() { unterminated'
  const open = src.indexOf('{')
  assert.equal(matchBrace(src, open), src.length)
  assert.equal(src.slice(open, matchBrace(src, open) + 1), '{ unterminated')
})

// ── testModuleSpans / stripTestModules / blankTestModules ─────────────────

test('testModuleSpans 只收 #[cfg(test)] mod …{}，不收 use 形式', () => {
  const src = '#[cfg(test)]\nuse foo;\n\n#[cfg(test)]\nmod tests { fn t() {} }\n'
  const spans = testModuleSpans(src)
  assert.equal(spans.length, 1)
  assert.ok(src.slice(spans[0][0], spans[0][1]).includes('fn t()'))
})

test('testModuleSpans 不计分号形式（mod tests; 没有花括号体）', () => {
  const src = '#[cfg(test)]\n#[path = "x.test.rs"]\nmod tests;\nAFTER'
  assert.equal(testModuleSpans(src).length, 0)
})

test('testModuleSpans 剔除测试模块后保留其后的生产代码', () => {
  // 实测踩坑：model/plugin.rs 的 mod tests 在中段，traverse 注册在其后
  const src = 'PROD_A\n#[cfg(test)]\nmod tests { fn t() {} }\nPROD_B\n'
  const out = stripTestModules(src)
  assert.ok(out.includes('PROD_A'))
  assert.ok(out.includes('PROD_B'), '测试模块之后的生产代码被丢了')
  assert.ok(!out.includes('fn t()'))
})

test('testModuleSpans 进度保证：不因畸形输入死循环', () => {
  const src = '#[cfg(test)] #[cfg(test)] #[cfg(test)] tail'
  assert.doesNotThrow(() => testModuleSpans(src))
})

test('blankTestModules 保住行数，且把测试体抹成空白', () => {
  const src = 'fn prod() {}\n#[cfg(test)]\nmod tests {\n  fn t() {}\n}\n'
  const out = blankTestModules(src)
  assert.equal(out.split('\n').length, src.split('\n').length)
  assert.ok(out.includes('fn prod() {}'))
  assert.ok(!out.includes('fn t()'))
})

test('stripTestModules 与 blankTestModules 剔除范围一致', () => {
  const src = 'A\n#[cfg(test)]\nmod tests { x }\nB\n'
  const stripped = stripTestModules(src).replace(/\s+/g, ' ').trim()
  const blanked = blankTestModules(src)
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .join(' ')
  assert.equal(stripped, blanked)
})

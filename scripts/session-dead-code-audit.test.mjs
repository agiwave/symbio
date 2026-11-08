/**
 * session-dead-code-audit 的 SD-002 回归测试 —— 证明它**会红**
 *
 * SD-002 拦的是「测试专用符号留在生产构建里、用 `cfg_attr(not(test), allow(dead_code))`
 * 换个拼写压掉同一条警告」。它的失效形态是**静默放行**：检测正则写宽/写窄，脚本照常
 * 亮绿灯，于是这个口子可以随便用。所以这里注入坏形态断言检出，并钉住两件事：
 *   ① 检出必须带**符号名 + 行号**（否则报告只知道「某处有」，无法处置）；
 *   ② 例外只能靠 `CFG_ATTR_EXEMPT` 的「路径 + 符号 + 理由」（理由空、登记过期都判红）。
 *
 * 用法：node --test scripts/session-dead-code-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  CFG_ATTR_EXEMPT,
  cfgAttrAllowInSource,
  compareCfgAttr,
  validateCfgAttrExempt,
} from './session-dead-code-audit.mjs'

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/** 在临时目录里铺一份 `.rs`，返回检出结果。 */
function scan(source) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'session-dead-code-'))
  try {
    fs.writeFileSync(path.join(root, 'fixture.rs'), source)
    return cfgAttrAllowInSource(root).map((h) => ({ ...h, basename: path.basename(h.path) }))
  } finally {
    rmTree(root)
  }
}

test('SD-002 单行 cfg_attr 包 allow(dead_code) 必须检出（带符号与行号）', () => {
  const hits = scan(
    '#[cfg_attr(not(test), allow(dead_code))]\npub(crate) fn only_tests_use_me() -> i32 {\n    1\n}\n',
  )
  assert.equal(hits.length, 1)
  assert.equal(hits[0].basename, 'fixture.rs')
  assert.equal(hits[0].line, 1)
  assert.equal(hits[0].symbol, 'only_tests_use_me')
})

test('SD-002 跨行属性同样检出（不是逐行正则）', () => {
  const hits = scan(
    '#[cfg_attr(\n    not(any(test, doc)),\n    allow(dead_code)\n)]\nfn multi_line() {}\n',
  )
  assert.equal(hits.length, 1)
  assert.equal(hits[0].line, 1, '报属性起始行，不是 allow 那一行')
  assert.equal(hits[0].symbol, 'multi_line')
})

test('SD-002 内层属性 #![cfg_attr(...)] 也检出', () => {
  const hits = scan('#![cfg_attr(not(test), allow(dead_code))]\nfn inner() {}\n')
  assert.equal(hits.length, 1)
  assert.equal(hits[0].symbol, 'inner')
})

test('SD-002 不误伤：#[cfg(test)]、别的 allow、文档里的举例', () => {
  const hits = scan(
    [
      '#[cfg(test)]',
      'fn lives_in_tests() {}',
      '#[cfg_attr(not(test), allow(unused_variables))]',
      'fn other_lint() {}',
      '/// 这里会写成 `#[cfg_attr(not(test), allow(dead_code))]` 但只是文档',
      'fn documented() {}',
      '',
    ].join('\n'),
  )
  assert.deepEqual(hits, [])
})

test('SD-002 不把 `allow(dead_code)` 不紧邻的 cfg_attr 算进来', () => {
  // 属性块只到自己的闭括号为止；后面函数体里的 allow 是另一个属性的事
  const hits = scan(
    '#[cfg_attr(not(test), allow(unused))]\nfn f() {\n    #[allow(dead_code)]\n    let _ = 1;\n}\n',
  )
  assert.deepEqual(hits, [], 'allow(dead_code) 不在这个 cfg_attr 的块内')
})

test('CFG_ATTR_EXEMPT 三列缺一不可（空理由 = 未登记）', () => {
  assert.deepEqual(validateCfgAttrExempt([]), [])
  assert.deepEqual(
    validateCfgAttrExempt([{ path: 'a.rs', symbol: 's', reason: '   ' }]),
    ['CFG_ATTR_EXEMPT[0]：缺 `reason`'],
  )
  assert.deepEqual(
    validateCfgAttrExempt([{ path: 'a.rs', reason: 'r' }]),
    ['CFG_ATTR_EXEMPT[0]：缺 `symbol`'],
  )
})

test('未登记的检出判红；登记匹配则放行；过期登记也判红', () => {
  const hit = { path: 'a.rs', line: 3, symbol: 's' }
  const ok = compareCfgAttr([hit], [{ path: 'a.rs', symbol: 's', reason: '协议契约' }])
  assert.deepEqual(ok, { violations: [], stale: [] })

  const unregistered = compareCfgAttr([hit], [])
  assert.equal(unregistered.violations.length, 1)
  assert.deepEqual(unregistered.stale, [])

  const stale = compareCfgAttr([], [{ path: 'a.rs', symbol: 's', reason: '协议契约' }])
  assert.equal(stale.violations.length, 0)
  assert.equal(stale.stale.length, 1, '源码里已不再出现 ⇒ 删登记，否则豁免变过期地图')
})

test('真仓：检出项全部有登记（当前应为 0 处）', () => {
  const hits = cfgAttrAllowInSource()
  const { violations, stale } = compareCfgAttr(hits, CFG_ATTR_EXEMPT)
  assert.deepEqual(violations, [])
  assert.deepEqual(stale, [])
})

// 防止「CFG_ATTR_EXEMPT 后来被人塞了条不解释自己的例外」：这个文件本身就是判据源，
// 加条目必须同步改上面那条真仓用例的语义，而不是把测试调宽。
test('CFG_ATTR_EXEMPT 当前为空（留例外要先有理由与真实检出项）', () => {
  assert.deepEqual(CFG_ATTR_EXEMPT, [])
})

/**
 * `scripts/facts/current.mjs` 的输入契约回归测试。
 *
 * 判据只有一条：**源文件读不到 / 提取结果为空，必须抛错**，不许渲染成 `0`。
 *
 * 为什么这条要单独立测试：`DE-017` 已经把「读空即全绿」命名为守卫唯一的坏死方式，
 * 并为 registry-audit 立了「条目数非零」的 dogfood 断言；`scripts/facts/` 下的结构
 * 事实生成器此前是唯一没接这条纪律的一族。它的失效形态比「漏写 `--ci`」更隐蔽——
 * 路径写错 → `catch` 吞掉 → 生成物写 0 → `gen-facts --check` 比对「生成结果 ==
 * 当前文件」→ 一致 → 本地与 CI 同时全绿。D-191 把 CLI 拆成独立 crate
 * （`crates/symbio-cli/`）之后，CLI 面就这样被确定性地投影成 0/0/0。
 *
 * 故本测试钉的是行为而不是字符串：把 helper 指向不存在的路径 **必须**抛错。
 */

import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { requiredSource, requiredHits } from './current.mjs'

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'facts-current-'))

test('requiredSource：文件不存在 ⇒ 抛错（不是返回空串）', () => {
  const dir = tmp()
  const missing = path.join(dir, 'args.rs')
  assert.equal(fs.existsSync(missing), false)
  assert.throws(() => requiredSource(missing, 'CLI 面'), /源文件不存在/)
})

test('requiredSource：文件存在但为空 / 全空白 ⇒ 抛错', () => {
  const dir = tmp()
  const blank = path.join(dir, 'blank.rs')
  fs.writeFileSync(blank, '  \n\n\t\n')
  assert.throws(() => requiredSource(blank, 'CLI 面'), /源文件是空的/)
})

test('requiredSource：文件存在且非空 ⇒ 原样返回内容', () => {
  const dir = tmp()
  const ok = path.join(dir, 'ok.rs')
  fs.writeFileSync(ok, 'const HELP: &str = "x";\n')
  assert.equal(requiredSource(ok, 'CLI 面'), 'const HELP: &str = "x";\n')
})

test('requiredHits：提取结果为空 ⇒ 抛错（源文件读到了，但正则没命中）', () => {
  assert.throws(() => requiredHits([], 'CLI 长选项'), /提取为空/)
  assert.throws(() => requiredHits(null, 'CLI 长选项'), /提取为空/)
})

test('requiredHits：非空 ⇒ 原样返回，供调用方继续用', () => {
  const got = requiredHits(['--message', '-m'], 'CLI 选项')
  assert.deepEqual(got, ['--message', '-m'])
})

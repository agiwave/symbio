/**
 * `scripts/facts/current.mjs` 的输入契约回归测试。
 *
 * 判据只有一条：**源文件读不到 / 提取结果为空，必须抛错**，不许渲染成 `0`。
 *
 * 为什么这条要单独立测试：`DE-017` 已经把「读空即全绿」命名为守卫唯一的坏死方式，
 * 并为 registry-audit 立了「条目数非零」的 dogfood 断言；`scripts/facts/` 下的结构
 * 事实生成器此前是唯一没接这条纪律的一族。它的失效形态比「漏写 `--ci`」更隐蔽——
 * 路径写错 → `catch` 吞掉 → 生成物写 0 → `gen-facts --check` 比对「生成结果 ==
 * 当前文件」→ 一致 → 本地与 CI 同时全绿。 把 CLI 拆成独立 crate
 * （`apps/cli/`）之后，CLI 面就这样被确定性地投影成 0/0/0。
 *
 * 故本测试钉的是行为而不是字符串：把 helper 指向不存在的路径 **必须**抛错。
 */

import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import {
  providerConsumers,
  providerSymbols,
  requiredHits,
  requiredSource,
  workspaceGraph,
} from './current.mjs'

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

const dep = (name, deps) => ({
  name,
  source: null,
  dependencies: deps.map(([depName, kind = null]) => ({ name: depName, kind })),
})

test('workspaceGraph：只算生产边，dev/build 与外部 crate 不进图', () => {
  const graph = workspaceGraph({
    packages: [
      dep('a', [['b'], ['b'], ['external'], ['c', 'dev'], ['d', 'build']]),
      dep('b', [['c']]),
      dep('c', []),
      dep('d', []),
      { name: 'external', source: 'registry', dependencies: [] },
    ],
  })
  assert.deepEqual(
    graph.rows.map((row) => [row.name, row.deps]),
    [
      ['a', ['b']],
      ['b', ['c']],
      ['c', []],
      ['d', []],
    ],
  )
  assert.equal(graph.edges, 2)
  assert.equal(graph.maxDepth, 3)
})

test('workspaceGraph：生产环不能静默投影成有限深度', () => {
  assert.throws(
    () => workspaceGraph({ packages: [dep('a', [['b']]), dep('b', [['a']])] }),
    /生产依赖图出现环/,
  )
})

test('providerSymbols：pub use 的别名取 as 后的出口名', () => {
  assert.deepEqual(
    providerSymbols(`
      pub use symbio_provider_vdfs_service::{
        id_of as vdfs_id_of, DirVdfs,
      };
      pub use symbio_provider_collectors::{new_registry, FactsOf};
    `),
    ['DirVdfs', 'FactsOf', 'new_registry', 'vdfs_id_of'],
  )
})

test('providerConsumers：认直接引用与花括号导入，不把同名裸标识符算进去', () => {
  const sources = {
    alpha: 'use symbio_providers::{DirVdfs, new_registry};',
    beta: 'let x = symbio_providers::DirVdfs::at(p);',
    gamma: 'let DirVdfs = local_type();',
  }
  assert.deepEqual(providerConsumers(['DirVdfs', 'agent_home_root'], sources), [
    { symbol: 'DirVdfs', consumers: ['alpha', 'beta'] },
    { symbol: 'agent_home_root', consumers: [] },
  ])
})

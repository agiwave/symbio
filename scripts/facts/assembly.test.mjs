/**
 * facts/assembly 回归测试
 *
 * 装配清单是**生成物**：它的价值全在「手改 / 漏声明 / 顺序冲突会红」。
 * 只测好输入的话，一个把所有校验都吞掉的生成器也能全绿。
 *
 * 跑法：node --test scripts/facts/assembly.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import assembly from './assembly.mjs'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
const GEN_FILE = path.join(REPO, 'crates', 'assembly.generated.rs')

/**
 * 一棵只有两个插件目录的临时仓库。
 * `overrides` 里的条目整份替换该插件的 `PLUGIN.yml`；值为 `null` 表示不写文件。
 */
function fixtureRepo(overrides = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'assembly-fact-'))
  const plugins = {
    alpha: 'plugin_id: alpha\nplugin_comment: 甲\nplugin_assembly: sub_agent\nplugin_order: 10\n',
    beta: 'plugin_id: beta\nplugin_comment: 乙\nplugin_assembly: sub_agent\nplugin_order: 20\n',
    ...overrides,
  }
  for (const [id, yml] of Object.entries(plugins)) {
    const dir = path.join(root, 'crates', 'plugins', id)
    fs.mkdirSync(dir, { recursive: true })
    if (yml !== null) fs.writeFileSync(path.join(dir, 'PLUGIN.yml'), yml)
  }
  return root
}

const render = (root) => assembly.render({ root })

function withRepo(overrides, fn) {
  const root = fixtureRepo(overrides)
  try {
    return fn(root)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('两个插件 ⇒ 两个 id 常量 + 按 order 排序的子树清单', () => {
  withRepo({}, (root) => {
    const out = render(root)
    assert.match(out, /pub const PLUGIN_ID_ALPHA: &str = "alpha";/)
    assert.match(out, /pub const PLUGIN_ID_BETA: &str = "beta";/)
    const list = out.slice(out.indexOf('ASSEMBLY_SUB_AGENT_PLUGINS'))
    assert.ok(list.indexOf('"alpha"') < list.indexOf('"beta"'), '清单必须按 plugin_order 排序')
  })
})

test('plugin_assembly: none ⇒ 只出 id 常量，不进子树清单', () => {
  withRepo(
    { beta: 'plugin_id: beta\nplugin_assembly: none\n' },
    (root) => {
      const out = render(root)
      assert.match(out, /pub const PLUGIN_ID_BETA: &str = "beta";/)
      const list = out.slice(out.indexOf('ASSEMBLY_SUB_AGENT_PLUGINS'))
      assert.doesNotMatch(list.slice(0, list.indexOf('];')), /"beta"/)
    },
  )
})

test('plugin_undisablable: true ⇒ 进不可停用集', () => {
  withRepo(
    { alpha: 'plugin_id: alpha\nplugin_assembly: sub_agent\nplugin_order: 10\nplugin_undisablable: true\n' },
    (root) => {
      const out = render(root)
      const list = out.slice(out.indexOf('ASSEMBLY_UNDISABLABLE_PLUGINS'))
      assert.match(list, /"alpha"/)
    },
  )
})

test('缺 PLUGIN.yml ⇒ 抛错（新插件必须带声明）', () => {
  assert.throws(
    () => withRepo({ beta: null }, (root) => render(root)),
    /缺少 symbio\/src\/plugins\/beta\/PLUGIN\.yml/,
  )
})

test('plugin_id 与目录名不一致 ⇒ 抛错', () => {
  assert.throws(
    () => withRepo({ beta: 'plugin_id: gamma\nplugin_assembly: sub_agent\nplugin_order: 20\n' }, (root) => render(root)),
    /与目录名 .* 不一致/,
  )
})

test('plugin_order 重复 ⇒ 抛错（顺序不确定的清单不是清单）', () => {
  assert.throws(
    () =>
      withRepo(
        { beta: 'plugin_id: beta\nplugin_assembly: sub_agent\nplugin_order: 10\n' },
        (root) => render(root),
      ),
    /plugin_order 10 同时来自/,
  )
})

test('sub_agent 缺 plugin_order ⇒ 抛错', () => {
  assert.throws(
    () => withRepo({ beta: 'plugin_id: beta\nplugin_assembly: sub_agent\n' }, (root) => render(root)),
    /必须给整数 plugin_order/,
  )
})

test('真仓的生成物与各插件 PLUGIN.yml 一致（手改过它就红）', () => {
  const committed = fs.existsSync(GEN_FILE) ? fs.readFileSync(GEN_FILE, 'utf8') : ''
  assert.equal(committed, render(REPO))
})

/**
 * uninstall-edge-audit 回归测试 —— 证明 EX-004 的静态一半**改输入后结论必须变**
 *
 * 这条守卫的失效形态都是「看起来在判」：
 *
 * - 只查 `Cargo.toml` 不查代码 ⇒ 经宿主门面 `pub use` 的符号引用漏网；
 * - 把生成链接块算成手写边 ⇒ 一开豁免就再也看不见新边；
 * - 扫到插件目录就整棵跳过 ⇒ 插件 A 引用插件 B（正是最该抓的那条）被一起跳过；
 * - 扫注释 / 字符串 ⇒ 文档提一句插件名就红；
 * - 插件清单一空就通过 ⇒ 判据没有输入却全绿。
 *
 * 本文件用临时目录搭假仓逐条注入；另有一条真仓冒烟。
 *
 * 用法：node --test scripts/uninstall-edge-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import {
  blankMarked,
  blankStrings,
  disablablePlugins,
  judge,
  manifestEdges,
  markerErrors,
  rustEdges,
} from './uninstall-edge-audit.mjs'

const script = fileURLToPath(new URL('./uninstall-edge-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(script), '..')

const HOST_LIB = [
  '//! 宿主装配 crate。',
  '// >>> plugin-links:begin',
  'pub use symbio_plugin_alpha;',
  'pub use symbio_plugin_beta;',
  '// <<< plugin-links:end',
].join('\n')

const HOST_CARGO = [
  '[package]',
  'name = "symbio"',
  '# >>> plugin-links:begin',
  'symbio-plugin-alpha = { path = "../symbio-plugin-alpha" }',
  'symbio-plugin-beta = { path = "../symbio-plugin-beta" }',
  '# <<< plugin-links:end',
].join('\n')

/** 搭一棵最小的假仓：alpha / beta 可卸载、locked 不可停用。 */
function makeTree(extra = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'uninstall-edge-'))
  const write = (rel, text) => {
    const abs = path.join(root, ...rel.split('/'))
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, text)
  }
  write('crates/symbio-plugin-alpha/PLUGIN.yml', 'plugin_id: alpha\n')
  write('crates/symbio-plugin-alpha/Cargo.toml', '[package]\nname = "symbio-plugin-alpha"\n')
  write('crates/symbio-plugin-alpha/src/lib.rs', 'pub fn a() {}\n')
  write('crates/symbio-plugin-beta/PLUGIN.yml', 'plugin_id: beta\n')
  write('crates/symbio-plugin-beta/Cargo.toml', '[package]\nname = "symbio-plugin-beta"\n')
  write('crates/symbio-plugin-beta/src/lib.rs', 'pub fn b() {}\n')
  write('crates/symbio-plugin-locked/PLUGIN.yml', 'plugin_id: locked\nplugin_undisablable: true\n')
  write('crates/symbio-plugin-locked/Cargo.toml', '[package]\nname = "symbio-plugin-locked"\n')
  write('crates/symbio-plugin-locked/src/lib.rs', 'pub fn c() {}\n')
  write('crates/symbio-kernel/src/lib.rs', 'pub fn k() {}\n')
  write('crates/symbio-kernel/Cargo.toml', '[package]\nname = "symbio-kernel"\n')
  write('crates/symbio/src/lib.rs', HOST_LIB)
  write('crates/symbio/Cargo.toml', HOST_CARGO)
  for (const [rel, text] of Object.entries(extra)) write(rel, text)
  return root
}

function withTree(extra, fn) {
  const root = makeTree(extra)
  try {
    return fn(root)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const edgeMessages = (root) => judge(root).errors

// ── 目标集合 ──────────────────────────────────────────────────────────────

test('disablablePlugins：排除不可停用与 plugin-api，crate 名转下划线', () => {
  withTree({}, (root) => {
    const { targets, skipped } = disablablePlugins(root)
    assert.deepEqual(targets.map((t) => t.id), ['alpha', 'beta'])
    assert.deepEqual(skipped.map((t) => t.id), ['locked'])
    assert.equal(targets[0].crate, 'symbio_plugin_alpha')
    assert.equal(targets[0].manifest, 'symbio-plugin-alpha')
  })
})

test('disablablePlugins：目录里一个插件都没有 ⇒ 抛错（范围读空不当作通过）', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'uninstall-edge-empty-'))
  try {
    fs.mkdirSync(path.join(root, 'crates', 'symbio-plugin-api'), { recursive: true })
    assert.throws(() => disablablePlugins(root), /一个插件目录都没有/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

// ── 置空的两把小刀 ────────────────────────────────────────────────────────

test('blankMarked：块含标记整段置空，块外原样，块数读得出', () => {
  const { lines, blocks } = blankMarked(['a', '// >>> plugin-links:begin', 'b', '// <<< plugin-links:end', 'c'])
  assert.deepEqual(lines, ['a', '', '', '', 'c'])
  assert.equal(blocks, 1)
})

test('blankMarked：缺标记时一行都不动（缺块由 markerErrors 单独判）', () => {
  const { lines, blocks } = blankMarked(['a', 'b'])
  assert.deepEqual(lines, ['a', 'b'])
  assert.equal(blocks, 0)
})

test('blankStrings：字符串内容置空，转义与引号结构保住', () => {
  assert.equal(blankStrings('let x = "abc";'), 'let x = "   ";')
  assert.equal(blankStrings('let s = "a\\"b"; use symbio_plugin_alpha;'), 'let s = "    "; use symbio_plugin_alpha;')
})

// ── Rust 代码边 ───────────────────────────────────────────────────────────

test('rustEdges：别的 crate 写 `use` ⇒ 红并点名目标', () => {
  withTree({ 'crates/symbio-kernel/src/lib.rs': 'use symbio_plugin_alpha::thing;\n' }, (root) => {
    const { hits } = rustEdges({ root, targets: disablablePlugins(root).targets })
    assert.equal(hits.length, 1)
    assert.equal(hits[0].target, 'alpha')
    assert.match(hits[0].where, /symbio-kernel\/src\/lib\.rs:1/)
  })
})

test('rustEdges：插件 A 引用插件 B ⇒ 红（最该抓的一条）', () => {
  withTree({ 'crates/symbio-plugin-beta/src/lib.rs': 'use symbio_plugin_alpha as a;\n' }, (root) => {
    const { hits } = rustEdges({ root, targets: disablablePlugins(root).targets })
    assert.deepEqual(hits.map((h) => h.target), ['alpha'])
  })
})

test('rustEdges：自己的 crate 引用自己 ⇒ 不算边', () => {
  withTree({ 'crates/symbio-plugin-alpha/src/lib.rs': 'pub use symbio_plugin_alpha::inner;\n' }, (root) => {
    assert.deepEqual(rustEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
  })
})

test('rustEdges：测试代码也算（卸载后 cargo test 编译不过）', () => {
  withTree({ 'crates/symbio-kernel/src/x.test.rs': '#[test] fn t() { symbio_plugin_alpha::f(); }\n' }, (root) => {
    assert.equal(rustEdges({ root, targets: disablablePlugins(root).targets }).hits.length, 1)
  })
})

test('rustEdges：注释与字符串里的名字都不算边', () => {
  withTree(
    {
      'crates/symbio-kernel/src/lib.rs': [
        '// use symbio_plugin_alpha::thing;',
        '/* 别引 symbio_plugin_alpha */',
        'const NAME: &str = "symbio_plugin_alpha";',
        '/// 文档里提一句 symbio_plugin_alpha',
      ].join('\n'),
    },
    (root) => {
      assert.deepEqual(rustEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
    },
  )
})

test('rustEdges：生成的链接块不算手写边', () => {
  withTree({}, (root) => {
    assert.deepEqual(rustEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
  })
})

test('rustEdges：引用了不可停用的插件 ⇒ 不判', () => {
  withTree({ 'crates/symbio-kernel/src/lib.rs': 'use symbio_plugin_locked::x;\n' }, (root) => {
    assert.deepEqual(rustEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
  })
})

test('rustEdges：名字只作前缀不误伤（symbio_plugin_alpha_extra 不等于 alpha）', () => {
  withTree({ 'crates/symbio-kernel/src/lib.rs': 'use symbio_plugin_alpha_extra::x;\n' }, (root) => {
    assert.deepEqual(rustEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
  })
})

// ── 清单边 ────────────────────────────────────────────────────────────────

test('manifestEdges：别的 crate 写依赖 ⇒ 红', () => {
  withTree(
    { 'crates/symbio-kernel/Cargo.toml': '[package]\nname = "symbio-kernel"\n\n[dependencies]\nsymbio-plugin-alpha = { path = "../symbio-plugin-alpha" }\n' },
    (root) => {
      const { hits } = manifestEdges({ root, targets: disablablePlugins(root).targets })
      assert.equal(hits.length, 1)
      assert.equal(hits[0].target, 'alpha')
    },
  )
})

test('manifestEdges：表头形态与改名依赖都算边', () => {
  withTree(
    {
      'crates/symbio-kernel/Cargo.toml': [
        '[package]',
        'name = "symbio-kernel"',
        '[dev-dependencies.symbio-plugin-alpha]',
        'path = "../symbio-plugin-alpha"',
        '[dependencies]',
        'a2 = { package = "symbio-plugin-beta", path = "../symbio-plugin-beta" }',
      ].join('\n'),
    },
    (root) => {
      const ids = manifestEdges({ root, targets: disablablePlugins(root).targets }).hits.map((h) => h.target).sort()
      assert.deepEqual(ids, ['alpha', 'beta'])
    },
  )
})

test('manifestEdges：自清单的 `name =` 不是边', () => {
  withTree({}, (root) => {
    assert.deepEqual(manifestEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
  })
})

test('manifestEdges：注释掉的依赖与生成块都不算边', () => {
  withTree(
    {
      'crates/symbio-kernel/Cargo.toml': [
        '[package]',
        'name = "symbio-kernel"',
        '# symbio-plugin-alpha = { path = "../symbio-plugin-alpha" }',
        '# >>> plugin-links:begin',
        'symbio-plugin-beta = { path = "../symbio-plugin-beta" }',
        '# <<< plugin-links:end',
      ].join('\n'),
    },
    (root) => {
      assert.deepEqual(manifestEdges({ root, targets: disablablePlugins(root).targets }).hits, [])
    },
  )
})

// ── 标记块 ────────────────────────────────────────────────────────────────

test('markerErrors：宿主门面缺标记块 ⇒ 红并指路 gen-facts', () => {
  withTree({ 'crates/symbio/src/lib.rs': 'pub use symbio_plugin_alpha;\n' }, (root) => {
    const [e] = markerErrors({ root })
    assert.match(e, /EX-004/)
    assert.match(e, /gen-facts/)
  })
})

test('markerErrors：装配点文件缺失 ⇒ 红（判据没有输入）', () => {
  withTree({ 'crates/symbio/Cargo.toml': '[package]\nname = "symbio"\n' }, (root) => {
    assert.match(markerErrors({ root }).join('\n'), /缺 .*标记块|不存在/)
  })
})

// ── 汇总 ──────────────────────────────────────────────────────────────────

test('judge：干净假仓 ⇒ 零违规，统计读得出', () => {
  withTree({}, (root) => {
    const r = judge(root)
    assert.deepEqual(r.errors, [])
    assert.equal(r.targets.length, 2)
    assert.ok(r.rust.scanned >= 3)
    assert.ok(r.manifest.scanned >= 3)
  })
})

test('judge：注入一条边 ⇒ 红并点名文件与目标', () => {
  withTree({ 'crates/symbio-kernel/src/lib.rs': 'use symbio_plugin_alpha::x;\n' }, (root) => {
    const [e] = edgeMessages(root)
    assert.match(e, /EX-004/)
    assert.match(e, /alpha/)
    assert.match(e, /symbio-kernel/)
  })
})

test('CLI：假仓跑绿（--root= 指路）', () => {
  withTree({}, (root) => {
    const r = spawnSync(process.execPath, [script, `--root=${root}`], { encoding: 'utf8', shell: false })
    assert.equal(r.status, 0, r.stderr || r.stdout)
    assert.match(r.stdout, /编译期边为 0/)
    assert.match(r.stdout, /不可停用/)
  })
})

// ── 真仓冒烟 ──────────────────────────────────────────────────────────────

test('真仓冒烟：20 个可卸载插件、编译期边为 0', () => {
  const r = spawnSync(process.execPath, [script], { cwd: repoRoot, encoding: 'utf8', shell: false })
  assert.equal(r.status, 0, r.stderr || r.stdout)
  assert.match(r.stdout, /可卸载插件的编译期边为 0/)
  assert.match(r.stdout, /plugin_manager/)
})

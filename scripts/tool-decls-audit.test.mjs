/**
 * tool-decls-audit 回归测试 —— 证明它**会红**，且**只在该红的地方红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。本脚本判的是「工具清单」与「声明面基线」
 * 两份账**逐名相等**，两个方向各守一头，故回归逐条注入：
 *
 *   ① 干净夹具（两份相等）→ exit 0；
 *   ② TD-001：工具在册而基线无行 → 红；
 *   ③ TD-002：基线有行而工具不在册 → 红；
 *   ④ 工具清单读不出（提取器坏）→ exit 2——**不许「读空即全绿」**：
 *      空集合不是「全部通过」，是判据失效。
 *
 * 两份输入都从外部给：工具清单走 `--root=<夹具根>`（复用 `facts/_capability.mjs::toolFacts`
 * 的提取器，夹具形态与 `plugin-special-case-audit.test.mjs` 同款），基线走
 * `TOOL_DECLS_BASELINE` 环境变量——守卫逻辑与具体工具名解耦，一次无关改名不会把
 * 这条测试染红。
 *
 * 用法：node --test scripts/tool-decls-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./tool-decls-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，见 test-layout-audit.test.mjs）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/**
 * 工具清单的**最小可提取**夹具。
 *
 * 为什么不直接用本仓真实的工具名：那样这条判据会随一次无关改名一起红（某个工具
 * 若被改名，红的理由却是「守卫坏了」）。工具名从代码提取这件事本身由 `facts/_capability`
 * 的测试守；这里只要求「提取接口给得出名字」，且能造出「多一个 / 少一个」两种偏差。
 */
function toolsFixture(names) {
  return {
    'crates/symbio-plugin-fake/src/tools.rs':
      [
        'pub fn table() -> Vec<ToolSpec> {',
        '    vec![',
        ...names.map((n) => `        tool("${n}", "说明", schema(), CapabilityRiskLevel::Low),`),
        '    ]',
        '}',
        '',
      ].join('\n'),
  }
}

/** 跑一次守卫：`tools` 为工具名夹具，`baseline` 为声明面基线（JSON 数组），`rawRoot` 可给完全空的夹具。 */
function run({ tools = ['agent_run'], baseline = ['agent_run'], emptyRoot = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tool-decls-'))
  try {
    if (!emptyRoot) {
      for (const [rel, content] of Object.entries(toolsFixture(tools))) {
        const abs = path.join(root, rel)
        fs.mkdirSync(path.dirname(abs), { recursive: true })
        fs.writeFileSync(abs, content)
      }
    }
    const result = spawnSync(
      process.execPath,
      [script, `--root=${root}`],
      {
        cwd: root,
        env: { ...process.env, NO_COLOR: '1', TOOL_DECLS_BASELINE: JSON.stringify(baseline) },
        encoding: 'utf8',
        timeout: 30_000,
      },
    )
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

test('干净夹具：两份账逐名相等 ⇒ exit 0', () => {
  const r = run({ tools: ['agent_run', 'vdfs_read'], baseline: ['agent_run', 'vdfs_read'] })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /✓ tool-decls-audit 通过/)
})

test('TD-001：工具在册而基线无行 ⇒ 红（声明面漏账）', () => {
  const r = run({ tools: ['agent_run', 'todo_write'], baseline: ['agent_run'] })
  assert.equal(r.status, 1, r.stderr)
  assert.match(r.stdout, /TD-001/)
})

test('TD-002：基线有行而工具不在册 ⇒ 红（悬空行）', () => {
  const r = run({ tools: ['agent_run'], baseline: ['agent_run', 'ghost_tool'] })
  assert.equal(r.status, 1, r.stderr)
  assert.match(r.stdout, /TD-002/)
})

test('工具清单读不出 ⇒ 判据失效，不许「读空即全绿」', () => {
  const r = run({ emptyRoot: true, baseline: ['agent_run'] })
  assert.equal(r.status, 2, r.stderr)
  assert.match(r.stderr, /失去判据/)
})

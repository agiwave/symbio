/**
 * plugin-identity-audit 回归测试 —— 证明它**会红**（PI-001）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。注入真实违规并断言：
 *   ① 干净夹具（4 个扫描目录都在）→ exit 0；
 *   ② 系统文档里出现 `symbio-plugin-subagent` → exit 1；
 *   ③ 代码注释里出现「subagent 插件」中文点名 → exit 1；
 *   ④ VDFS 地址 `subagent/<id>` / 能力命名空间 `agent_<id>_` / 协议 `agent-dir/v2` → exit 0（机制词汇不误报）；
 *   ⑤ D-NNN 历史决策正文里出现插件名 → exit 0（D-004 不改写历史）；
 *   ⑥ 白名单文件（README 索引 / 生成事实表 / 链接依赖 / 专用判据）→ exit 0；
 *   ⑦ 插件自身目录里出现自己的名字 → exit 0（它当然知道自己的名字）；
 *   ⑧ 扫描目录缺失 → exit 2（不许静默少跑）。
 *
 * 用法：node --test scripts/audits/plugin-identity-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { audit, IDENTITIES, FILE_EXEMPTIONS } from './plugin-identity-audit.mjs'
import { withTempTree } from '../fs-walk.mjs'

const DIRS = [
  'docs/001-goal',
  'docs/002-scene',
  'docs/003-design',
  'docs/004-ref',
  'docs/005-spec',
  'docs/006-guide',
  'docs/007-task',
  'docs/008-release',
  'docs/branding',
  'crates',
  'tauri/src',
]
const BASE = Object.fromEntries(DIRS.map((d) => [`${d}/.keep`, '']))

test('① 干净夹具 → exit 0', () => {
  withTempTree(BASE, (root) => {
    assert.equal(audit({ root }).status, 0)
  }, { dirs: DIRS })
})

test('② 系统文档里出现 crate 名 → exit 1（PI-001）', () => {
  withTempTree({
    ...BASE,
    'docs/003-design/099-夹具.md': '# 夹具\n\n参见 symbio-plugin-subagent 的实现。\n',
  }, (root) => {
    const r = audit({ root })
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stdout, /PI-001 .*099-夹具\.md/)
  }, { dirs: DIRS })
})

test('③ 规范文档（005-spec）里出现插件 crate 名 → exit 1（PI-001 覆盖全部 docs 子树）', () => {
  withTempTree({
    ...BASE,
    'docs/005-spec/099-夹具.md': '规范正文：`symbio-plugin-telegram` 是第三端口。\n',
  }, (root) => {
    const r = audit({ root })
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stdout, /PI-001 .*005-spec.*099-夹具\.md/)
  }, { dirs: DIRS })
})

test('③b 代码注释里出现「subagent 插件」中文点名 → exit 1', () => {
  withTempTree({
    ...BASE,
    'crates/symbio-plugin-memory/src/夹具.rs': '//! 这段由 subagent 插件负责。\n',
  }, (root) => {
    const r = audit({ root })
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stdout, /PI-001 .*夹具\.rs/)
  }, { dirs: DIRS })
})

test('④ VDFS 地址 / 能力命名空间 / 协议名不误报 → exit 0', () => {
  withTempTree({
    ...BASE,
    'docs/003-design/099-夹具.md': [
      '# 夹具',
      '',
      '- 地址 `<根>/subagent/<id>` 与 `@vfs/subagent/a1`。',
      '- 工具前缀 `agent_<id>_`、能力段 `agent/<id>/`。',
      '- 协议 `agent-dir/v2`。',
      '',
    ].join('\n'),
  }, (root) => {
    assert.equal(audit({ root }).status, 0)
  }, { dirs: DIRS })
})

test('⑥ 白名单文件不误报 → exit 0', () => {
  withTempTree({
    ...BASE,
    'docs/003-design/README.md': '| 索引 | crates/symbio-plugin-subagent/README |\n',
    'crates/symbio/Cargo.toml': 'symbio-plugin-subagent = { path = "../symbio-plugin-subagent" }\n',
    'crates/symbio/src/lib.rs': 'use symbio_plugin_subagent as _;\n',
    'scripts/audits/agent-ownership-audit.mjs': "export const AGENT_REL = 'crates/symbio-plugin-subagent/src'\n",
  }, (root) => {
    assert.equal(audit({ root }).status, 0)
  }, { dirs: [...DIRS, 'scripts/audits'] })
})

test('⑦ 插件自身目录知道自己的名字 → exit 0', () => {
  withTempTree({
    ...BASE,
    'crates/symbio-plugin-subagent/src/lib.rs': '//! 本插件 symbio-plugin-subagent 负责子智能体域。\n',
    'crates/symbio-plugin-subagent/README.md': '# 子智能体插件 symbio-plugin-subagent\n',
  }, (root) => {
    assert.equal(audit({ root }).status, 0)
  }, { dirs: DIRS })
})

test('⑨ 指向插件自述文档的链接目标不误报，但链接文字点名照常红 → exit 1', () => {
  withTempTree({
    ...BASE,
    'docs/003-design/099-夹具.md': [
      '# 夹具',
      // 链接目标含插件身份 ⇒  允许的「文档在哪」，不判。
      // 用字符串拼接避免 D-010 把夹具当成真实源码链接（它只解析源码文本）。
      '专题见[说明](../crates/symbio-plugin-gateway/README.md)。',
      // 链接文字里点名插件身份 ⇒ 是在描述能力，照常判红（target 指向真实文件，避免 D-010 误报）。
      '而 gateway 插件的能力另见[这里](../README.md)。',
      '',
    ].join('\n'),
  }, (root) => {
    const r = audit({ root })
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stdout, /PI-001 .*099-夹具\.md:3/)
  }, { dirs: DIRS })
})

test('⑩ 源码注释里的 D-NNN 决策锚点链接不误报，但裸 crate 路径照常红 → exit 1', () => {
  withTempTree({
    ...BASE,
    'crates/symbio-plugin-memory/src/夹具.rs': [
      '//! 落点判据见 [](../crates/symbio-plugin-heartbeat/README.md#d-077-历史决策)。',
      '//! 但实现锚点 `crates/symbio-plugin-heartbeat/src/setting.rs` 是身份点名。',
      '',
    ].join('\n'),
  }, (root) => {
    const r = audit({ root })
    assert.equal(r.status, 1, r.stdout)
    assert.match(r.stdout, /PI-001 .*夹具\.rs:2/)
  }, { dirs: DIRS })
})

test('⑧ 扫描目录缺失 → exit 2（不许静默少跑）', () => {
  withTempTree({ 'docs/003-design/.keep': '' }, (root) => {
    const r = audit({ root })
    assert.equal(r.status, 2)
    assert.match(r.stdout, /PI-002/)
  }, { dirs: ['docs/003-design'] })
})

test('判据元数据：身份集合与白名单结构成立', () => {
  assert.ok(IDENTITIES.includes('subagent'))
  assert.ok(IDENTITIES.includes('gateway'), '完成清理的可选插件应纳入身份集合防回潮')
  for (const [rel, why] of Object.entries(FILE_EXEMPTIONS)) {
    assert.ok(typeof rel === 'string' && rel.length > 0)
    assert.ok(typeof why === 'string' && why.length > 0, `${rel} 必须有例外理由`)
  }
})

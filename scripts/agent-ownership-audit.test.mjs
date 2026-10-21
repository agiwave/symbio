import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

// `agent-ownership-audit.mjs`（AO-001…006）是**判定型**守卫，按本仓教条每条都配
// 「注入真实违规并断言脚本变红」的回归用例——09 报告 D-4 那 5 处过期描述全是
// 「没有任何东西会因此变红」的形状，判据写错等于回到没有守卫之前。
//
// 仓库根经 `--root=` 注入，故这里在临时目录造一棵最小仓库：
//   <root>/symbio/src/plugins/<name>/          插件目录（AO-001/003/005/006 的目录事实）
//   <root>/symbio/src/plugins/agent/README.md  成员资格判词 + 兄弟插件指路
//   <root>/symbio/src/plugins/agent/host/*.rs  职责表 + 归属表副本 + 真实模块文件
//   <root>/symbio/src/symbio_core/assembly/mod.rs  装配清单
const script = fileURLToPath(new URL('./agent-ownership-audit.mjs', import.meta.url))

const BASE_README = [
  '# Agent 插件',
  '',
  '子树挂与父 Agent 同构的默认插件集：',
  '',
  '- `memory` 在其中：两作用域同属一个所有者。',
  '- `setting` 在其中：子树里也有一份。',
  '',
  '## 关联',
  '',
  '- 另一层记忆：`../memory/README.md`',
  '',
].join('\n')

const BASE_MOD_RS = [
  '//! agent 宿主接入层。',
  '//!',
  '//! | 模块 | 职责 |',
  '//! |---|---|',
  '//! | [`plugin`] | 装配子树 |',
  '//!',
  '//! | 东西 | 归谁 | 地址 |',
  '//! |---|---|---|',
  '//! | 智能体自身的记忆（`AGENTS.md`） | `memory` | `<根>/memory/AGENTS.md` |',
  '//! | 智能体自身的信息设置 | `setting` | `<根>/setting/PLUGIN.yml` |',
].join('\n')

const BASE_PLUGIN_RS = [
  '//! 边界：本插件不拥有当前智能体自身的东西。',
  '//!',
  '//! | 东西 | 归谁 | 地址 |',
  '//! |---|---|---|',
  '//! | 智能体自身的记忆（`AGENTS.md`） | `memory` | `<根>/memory/AGENTS.md` |',
  '//! | 智能体自身的信息设置 | `setting` | `<根>/setting/PLUGIN.yml` |',
].join('\n')

const BASE_ASSEMBLY = ['pub const ASSEMBLY_SUB_AGENT_PLUGINS: &[&str] = &[', '    "memory",', '    "setting",', '];'].join('\n')

/**
 * 造一棵最小仓库跑一次审计。`files` 的值可以是字符串（整份覆盖）或函数
 * （拿到该文件的现行内容再改），未列出的用基线内容。
 */
function audit({ readme, modRs, pluginRs, assembly, extraHost = {}, extraPluginDirs = [] } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-ownership-audit-'))
  try {
    const agentHost = path.join(root, 'symbio/src/plugins/agent/host')
    fs.mkdirSync(agentHost, { recursive: true })
    for (const name of ['agent', 'memory', 'setting', ...extraPluginDirs]) {
      fs.mkdirSync(path.join(root, 'symbio/src/plugins', name), { recursive: true })
    }
    fs.writeFileSync(path.join(root, 'symbio/src/plugins/memory/README.md'), '# 记忆插件\n')
    fs.writeFileSync(path.join(root, 'symbio/src/plugins/agent/README.md'), readme ?? BASE_README)
    fs.writeFileSync(path.join(root, 'symbio/src/plugins/agent/host/mod.rs'), modRs ?? BASE_MOD_RS)
    fs.writeFileSync(path.join(root, 'symbio/src/plugins/agent/host/plugin.rs'), pluginRs ?? BASE_PLUGIN_RS)
    for (const [name, src] of Object.entries(extraHost)) {
      fs.writeFileSync(path.join(agentHost, name), src)
    }
    const coreDir = path.join(root, 'symbio/src/symbio_core/assembly')
    fs.mkdirSync(coreDir, { recursive: true })
    fs.writeFileSync(path.join(coreDir, 'mod.rs'), assembly ?? BASE_ASSEMBLY)

    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 20000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('一致的那一侧：基线最小仓库全绿', () => {
  const r = audit()
  assert.equal(r.status, 0, r.stdout)
})

test('AO-003：判词说反（`vdfs` 在清单里却写「不在其中」）必红', () => {
  const r = audit({
    assembly: BASE_ASSEMBLY.replace('"setting",', '"setting",\n    "vdfs",'),
    readme: BASE_README.replace('## 关联', '- `vdfs` 不在其中：单槽归系统 Agent。\n\n## 关联'),
    extraPluginDirs: ['vdfs'],
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /AO-003/)
  assert.match(r.stdout, /vdfs/)
})

test('AO-003 + AO-005：判一个不存在的插件（`work`）与幽灵兄弟目录各红一条', () => {
  const r = audit({
    readme: BASE_README.replace('- `setting` 在其中', '- `work` 在其中：注入工作区记忆\n- `setting` 在其中').replace(
      '`../memory/README.md`',
      '`../work/README.md`',
    ),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /AO-003.*work/)
  assert.match(r.stdout, /AO-005.*work/)
})

test('AO-005：文档指认的 `host/<name>.rs` 必须存在（D-4 的第三处形状）', () => {
  const r = audit({
    readme: BASE_README.replace('## 关联', '| 模块 | `host/instruction.rs` |\n|---|---|\n\n## 关联'),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /AO-005/)
  assert.match(r.stdout, /host\/instruction\.rs/)
})

test('AO-001：归属表的「归谁」指向不存在的插件、或地址首段与归谁不同源都判红', () => {
  const r = audit({
    modRs: BASE_MOD_RS.replace('| `memory` | `<根>/memory/AGENTS.md` |', '| `work` | `<根>/memory/AGENTS.md` |'),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /AO-001/)

  const drift = audit({
    modRs: BASE_MOD_RS.replace('| `memory` | `<根>/memory/AGENTS.md` |', '| `memory` | `<根>/work/AGENTS.md` |'),
    pluginRs: BASE_PLUGIN_RS.replace('| `memory` | `<根>/memory/AGENTS.md` |', '| `memory` | `<根>/work/AGENTS.md` |'),
  })
  assert.equal(drift.status, 1, drift.stdout)
  assert.match(drift.stdout, /AO-001.*不同源/)
})

test('AO-002：归属表的两份副本漂了就红（改一处忘改另一处）', () => {
  const r = audit({
    pluginRs: BASE_PLUGIN_RS.replace('| `memory` | `<根>/memory/AGENTS.md` |', '| `setting` | `<根>/setting/PLUGIN.yml` |'),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /AO-002/)
  assert.match(r.stdout, /智能体自身的记忆/)
})

test('AO-004：职责表与实际模块双向判——列了没有的红，加了不登记也红', () => {
  const ghost = audit({ modRs: BASE_MOD_RS.replace('| [`plugin`] |', '| [`scope`] |') })
  assert.equal(ghost.status, 1, ghost.stdout)
  assert.match(ghost.stdout, /AO-004/)
  assert.match(ghost.stdout, /scope/)

  const unlisted = audit({ extraHost: { 'scope.rs': '//! scope 层。\n' } })
  assert.equal(unlisted.status, 1, unlisted.stdout)
  assert.match(unlisted.stdout, /AO-004/)
  assert.match(unlisted.stdout, /没进职责表/)
})

test('AO-006：清单写了不存在的插件目录名必红（静默少挂的形状）', () => {
  const r = audit({ assembly: BASE_ASSEMBLY.replace('"memory",', '"memory",\n    "work",') })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /AO-006/)
  assert.match(r.stdout, /work/)
})

test('豁免要留痕：带理由的 `agent-ownership-allow` 放行，空理由仍红', () => {
  const withReason = audit({
    readme: BASE_README.replace('## 关联', '- `work` 在其中：历史目录<!-- agent-ownership-allow AO-003: 由 T-214 处理 -->\n\n## 关联'),
  })
  assert.equal(withReason.status, 0, withReason.stdout)

  const noReason = audit({
    readme: BASE_README.replace('## 关联', '- `work` 在其中：历史目录<!-- agent-ownership-allow AO-003: -->\n\n## 关联'),
  })
  assert.equal(noReason.status, 1, noReason.stdout)
  assert.match(noReason.stdout, /AO-003/)
})

test('来源读不出时退成 exit 2，不许退化成「没发现违规」', () => {
  const r = audit({ assembly: '// 常量被改名了\n' })
  assert.equal(r.status, 2, r.stdout)
  assert.match(r.stdout, /审计范围读不出/)
})

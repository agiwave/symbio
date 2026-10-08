/**
 * doc-symbol-audit 回归测试
 *
 * 判据的每条分支都用「注入真实违规并断言变红 / 注入合法文本并断言不变红」
 * 双向钉住：一个只会亮绿灯的守卫等于没有守卫，它腐烂的方式恰恰是规则写错
 * 后永远不命中（判据写宽 ⇒ 注释、普通词、单段词误报 ⇒ 被豁免喂到失效）。
 *
 * 跑法：node --test scripts/doc-symbol-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./doc-symbol-audit.mjs', import.meta.url))

/**
 * 在临时目录造一棵最小仓库并跑审计。
 * @param {Record<string, string>} files 相对仓库根的路径 → 内容
 */
function audit (files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-symbol-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 20000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

/** 最小 Rust 语料：两个真实符号（一个带前缀、一个仅字段名）+ 一个常量族 + 一个只在引号里的外部约定名 */
const RS = {
  'symbio/src/lib.rs': [
    'pub const PLUGIN_PAYLOAD_KEY: &str = "payload";',
    'pub const ROUTE_SESSION_CHAT_SEND: &str = "session/chat/send";',
    'pub const WIRE_TOTAL: u32 = 2;',
    'fn register_option_field(order: i32) {}',
    'fn build() { let v = env!("CARGO_PKG_VERSION"); }',
  ].join('\n'),
}

/** 最小前端语料：只有跨栈常量名，后端 .rs 里没有 */
const TS = {
  'tauri/src/constants/pages.ts': 'export const VDFS_PAGE_SIZE = 50\n',
}

test('失效指认变红：文档指认源码里不存在的符号（exit 1）', () => {
  const r = audit({ ...RS, 'README.md': '桶名见 `symbio_core::GONE_FN`。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-005 README\.md:1/)
  assert.match(r.stdout, /GONE_FN/)
})

test('整词边界：`register_option` 不因 `register_option_field` 存在而放行（exit 1）', () => {
  const r = audit({ ...RS, 'README.md': '接口 `OptionVisitor::register_option`。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /register_option`/)
})

test('有效指认通过（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '桶名见 `symbio_core::PLUGIN_PAYLOAD_KEY`。\n' })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /doc-symbol-audit 通过/)
})

test('单段反引号词：代码里没有同族时不是符号指认（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '旧名 `GONE_WORD` 已不再使用。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('非标识符形态（泛型 / 路径带参数）不查（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '形如 `map::get::<K, V>` 的写法。\n' })
  assert.equal(r.status, 0, r.stdout)
})

// ── D-009：裸常量名（同族闸把环境变量、CI 密钥挡在外面）──────────────────

test('D-009 改名漂移变红：同族有成员、这个名查无（exit 1）', () => {
  const r = audit({ ...RS, 'README.md': '分流由路由常量 `ROUTE_TRIAGE_DECIDE` 决定。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-009 README\.md:1/)
  assert.match(r.stdout, /ROUTE_TRIAGE_DECIDE/)
})

test('D-009 逐字存在即通过（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '分流由路由常量 `ROUTE_SESSION_CHAT_SEND` 决定。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 同族闸：本仓没有这一族 ⇒ 是环境变量 / CI 密钥，不判（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '签名要在 CI 里配 `APPLE_CERTIFICATE_PASSWORD`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 族名不被引号污染：`env!("CARGO_PKG_VERSION")` 开不出 CARGO_ 一族（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '构建前先设 `CARGO_TARGET_DIR` 指定产物目录。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 族名不由注释开出：注释提过 CHAT_FLOW_* 而代码没有 ⇒ 文档的 CHAT_SEND 不判（exit 0）', () => {
  const r = audit({
    ...RS,
    'tauri/src/stores/sessions.ts': '// CHAT_FLOW_ANALYSIS 早就不在了\nexport const unrelated = 1\n',
    'README.md': '发送开关叫 `CHAT_SEND`。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 认前端语料：只有 ts 里声明的常量名，文档指认它合法（exit 0）', () => {
  const r = audit({ ...RS, ...TS, 'README.md': '分页大小取 `VDFS_PAGE_SIZE`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 前端族同样开闸：VDFS_ 有成员而这个名字查无 ⇒ 红（exit 1）', () => {
  const r = audit({ ...RS, ...TS, 'README.md': '分页大小取 `VDFS_PAGE_SZIE`。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-009 .*VDFS_PAGE_SZIE/)
})

/** 最小工具脚本语料：`WIRE_` 一族由 Rust 开（见 RS），脚本里只声明其中一个名字 */
const MS = {
  'scripts/gate.d/30-docs.mjs': 'const WIRE_ONLY = new Set()\nconst ZZZ_RUNS = 1\n',
}

test('D-009 认工具脚本语料：只有 .mjs 里写过的名字，文档指认它合法（exit 0）', () => {
  const r = audit({ ...RS, ...MS, 'README.md': '非守卫的测试列在 `WIRE_ONLY` 里。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 脚本语料不放宽判据：同族有成员而这个名字查无 ⇒ 红（exit 1）', () => {
  const r = audit({ ...RS, ...MS, 'README.md': '非守卫的测试列在 `WIRE_ONLYE` 里。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-009 .*WIRE_ONLYE/)
})

test('D-009 脚本不开命名空间：ZZZ_ 只存在于脚本里 ⇒ 文档的 ZZZ_UNRELATED 不判（exit 0）', () => {
  const r = audit({ ...RS, ...MS, 'README.md': '构建产物的目录是 `ZZZ_UNRELATED_DIR`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 不认测试语料：只在 *.test.mjs 里出现过的假名不算存在（exit 1）', () => {
  const r = audit({
    ...RS,
    ...MS,
    'scripts/gate.d/30-docs.test.mjs': 'const WIRE_INVENTED = 1\n',
    'README.md': '名单里还有一项 `WIRE_INVENTED`。\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-009 .*WIRE_INVENTED/)
})

test('D-009 单段全大写但没有下划线 ⇒ 与普通词无法区分，不查（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '配置键 `PAYLOAD` 与开关 `VERBOSE`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 承认通道：目标态指认写明落在哪一批即放行（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/plan/09.md': '新增 `ROUTE_TRIAGE_DECIDE`<!-- doc-symbol-allow: 目标态，09 的 S2 才落这个常量 -->\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-009 承认通道：空理由视为未承认（exit 1）', () => {
  const r = audit({
    ...RS,
    'README.md': '新增 `ROUTE_TRIAGE_DECIDE`<!-- doc-symbol-allow: -->\n',
  })
  assert.equal(r.status, 1, r.stdout)
})

test('历史行豁免：行内含「已删除」（exit 0）', () => {
  const r = audit({
    ...RS,
    'README.md': '| `OptionVisitor::GONE_METHOD` | **已删除** |\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('历史行豁免：台账的「判为删除」说的就是这名字不在了（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/plan/04.md': '| ⑫ | `ROUTE_GONE_ARM` 判为删除（只删 Rust 常量，路由保留） |\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('DECISIONS.md 整体排除：ADR 是历史快照（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/DECISIONS.md': '当年引入 `symbio_core::GONE_FN` 又删掉了。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('docs/decisions/ 整目录排除：ADR 分域正文与索引同源（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/decisions/core.md': '当年引入 `symbio_core::GONE_FN` 又删掉了。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('docs/archive/ 整目录排除：归档是历史快照（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/archive/old-plan.md': '当时叫 `symbio_core::GONE_FN`。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('排除只认整目录前缀：docs/decisions.md 这类同名前缀文件仍受查（exit 1）', () => {
  const r = audit({
    ...RS,
    'docs/decisions.md': '指认 `symbio_core::GONE_FN`。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /docs\/decisions\.md:1/)
})

test('承认通道：理由非空则放行（exit 0）', () => {
  const r = audit({
    ...RS,
    'README.md': '桶名见 `external_crate::GONE_FN`。<!-- doc-symbol-allow: 外部 crate 符号 -->\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('承认通道：空理由视为未承认（exit 1）', () => {
  const r = audit({
    ...RS,
    'README.md': '桶名见 `external_crate::GONE_FN`。<!-- doc-symbol-allow: -->\n',
  })
  assert.equal(r.status, 1, r.stdout)
})

test('审计范围读不出：没有任何 md ⇒ 失败而非绿灯（exit 1）', () => {
  const r = audit(RS)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /0 篇候选 md/)
})

test('审计范围读不出：没有 .rs 语料 ⇒ 失败而非绿灯（exit 1）', () => {
  const r = audit({ 'README.md': '正常文档。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /0 个 \.rs 语料根/)
})

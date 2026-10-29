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
  'crates/symbio/lib.rs': [
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
    '007-对话面插件拆分实施方案.md': '新增 `ROUTE_TRIAGE_DECIDE`<!-- doc-symbol-allow: 目标态，09 的 S2 才落这个常量 -->\n',
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
    '004-工程落地.md': '| ⑫ | `ROUTE_GONE_ARM` 判为删除（只删 Rust 常量，路由保留） |\n',
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

test('排除只认整目录：docs/archive.md 这类同名前缀文件仍受查（exit 1）', () => {
  const r = audit({
    ...RS,
    'docs/archive.md': '指认 `symbio_core::GONE_FN`。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /docs\/archive\.md:1/)
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

// ── D-011：能力名册的「锚点」列（DE-008 §6.1）──────────────────────────────
//
// 名册是坐标系的**唯一一份清单**，但清单本身不证明"这条能力在代码里存在"。
// 这一组用例钉的是「写了锚点就必须指得到」——不可证伪的锚点等于把 E6
// （验的是文档自洽，不是文档 == 代码）原样搬进坐标系。

/** 最小名册：一行「工作记忆」，锚点列由用例填 */
const roster = (anchor) => ({
  'docs/003-design/008-能力注册表与能力坐标系.md': [
    '| 能力 | 维 | 轴 | 天梯 | 等级 | 类型 | 落点 | 锚点 |',
    '|---|---|---|:-:|:-:|:-:|:-:|---|',
    `| 工作记忆 | 知 | 抽象层级 | T1 | L1 | 认知 | S01 | ${anchor} |`,
    '',
  ].join('\n'),
})
/** 锚点指向的文件（最小仓库里真有这个文件，D-011 才验得到「文件存在」之外的部分） */
const ROSTER_RS = { 'crates/symbio-plugin-loop/src/context.rs': 'pub fn ctx_load() {}\n' }

test('D-011 锚点指向不存在的符号 ⇒ 红（改一处即红，这是本条的硬要求）', () => {
  const r = audit({ ...RS, ...ROSTER_RS, ...roster('`crates/symbio-plugin-loop/src/context.rs::GONE_FN`') })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-011 .*锚点符号在 Rust 源码中不存在/)
  assert.match(r.stdout, /GONE_FN/)
})

test('D-011 锚点指向仓内不存在的文件 ⇒ 红（文件改名了还是从来没落地？）', () => {
  const r = audit({ ...RS, ...roster('`crates/symbio-plugin-session/src/gone.rs`') })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-011 .*文件不在仓内/)
})

test('D-011 锚点列不是反引号形态 ⇒ 红（列必须有形状，不是自由文本）', () => {
  const r = audit({ ...RS, ...ROSTER_RS, ...roster('在 context 里') })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-011 .*反引号形态/)
})

test('D-011 有效锚点通过：`文件` 与 `文件::符号` 两种形态都认（exit 0）', () => {
  const r = audit({
    ...RS,
    ...ROSTER_RS,
    'docs/003-design/008-能力注册表与能力坐标系.md': [
      '| 能力 | 维 | 轴 | 天梯 | 等级 | 类型 | 落点 | 锚点 |',
      '|---|---|---|:-:|:-:|:-:|:-:|---|',
      '| 工作记忆 | 知 | 抽象层级 | T1 | L1 | 认知 | S01 | `crates/symbio-plugin-loop/src/context.rs` |',
      '| 语义记忆 | 知 | 时域跨度 | T3 | L3 | 认知 | S06 | `crates/symbio-plugin-loop/src/context.rs::ctx_load` |',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /名册锚点 2\/2 已锚定/, '锚定率进日志——名册在长、锚定率不动要看得见')
})

test('D-011 `—` 是「尚未落地」：不判红，且不计入已锚定（exit 0）', () => {
  const r = audit({ ...RS, ...ROSTER_RS, ...roster('—') })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /名册锚点 0\/1 已锚定/)
})

test('D-011 名册不在扫描范围 ⇒ 本条不适用，不为「没有名册」陪跑（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '正常文档。\n' })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /D-011 不适用/)
})

test('D-011 名册一行都读不出 ⇒ 红（表格结构变了还是维的闭集写错了？）', () => {
  const r = audit({
    ...RS,
    'docs/003-design/008-能力注册表与能力坐标系.md': [
      '| 能力 | 维 | 轴 | 天梯 | 等级 | 类型 | 落点 | 锚点 |',
      '|---|---|---|:-:|:-:|:-:|:-:|---|',
      '| 工作记忆 | 未知维 | 抽象层级 | T1 | L1 | 认知 | S01 | — |',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-011 .*名册一行都没读出/)
})

// ── D-012：CLI 产物指路路径 == 构建落点 ────────────────────────────────
//
// 这条判据的全部价值在于「读者照抄的那一行是对的」，所以双向都要钉：
// 旧布局写法必红，而**不同目录下的正确写法**（根 README 无前缀、crate 文档带 `../`）
// 不能因为归一化规则写错而假红——假红的下场是被人加豁免喂掉，真红随之一起失效。

/** 讲构建的文档（D-012 的对象）：teach 文件在场本条才适用 */
const teach = (lines) => ({ 'cli/README.md': ['# cli', '', '构建：`node scripts/build-cli.mjs`', ...lines, ''].join('\n') })

test('D-012 旧布局写法变红：带具名目录段（symbio/target/…）', () => {
  const r = audit({ ...RS, 'cli/Cargo.toml': '[package]\nname = "cli"\n', ...teach(['产物：`../symbio/target/debug/symbio-cli.exe`']) })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-012 cli\/README\.md:4/)
  assert.match(r.stdout, /具名目录段/)
})

test('D-012 落点一致即通过：crate 文档用 `../target/…`（执行目录是 cli/）', () => {
  const r = audit({ ...RS, 'cli/Cargo.toml': '[package]\nname = "cli"\n', ...teach(['产物：`../target/debug/symbio-cli.exe`']) })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /CLI 产物指路 1 处与构建落点一致/)
})

test('D-012 执行目录判错就假红：仓库根的文档不许带 `../`', () => {
  const r = audit({ ...RS, 'README.md': '构建见 `scripts/build-cli.mjs`，产物：`target/debug/symbio-cli.exe`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-012 少一层前缀也是错：cli 的文档写 `target/…` ⇒ 指的是 cli/target', () => {
  const r = audit({ ...RS, 'cli/Cargo.toml': '[package]\nname = "cli"\n', ...teach(['产物：`target/debug/symbio-cli.exe`']) })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /不是构建落点/)
})

test('D-012 release 同理：落点跟 profile 走', () => {
  const ok = audit({ ...RS, 'cli/Cargo.toml': '[package]\nname = "cli"\n', ...teach(['门禁用 `../target/release/symbio-cli.exe`']) })
  assert.equal(ok.status, 0, ok.stdout)
  const bad = audit({ ...RS, 'cli/Cargo.toml': '[package]\nname = "cli"\n', ...teach(['门禁用 `cli/target/release/symbio-cli.exe`']) })
  assert.equal(bad.status, 1, bad.stdout)
  assert.match(bad.stdout, /target\/release\/symbio-cli/)
})

test('D-012 平台后缀不参与判定：无 `.exe` 的写法同样合法（Linux 跑门禁不许假红）', () => {
  const r = audit({ ...RS, 'README.md': '构建见 `scripts/build-cli.mjs`，产物：`target/debug/symbio-cli`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('D-012 历史行豁免：说「这条旧路径已删除」的行不判', () => {
  const r = audit({
    ...RS,
    'cli/Cargo.toml': '[package]\nname = "cli"\n',
    ...teach([
      '产物：`../target/debug/symbio-cli.exe`。',
      '原先写的 `../symbio/target/debug/symbio-cli.exe` 已删除，不再指向旧产物。',
    ]),
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /CLI 产物指路 1 处/)
})

test('D-012 任务登记册排除：条目引用旧路径是记录本身，不是指路', () => {
  const r = audit({
    ...RS,
    'cli/Cargo.toml': '[package]\nname = "cli"\n',
    ...teach(['产物：`../target/debug/symbio-cli.exe`']),
    'docs/007-task/141-x.md': '事故经过：照 `../symbio/target/debug/symbio-cli.exe` 跑，全是旧产物。\n',
  })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /CLI 产物指路 1 处/)
})

test('D-012 讲构建却一处产物路径都没写 ⇒ 红（指路断了 ≠ 判据脱节）', () => {
  const r = audit({ ...RS, 'cli/Cargo.toml': '[package]\nname = "cli"\n', ...teach(['构建成功即可运行。']) })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-012 .*一处 CLI 产物路径都没写出/)
})

test('D-012 没有讲构建的文档 ⇒ 不适用（不陪跑，日志说明原因）', () => {
  const r = audit({ ...RS, 'README.md': '正常文档。\n' })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /D-012 不适用/)
})

// ── D-014：§8 取值 ↔ 投影模块 三方对账 ──────────────────────────────────
//
// 这条判据守的是**两套名字**：§8 的取值（事件里能写的视图名）与 `projection/*.rs`
// （实现文件）不是一一对应，映射此前只活在文件头注释里。三个方向各注入一次违规，
// 都在真实数据形态上验证——只造一个「都对」的样本等于没验。

/** 最小参数表：`projection` 行的取值域就是 ① 侧的对象 */
const PARAMS = {
  'docs/003-design/001-核心架构.md': [
    '# 架构',
    '',
    '| 参数键 | 取值域 | 平凡值 | 用于 |',
    '|---|---|---|---|',
    '| `projection` | `readyset` / `turnstate` / **`conation`** | 仅 `readyset` | 视图定义 |',
    '',
  ].join('\n'),
}

/** 三个实现文件 + 一个读侧函数文件（§2 的对象） */
const PROJ_RS = {
  'crates/symbio-core/src/projection/readyset.rs': '//! readyset\n',
  'crates/symbio-core/src/projection/turnstate.rs': '//! turnstate\n',
  'crates/symbio-core/src/projection/conation.rs': '//! conation\n',
  'crates/symbio-core/src/projection/span.rs': '//! span 读数口\n',
}

/** 映射表：`v` 覆盖 §1 的行，`m` 覆盖 §2 的行 */
const mapDoc = ({ v, m }) => ({
  'docs/004-ref/009-投影映射.md': [
    '# 投影映射',
    '',
    '## 1. 取值',
    '',
    '| §8 取值 | 承接方 | 形态 | 一句话 |',
    '|---|---|---|:-:|',
    ...v,
    '',
    '## 2. 无取值的模块',
    '',
    '| 模块 | 类别 | 职责 |',
    '|---|---|---|',
    ...m,
    '',
  ].join('\n'),
})

const OK = {
  v: [
    '| `readyset` | `projection/readyset.rs` | 模块 | 就绪判定 |',
    '| `turnstate` | `projection/turnstate.rs` | 模块 | 轮态 |',
    '| `conation` | `projection/conation.rs` | 模块 | 欲的读侧 |',
  ],
  m: ['| `projection/span.rs` | 读数口 | 时域跨度的复算口径，报数不建视图 |'],
}
const fixture = (map = OK) => ({ ...RS, ...PARAMS, ...PROJ_RS, 'README.md': '正常文档。\n', ...mapDoc(map) })

test('D-014 三侧一致 ⇒ 通过（exit 0）', () => {
  const r = audit(fixture())
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /投影取值 3 个（3 个有模块），4 个模块全归类/)
})

test('D-014 §8 加了取值、映射表没跟上 ⇒ 红（exit 1）', () => {
  const files = fixture()
  files['docs/003-design/001-核心架构.md'] = files['docs/003-design/001-核心架构.md']
    .replace('`conation`', '`conation` / `display`')
  const r = audit(files)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-014 .*§8 有取值 `display`/)
})

test('D-014 映射表留着 §8 已删的取值 ⇒ 红（表过期）', () => {
  const r = audit(fixture({ ...OK, v: [...OK.v, '| `ghost` | — | 已裁决不做 | 已经不存在的取值 |'] }))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-014 .*映射表有取值 `ghost`/)
})

test('D-014 承接方指向不存在的文件 ⇒ 红（exit 1）', () => {
  const r = audit(fixture({
    ...OK,
    v: OK.v.map((l) => l.replace('projection/readyset.rs', 'projection/ready_set.rs')),
  }))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-014 .*承接方不在投影目录里：`projection\/ready_set\.rs`/)
})

test('D-014 目录里多了未归类模块 ⇒ 红（悄悄多一个投影）', () => {
  const files = fixture()
  files['crates/symbio-core/src/projection/legend.rs'] = '//! 未登记的新视图\n'
  const r = audit(files)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-014 .*`projection\/legend\.rs` 没有归属/)
})

test('D-014 一个模块被两个取值认领 ⇒ 红（说明有一行是猜的）', () => {
  const r = audit(fixture({
    ...OK,
    v: [OK.v[0], OK.v[1], OK.v[2].replace('projection/conation.rs', 'projection/readyset.rs')],
  }))
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-014 .*同时被 `readyset` 与 `conation` 认领/)
})

test('D-014 `—` 行合法：平凡值 / 写侧函数 / 已裁决不做都不是缺实现（exit 0）', () => {
  const files = fixture({
    ...OK,
    v: [
      '| `readyset` | `projection/readyset.rs` | 模块 | 就绪判定 |',
      '| `turnstate` | `projection/turnstate.rs` | 模块 | 轮态 |',
      '| `conation` | `projection/conation.rs` | 模块 | 欲的读侧 |',
      '| `eval` | — | 平凡值 | 不启用任何投影，本身就是平凡值 |',
    ],
  })
  // `eval` 必须同时在 §8 的取值域里——否则这一行是「表过期」，不是「已裁决的现状」
  files['docs/003-design/001-核心架构.md'] = files['docs/003-design/001-核心架构.md']
    .replace('`conation`', '`conation` / `eval`')
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /投影取值 4 个（3 个有模块）/)
})

test('D-014 映射表或投影目录不在扫描范围内 ⇒ 不适用（不陪跑）', () => {
  const r = audit({ ...RS, ...PARAMS, 'README.md': '正常文档。\n' })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /D-014 不适用/)
})

// ── D-020：`类型::成员` 的后半段必须在**那个类型**的定义体里 ──────────────
// 现场：设计文档写着 `Verdict::Unmanned`，可 `Unmanned` 只是同期另一个枚举的变体
// （轮态那边），`Verdict` 的定义体里从来没有过。D-005 只问「这个名字全仓出现过没有」，
// 于是同一个名字在错的类型上照样绿灯——**改名后最典型的腐烂形态**。
const TYPES = {
  'crates/symbio/lib.rs': [
    'pub struct Verdict { pub ok: bool }',
    'pub enum Gap { Unmanned, Filled }',
    'pub trait Cap { fn execute(&self); }',
    'impl Verdict { pub fn finish(&self) {} }',
    'pub fn from() -> String { String::from("x") }',
  ].join('\n'),
}

test('D-020 成员记在别的类型上 ⇒ 红（D-005 只问「全仓出现过没有」）', () => {
  const r = audit({ ...TYPES, 'README.md': '结算产出 `Verdict::Unmanned`。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-020 README\.md:1/)
  assert.match(r.stdout, /Verdict::Unmanned/)
})

test('D-020 字段 / impl 方法 / trait 方法都算「定义体里有」⇒ 通过（exit 0）', () => {
  const r = audit({
    ...TYPES,
    'README.md': '读 `Verdict::ok`，调 `Verdict::finish`，能力面 `Cap::execute`。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('D-020 外部类型不判（仓内没有定义体可比，`String::from` 不是它能核对的）', () => {
  const r = audit({ ...TYPES, 'README.md': '转换走 `String::from`。\n' })
  assert.equal(r.status, 0, r.stdout)
})

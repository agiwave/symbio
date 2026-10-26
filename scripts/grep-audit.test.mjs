import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { machine, budget } from './gate.d/_machine.mjs'

const script = fileURLToPath(new URL('./grep-audit.mjs', import.meta.url))
const repoRoot = fileURLToPath(new URL('..', import.meta.url))

// 全仓扫描的预算**由本机实测决定**（`_machine.mjs::budget`：上一轮耗时中位数 × 2.5）。
//
// 为什么不能写死：这条扫描的耗时几乎全部是「读遍全仓 .rs/.md/.mjs」，它随
// **磁盘速度、文件数、以及同一时刻还有谁在读盘**变化——本机空闲 2–6s，与 cargo
// 冷编译同批时 >30s，差一个数量级。写死一个数只能在「太短 ⇒ 假红」与
// 「太长 ⇒ 真挂死要等很久」之间选一个。
//
// `budget` 的 key 与门禁里的任务名对应（`30-docs.mjs` 的 `${name} 回归测试`）。
// 对不上时退回静态基准 120s（× 单核探针系数），**不会错，只是不自适应**——
// 所以这里硬写这个字符串是可以接受的：它唯一的失效方式是改名，而改名会让
// 预算退回保守值，不会造成假红。
const SCAN_TIMEOUT_MS = budget(await machine(), 'docs::grep-audit 回归测试', 120000)
function audit(source, { waiver = '', strict = false, scope = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-'))
  try {
    const plugins = path.join(root, 'crates/plugins')
    fs.mkdirSync(path.join(plugins, 'other'), { recursive: true })
    fs.writeFileSync(path.join(plugins, 'other/sample.rs'), source.replace('WAIVER', waiver))
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
    // fixture 树里没有全仓的存量债务：S-014 的棘轮基线钉在 0，它不参与这些用例的成败
    env.LEGACY_ROLE_WRITE_BASELINE = '0'
    if (scope) env.SCOPE = plugins
    const result = spawnSync(process.execPath, [script, ...(strict ? ['--strict'] : [])], {
      cwd: root, env, encoding: 'utf8', timeout: 10000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}
const suspect = `use std::sync::Mutex;
async fn example() {
    let guard = mutex.lock().unwrap(); WAIVER
    work().await;
    drop(guard);
}
`
test('default scope checks a non-agent plugin', () => {
  const r = audit(suspect)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /other\/sample.rs:3/)
})
test('test markers do not suppress S-002', () => {
  assert.equal(audit('#[tokio::test]\n' + suspect).status, 1)
})
test('explicit scope still works', () => {
  assert.equal(audit(suspect, { scope: true }).status, 1)
})
test('reviewed same-line waiver requires a reason', () => {
  assert.equal(audit(suspect, { waiver: '// grep-audit-allow S-002: reviewed fixture' }).status, 0)
  assert.equal(audit(suspect, { waiver: '// grep-audit-allow S-002:   ' }).status, 1)
})
test('waiver does not suppress another lock', () => {
  const source = suspect + suspect.replace('WAIVER', '')
  assert.equal(audit(source, { waiver: '// grep-audit-allow S-002: reviewed fixture' }).status, 1)
})
test('warnings fail only in strict mode', () => {
  const source = 'async fn example() {\n let _ = work().await;\n}\n'
  assert.equal(audit(source).status, 0)
  assert.equal(audit(source, { strict: true }).status, 2)
})

// ── S-002-bonus：业务路径 `let _ = ...await` 吞错 ──────────────────────
// 本规则是**正则近似**，必然有假阳性：被 await 的 future 可能本就不返回 Result
// （如 `fire_hook` 返回 `HookOutput`），「关闭 / 清理 / kill / flush」这类收尾动作
// 失败时也没有后续动作可做。故与 S-002 / S-008 / S-009 / S-010 一致，留一条
// **必须写理由**的逐行豁免——否则「请人工 review」的结论无处落笔，每次跑门禁都得
// 从头再 review 一遍，永不消失的告警只会教人忽略整个审计。
const bonusSuspect = 'async fn example() {\n let _ = work().await; WAIVER\n}\n'
test('S-002-bonus names the offending line and the remedy', () => {
  const r = audit('async fn example() {\n let _ = work().await;\n}\n')
  assert.equal(r.status, 0) // 仅 WARNING：非 strict 不判失败
  assert.match(r.stdout, /sample\.rs:2:let _ = work\(\)\.await;/)
  assert.match(r.stdout, /plugin_warn/)
})
test('S-002-bonus waiver requires a reason', () => {
  assert.equal(
    audit(bonusSuspect, { waiver: '// grep-audit-allow S-002-bonus: 收尾动作，失败无可为', strict: true }).status,
    0,
  )
  assert.equal(audit(bonusSuspect, { waiver: '// grep-audit-allow S-002-bonus:   ', strict: true }).status, 2)
})
test('S-002-bonus waiver covers only its own line', () => {
  const source = 'async fn example() {\n let _ = a().await; WAIVER\n let _ = b().await;\n}\n'
  assert.equal(audit(source, { waiver: '// grep-audit-allow S-002-bonus: 仅此行', strict: true }).status, 2)
})
test('S-002-bonus waiver for another rule does not leak in', () => {
  assert.equal(audit(bonusSuspect, { waiver: '// grep-audit-allow S-002: reviewed fixture', strict: true }).status, 2)
})
test('S-002-bonus ignores the pattern when it appears inside a comment', () => {
  // 本规则判的是**代码形状**；注释里写「这里为什么可以 let _ = x().await」正是在
  // 解释它，把解释判成违规等于惩罚留痕（本次自查时真踩到过：三处说明性注释各报一条）。
  const source = '/// 收尾点写 `let _ = h.await;` 是刻意的。\n// let _ = a().await;\nasync fn f() {}\n'
  assert.equal(audit(source, { strict: true }).status, 0)
  // 但**行尾**注释不能用来藏代码：同一行前半仍是代码，照判。
  assert.equal(audit('async fn f() {\n let _ = b().await; // 说明\n}\n', { strict: true }).status, 2)
})

// ── S-008：VdfsNode.status 不得用裸字面量 ──────────────────────────────
// 词表只有 `VDFS_STATUS_*` 一套；裸字面量在改名时不会编译失败（该词表曾把
// `error` 改名为 `failed`，留下过化石，见 vdfs/words.rs::VDFS_STATUS_FAILED）。
const statusSuspect = `fn node_of() -> VdfsNode {
    let mut n = VdfsNode::file();
    n.status = "active".to_string(); WAIVER
    n
}
`
test('S-008 fires on a bare status literal', () => {
  const r = audit(statusSuspect)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /sample\.rs:3/)
})
test('S-008 fires on a literal inside an if branch', () => {
  const r = audit(`fn node_of(enabled: bool) -> VdfsNode {
    let mut n = VdfsNode::file();
    n.status = if enabled {
        "active".to_string()
    } else {
        "disabled".to_string()
    };
    n
}
`)
  assert.equal(r.status, 1)
})
test('S-008 fires on with_status receiving a bare literal', () => {
  const r = audit('fn f() -> OptionNode {\n    OptionNode::new("a", "A").with_status("disabled")\n}\n')
  assert.equal(r.status, 1)
})
test('S-008 stays silent when the word comes from a constant', () => {
  const r = audit(
    'fn f() -> VdfsNode {\n    let mut n = VdfsNode::file();\n    n.status = VDFS_STATUS_ACTIVE.to_string();\n    n\n}\n',
  )
  assert.equal(r.status, 0)
})
test('S-008 does not mistake format! / response envelope for a status word', () => {
  // 已知边界：`status: "success"`（响应信封）是另一套词表，按位置判会误报，故只认
  // `.status = ` 与 `with_status(` 两处。`format!` 同样不算字面量赋值。
  const r = audit(
    'fn f(k: &str) -> VdfsNode {\n    let mut n = VdfsNode::file();\n    n.status = format!("{k}");\n    n\n}\n',
  )
  assert.equal(r.status, 0)
})
test('S-008 waiver requires a reason', () => {
  assert.equal(audit(statusSuspect, { waiver: '// grep-audit-allow S-008: reviewed fixture' }).status, 0)
  assert.equal(audit(statusSuspect, { waiver: '// grep-audit-allow S-008:   ' }).status, 1)
})

// ── S-009：事件总线 kind 不得用裸字面量 ────────────────────────────────
// 词表只有 `symbio_core::event_bus::KIND_*` 一套；`kind` 是跨进程字符串，裸字面量
// 改名时不会编译失败。本规则拦的正是「发布点写字面量 → 常量无人引用」的**成因**
// （那正是 R-001 当初报出 `KIND_SESSION` / `KIND_SYSTEM` 的由来）。
const kindSuspect = `async fn emit() {
    EventBus::publish("session", None, data).await; WAIVER
}
`
test('S-009 fires on a bare kind literal', () => {
  const r = audit(kindSuspect)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /sample\.rs:2/)
})
test('S-009 fires on try_publish and on a receiver call', () => {
  assert.equal(audit('fn f() {\n    EventBus::try_publish("vdfs", None, d);\n}\n').status, 1)
  assert.equal(audit('fn f() {\n    bus.publish("system", None, d);\n}\n').status, 1)
})
test('S-009 stays silent when the kind comes from a constant', () => {
  assert.equal(audit('fn f() {\n    EventBus::try_publish(KIND_SESSION, None, d);\n}\n').status, 0)
})
test('S-009 does not mistake a non-literal first argument', () => {
  assert.equal(audit('fn f(kind: &str) {\n    EventBus::try_publish(kind, None, d);\n}\n').status, 0)
})
test('S-009 waiver requires a reason', () => {
  assert.equal(audit(kindSuspect, { waiver: '// grep-audit-allow S-009: reviewed fixture' }).status, 0)
  assert.equal(audit(kindSuspect, { waiver: '// grep-audit-allow S-009:   ' }).status, 1)
})

// ── S-010：vdfs 挂载根名不得出现在 vdfs 插件之外 ──────────────────────
// 根名是 vdfs 插件的挂载规则（plugins/vdfs/fs.rs::VDFS_ADDR_ROOT，全仓唯一字面量）。
// 历史上它曾蔓延到前端与十几份文档，「改个挂载名」变成全仓手术——本规则把收口钉死。
// S-010 的扫描根与 resolveScope 同策略（cwd 有 symbio/ 即视为仓库树），故用
// 独立的夹具构造器：按相对路径布文件，直接跑脚本。
function s010Audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-s010-'))
  try {
    // 仓库树标记：S-010 以「cwd 下有 symbio/」判定扫描根，缺了会回落到真实仓库
    fs.mkdirSync(path.join(root, 'crates', 'plugins'), { recursive: true })
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
    // 同 `audit()`：fixture 里不比全仓存量，S-014 棘轮基线钉 0
    env.LEGACY_ROLE_WRITE_BASELINE = '0'
    return spawnSync(process.execPath, [script], { cwd: root, env, encoding: 'utf8', timeout: 10000 })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const S010_SUSPECT_TS = "const x = '.vdfs/session/abc'\n"
const S010_SUSPECT_MD = '# 指南\n\n会话在 `.vdfs/session` 之下。\n'

test('S-010 fires outside the vdfs plugin (ts / md / rust)', () => {
  for (const [rel, content] of [
    ['tauri/src/schemas/x.ts', S010_SUSPECT_TS],
    ['docs/006-guide/x.md', S010_SUSPECT_MD],
    ['crates/symbio-plugin-session/src/memory.rs', 'pub const A: &str = ".vdfs/session";\n'],
  ]) {
    const r = s010Audit({ [rel]: content })
    assert.equal(r.status, 1, rel)
    assert.match(r.stdout, /S-010/)
  }
})

test('S-010 catches stale and suffixed root spellings too', () => {
  assert.equal(s010Audit({ 'tauri/src/x.ts': "const a = '.vdfsv2'\n" }).status, 1)
  assert.equal(s010Audit({ 'tauri/src/x.ts': "const b = '.vdfs2/x'\n" }).status, 1)
})

test('S-010 stays silent inside the vdfs plugin (the owner)', () => {
  const r = s010Audit({
    'crates/symbio-plugin-vdfs/src/fs.rs': 'pub const VDFS_ADDR_ROOT: &str = ".vdfsv2";\n',
    'crates/symbio-plugin-vdfs/README.md': '根目录 `.vdfs/session` 说明。\n',
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /S-010 通过/)
})

test('S-010 exempts historical records (archive)', () => {
  // doc-link-allow D-010: 下一行讲的是「已删除的旧豁免」，文件确实不存在——史实叙述
  // `docs/CHANGELOG.md` 曾与 archive 并列豁免；该机制已废除（变更历史以 `git log` 为准），
  // 文件也随之删除，故这里只保留 archive 这一条豁免。
  const r = s010Audit({
    'crates/symbio-plugin-vdfs/src/fs.rs': '', // 标记：这是一个仓库树
    'docs/archive/old-design.md': S010_SUSPECT_MD,
  })
  assert.equal(r.status, 0)
})

test('S-010 is not fooled by lookalike tokens (browser route / import paths)', () => {
  const r = s010Audit({
    'crates/symbio-plugin-vdfs/src/fs.rs': '', // 标记：这是一个仓库树
    'tauri/src/x.ts': [
      "import { vdfsJoin } from '../vdfs'", // 相对导入
      "import { vdfsRoot } from '@/schemas/vdfsRoot'", // 别名导入
      "const route = '/vdfs/session'", // 浏览器路由（无前导点）
    ].join('\n'),
  })
  assert.equal(r.status, 0)
})

test('S-010 skips runtime data dirs (`.symbio` homedir), not just build output', () => {
  // 实测事故：一次会话把 `.vdfs` 写进 `.symbio/session/<id>/MEMORY.md`（模型在
  // 正文里引用挂载路径），门禁因**运行期数据**判红——源码一个字节没改。
  // `.symbio/` 是运行期 homedir 且被 `.gitignore` 忽略，不属本规则对象。
  const r = s010Audit({
    'crates/symbio-plugin-vdfs/src/fs.rs': '', // 标记：这是一个仓库树
    '.symbio/session/09d74431/MEMORY.md': '会话落在 `.vdfs/session/<id>` 之下。\n',
  })
  assert.equal(r.status, 0, '运行期数据目录不应参与源码审计')
})

test('S-010 still fires inside the repo tree when runtime dirs are present', () => {
  // 反例守卫：跳过 `.symbio` 不能顺手把整棵树放过去。
  const r = s010Audit({
    'crates/symbio-plugin-vdfs/src/fs.rs': '', // 标记：这是一个仓库树
    '.symbio/session/09d74431/MEMORY.md': '会话落在 `.vdfs/session/<id>` 之下。\n',
    'docs/006-guide/x.md': S010_SUSPECT_MD,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-010/)
})

test('S-010 waiver requires a reason (same line or the line above)', () => {
  // 仓库树标记：crates/plugins（同时是其它规则的扫描范围）
  const mark = { 'crates/symbio-plugin-vdfs/src/fs.rs': '' }
  const r1 = s010Audit({ ...mark, 'docs/x.md': `<!-- grep-audit-allow S-010: 历史快照示例 -->\n.vdfs/session\n` })
  assert.equal(r1.status, 0)
  const r2 = s010Audit({ ...mark, 'docs/x.md': `.vdfs/session <!-- grep-audit-allow S-010: 引用旧文 -->\n` })
  assert.equal(r2.status, 0)
  const r3 = s010Audit({ ...mark, 'docs/x.md': `.vdfs/session <!-- grep-audit-allow S-010:   -->\n` })
  assert.equal(r3.status, 1)
})

// ── S-011：补充注入点的位置（两处行序断言）──────────────────────────────
// 夹具与 S-010 共用（`s010Audit` 建的就是「cwd 下有 crates/ 的仓库树」），
// 落点由文件相对路径决定——S-011 认 `turn_runner/stream.rs` 与 `turn_exec/call.rs` 两处
// （D-191 后注入点从 session 的单个 `turn_runner.rs` 搬到 loop 的这两个子文件）。
const s011Audit = s010Audit
const S011_RUNNER = 'crates/symbio-plugin-loop/src/turn_runner/stream.rs'
const S011_EXEC = 'crates/symbio-plugin-loop/src/turn_exec/call.rs'

/** 位置正确的形态：2e 结果接进请求 → 2f 补充接进请求 */
const S011_RUNNER_OK = `fn run() {
    run.exchange.extend(tool_exchange_messages(&rt, &outcomes, run.tool_round));
    run.tool_round += 1;
    run.exchange.extend(injected);
}
`
/** 位置正确的形态：先抽干（拿得到 `supplemented_at`）→ 再判定要不要汇报 */
const S011_EXEC_OK = `fn run() {
    let injected = supplement_into_request(batch, anchor, store.as_ref());
    let supplemented_at = (!injected.is_empty()).then(clock_now_ms);
    if let Some(port) = report.as_ref() {}
}
`
const s011Ok = (runner = S011_RUNNER_OK, exec = S011_EXEC_OK) => ({
  [S011_RUNNER]: runner,
  [S011_EXEC]: exec,
})

test('S-011 passes when both orderings hold', () => {
  const r = s011Audit(s011Ok())
  assert.equal(r.status, 0)
  assert.match(r.stdout, /S-011 通过：.*stream\.rs 2e 工具结果接进请求\(2\) 在 2f 轮内补充接进请求\(4\) 之前/)
  assert.match(r.stdout, /S-011 通过：.*call\.rs `supplement_into_request` 抽干点\(2\) 在 `report` 汇报派发点\(4\) 之前/)
})

test('S-011 fires when the supplement is injected before the tool results land', () => {
  // 真实的后果不是「顺序不好看」：补充插进 `assistant(tool_calls)` 与它的 `tool` 结果
  // 之间，部分协议直接 400——而**没有任何测试会因为搬家而变红**。
  const r = s011Audit(
    s011Ok(`fn run() {
    run.exchange.extend(injected);
    run.exchange.extend(tool_exchange_messages(&rt, &outcomes, run.tool_round));
}
`),
  )
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-011 位置错/)
})

test('S-011 fires when the report is judged before the drain', () => {
  // 反了 ⇒ `supplemented_at` 在判定那一刻还不知道，静默时钟没归零：
  // 用户已经开口，插件照样按「没人说话」报一次进度。
  const r = s011Audit(
    s011Ok(S011_RUNNER_OK, `fn run() {
    if let Some(port) = report.as_ref() {}
    let injected = supplement_into_request(batch, anchor, store.as_ref());
}
`),
  )
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-011 位置错/)
})

test('S-011 errors when an anchor disappears (guard must not silently no-op)', () => {
  // 锚点缺失 = 注入点被删或被改名。守卫此时**必须响亮**：让它悄悄退化成空转，
  // 它会以绿灯的形式提供虚假保证——比没有守卫更糟。故"锚点缺失"不接受豁免。
  const renamed = s011Audit({
    ...s011Ok(),
    [S011_RUNNER]: S011_RUNNER_OK.replace('tool_exchange_messages', 'round_exchange_messages'),
  })
  assert.equal(renamed.status, 1)
  assert.match(renamed.stdout, /S-011 锚点缺失/)

  const reportGone = s011Audit({
    ...s011Ok(),
    [S011_EXEC]: `fn run() {
    let injected = supplement_into_request(batch, anchor, store.as_ref());
}
`,
  })
  assert.equal(reportGone.status, 1)
  assert.match(reportGone.stdout, /`report` 汇报派发点/)

  // 豁免通道对"锚点缺失"无效——无从判断的事不能靠签名放行
  const waivedButMissing = s011Audit({
    ...s011Ok(),
    [S011_EXEC]: `fn run() {
    let injected = supplement_into_request(batch, anchor, store.as_ref());
    let drafted = report_now(supplemented_at); // grep-audit-allow S-011: 已确认
}
`,
  })
  assert.equal(waivedButMissing.status, 1)
})

test('S-011 counts only code lines (a commented-out inject is "missing", not "fine")', () => {
  const r = s011Audit({
    ...s011Ok(),
    [S011_RUNNER]: `fn run() {
    run.exchange.extend(tool_exchange_messages(&rt, &outcomes, run.tool_round));
    // run.exchange.extend(injected);  ← 注释掉注入调用
}
`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-011 锚点缺失/)
})

test('S-011 ignores the `fn supplement_into_request` definition (only the call site anchors)', () => {
  // 真实文件里函数定义在**调用点之后**，但规则不能依赖这个巧合：
  // 若把定义当成锚点就会误判成"位置错"。
  const r = s011Audit({
    ...s011Ok(),
    [S011_EXEC]: `fn run() {
    let injected = supplement_into_request(batch, anchor, store.as_ref());
    if let Some(port) = report.as_ref() {}
}
fn supplement_into_request(store: &Store, batch: &Batch) -> Vec<Msg> { Vec::new() }
`,
  })
  assert.equal(r.status, 0)
})

test('S-011 waiver requires a reason', () => {
  const bad = `fn run() {
    if let Some(port) = report.as_ref() {} WAIVER
    let injected = supplement_into_request(batch, anchor, store.as_ref());
}
`
  const withReason = s011Audit({
    ...s011Ok(),
    [S011_EXEC]: bad.replace('WAIVER', '// grep-audit-allow S-011: 判定口由外层统一持有'),
  })
  assert.equal(withReason.status, 0)
  const emptyReason = s011Audit({
    ...s011Ok(),
    [S011_EXEC]: bad.replace('WAIVER', '// grep-audit-allow S-011:   '),
  })
  assert.equal(emptyReason.status, 1)
})

test('S-011 is skipped (not failed) when either file is outside the scope', () => {
  const none = audit('fn unrelated() {}\n')
  assert.equal(none.status, 0)
  assert.match(none.stdout, /S-011 跳过：scope 内没有 turn_runner\/stream\.rs/)
  assert.match(none.stdout, /S-011 跳过：scope 内没有 turn_exec\/call\.rs/)

  // 只布一个文件：另一个跳过，布下的那个照常判
  const one = s011Audit({ [S011_RUNNER]: S011_RUNNER_OK })
  assert.equal(one.status, 0)
  assert.match(one.stdout, /S-011 通过：.*stream\.rs/)
  assert.match(one.stdout, /S-011 跳过：scope 内没有 turn_exec\/call\.rs/)
})

// ── S-012: 代码注释里的实施排期指认 ────────────────────────────────────
// 这批注释没有 owner：「本批未修」的下一批修了，句子就指着已不存在的时间点；
// 单独的「批 2」在批次重排后无人能解。判据只取两种无歧义形态（待办标记 / 无出处批次号）。
const S012_FILE = 'crates/symbio-core/src/actors/mod.rs'

test('S-012 fires on a batch number with no plan anchor', () => {
  const r = s010Audit({ [S012_FILE]: '/// 运行器据此停止工具循环（批 2 的恢复前提）。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-012/)
  assert.match(r.stdout, /批次号没有出处/)
})

test('S-012 accepts the same batch number with its plan anchor', () => {
  const src =
    '/// 恢复前提见 [plan/11 批 2](../../../../docs/003-design/011-Agent目录规范.md)。\n'
  assert.equal(s010Audit({ [S012_FILE]: src }).status, 0)
})

test('S-012 fires on a TODO / FIXME marker left in a comment', () => {
  for (const marker of ['TODO', 'FIXME', 'XXX', 'HACK']) {
    const r = s010Audit({ [S012_FILE]: `// ${marker}: 补上回退分支\n` })
    assert.equal(r.status, 1, `${marker} ⇒ ${r.stdout}`)
    assert.match(r.stdout, /待办登记进计划文档/)
  }
})

test('S-012 judges only comment lines (a string holding "// TODO" is code)', () => {
  assert.equal(s010Audit({ [S012_FILE]: 'fn f() { let s = "// TODO 用户写的待办"; }\n' }).status, 0)
})

test('S-012 does not touch runtime prose about a batch of tool calls', () => {
  // 「本批」在这里是**运行期事实**（这一批工具调用），不是排期——误报的代价是守卫被喂死。
  const src = '/// 本批未执行的工具调用不得停在 `Streaming`。\n// 诚实缺口：网格少一格。\n'
  assert.equal(s010Audit({ [S012_FILE]: src }).status, 0)
})

test('S-012 covers cli and the verify programs, not just symbio', () => {
  for (const file of ['crates/symbio-cli/src/client.rs', 'docs/003-design/verify/scenario_ladder.rs']) {
    const r = s010Audit({ [file]: '/// 这一格随批 3 补。\n' })
    assert.equal(r.status, 1, `${file} ⇒ ${r.stdout}`)
  }
})

test('S-012 waiver requires a reason (same line or the line above)', () => {
  const hit = '/// 运行器据此停止工具循环（批 2 的恢复前提）。\n'
  for (const waived of [
    hit.replace('\n', '  // grep-audit-allow S-012: 同段首行已给 plan/11 出处\n'),
    `// grep-audit-allow S-012: 同段首行已给 plan/11 出处\n${hit}`,
  ]) {
    const r = s010Audit({ [S012_FILE]: waived })
    assert.equal(r.status, 0, r.stdout)
  }
  for (const empty of [hit.replace('\n', '  // grep-audit-allow S-012:   \n'), `// grep-audit-allow S-012:  \n${hit}`]) {
    const r = s010Audit({ [S012_FILE]: empty })
    assert.equal(r.status, 1, r.stdout)
  }
})

test('S-012 finds no batch-relative narration in this repository', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30000,
  })
  assert.match(r.stdout, /S-012 通过/)
})

// ── S-013: `WalkBuilder` 构造点必须钉遍历序 ────────────────────────────
// 默认遍历序 = 文件系统返回序（NTFS 名字序 / ext4 哈希序）。下游的 `offset` /
// `head_limit` 分页窗口、`MAX_RESULTS` 与降级路径的 `limit` 截断都切在这个序上，
// 于是同一查询换平台就换一批行。病灶见 ①（两条分页单测 Linux 红）。
const S013_FILE = 'crates/symbio-plugin-local/src/walker.rs'
const S013_UNPINNED = `fn collect(workdir: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(workdir);
    builder.standard_filters(true).require_git(false);
    builder.build().filter_map(Result::ok).map(|e| e.path().to_path_buf()).collect()
}
`

test('S-013 fires on a WalkBuilder construction with no sort in scope', () => {
  const r = s010Audit({ [S013_FILE]: S013_UNPINNED })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-013 未钉遍历序/)
})

test('S-013 accepts sort_by_file_name and the generic sort_by', () => {
  for (const sorter of [
    '    builder.sort_by_file_name(|a, b| a.cmp(b));\n',
    '    builder.sort_by(|a, b| a.cmp(b));\n',
  ]) {
    const src = S013_UNPINNED.replace(
      '    builder.build()',
      sorter + '    builder.build()',
    )
    assert.equal(s010Audit({ [S013_FILE]: src }).status, 0, sorter)
  }
})

test('S-013 does not let the next function\'s sort excuse this one', () => {
  // 作用域边界：往后扫到下一个 `fn` 起头为止——别的函数里钉了序，不替这里说话。
  const src = S013_UNPINNED + 'fn other() { let v = x.sort_by_file_name(|a, b| a.cmp(b)); }\n'
  const r = s010Audit({ [S013_FILE]: src })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-013 未钉遍历序/)
})

test('S-013 waiver requires a reason (same line or the line above)', () => {
  const hit = '    let mut builder = WalkBuilder::new(workdir);\n'
  for (const waived of [
    hit.replace('\n', '  // grep-audit-allow S-013: 取用方末尾按 rel 整体排序\n'),
    `// grep-audit-allow S-013: 取用方末尾按 rel 整体排序\n${hit}`,
  ]) {
    const r = s010Audit({ [S013_FILE]: S013_UNPINNED.replace(hit, waived) })
    assert.equal(r.status, 0, r.stdout)
  }
  const empty = s010Audit({
    [S013_FILE]: S013_UNPINNED.replace(hit, hit.replace('\n', '  // grep-audit-allow S-013:   \n')),
  })
  assert.equal(empty.status, 1, empty.stdout)
})

test('S-013 is silent when the scope has no WalkBuilder at all', () => {
  const r = s010Audit({ [S013_FILE]: 'fn f() { let a = 1; }\n' })
  assert.match(r.stdout, /S-013 通过：scope 内没有 WalkBuilder 构造点/)
})

test('S-013 finds every WalkBuilder construction pinned in this repository', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30000,
  })
  assert.match(r.stdout, /S-013 通过：3 处 WalkBuilder 构造点都钉了遍历序/)
})

// ── S-014: ChatNode 存量 `role` 格直写棘轮 ─────────────────────────────
// 棘轮的三面都要钉住：升即红、降了不挪基线也即红、持平才过——
// 只钉第一面的话，债务会换个名字继续长（基线永远等于昨天的读数）。
const S014_FILE = 'crates/symbio-plugin-session/src/sample.rs'

function roleAudit(source, baseline) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-s014-'))
  try {
    // 仓库树标记：扫描根按「cwd 下有 symbio/」判定，缺了会回落到真实仓库
    const target = path.join(root, S014_FILE)
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(target, source)
    const env = { ...process.env, NO_COLOR: '1', LEGACY_ROLE_WRITE_BASELINE: String(baseline) }
    delete env.SCOPE
    return spawnSync(process.execPath, [script], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 10000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

/**
 * 真实仓库上的 S-014 运行（`LEGACY_ROLE_WRITE_BASELINE` 可覆盖基线）。
 *
 * 与 `roleAudit`（临时树）分开：`S014_IS_REPO` 守卫把「持平 / 该收紧」两条分支
 * 限定在真实仓库上跑，因此这两条分支只能在真实仓库上钉。基线置大 ⇒ 触发
 * 「该收紧」；不置 ⇒ 触发「持平」——都不动仓库里的存量。
 */
function roleAuditRepo(baseline) {
  const env = {
    ...process.env,
    NO_COLOR: '1',
    LEGACY_ROLE_WRITE_BASELINE: String(baseline),
  }
  delete env.SCOPE
  return spawnSync(process.execPath, [script], {
    cwd: repoRoot,
    env,
    encoding: 'utf8',
    timeout: 60000,
  })
}

test('S-014 fixture 树不比棘轮：扫描根不是真实仓库 ⇒ 跳过', () => {
  const src = 'fn n() -> ChatNode {\n    ChatNode { role: Some(NodeRole::Assistant), ..Default::default() }\n}\n'
  const r = roleAudit(src, 1)
  assert.equal(r.status, 0, r.stdout)
  // 临时树里 `symbio/src` 只躺着一个 fixture，存量恒 0/1——它与基线的差是「树不对」，
  // 不是「该收紧」。据此判红会让守卫自己的回归测试变成噪音。
  assert.match(r.stdout, /S-014 跳过：当前树不是真实仓库（临时 fixture），棘轮不比/)
})

test('S-014 新增直写即红，并点名命中处与改法', () => {
  const src = 'fn n() -> ChatNode {\n    ChatNode { role: Some(NodeRole::Assistant), ..Default::default() }\n}\n'
  const r = roleAudit(src, 0)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-014 违例：存量 role 格直写 1 处 > 基线 0/)
  assert.match(r.stdout, /sample\.rs:2/)
  assert.match(r.stdout, /身份写 .*principal.*，不写 role/)
})

test('S-014 存量降了却不挪基线同样判红（棘轮只许留痕地下调）', () => {
  // 这条分支被 `S014_IS_REPO` 限定在真实仓库上跑，所以这里直接把基线置到
  // **超过真实存量**（不动仓里任何存量）：读数 < 基线 ⇒ 该收紧。
  const r = roleAuditRepo(9999)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-014 基线该收紧：存量 role 格直写 \d+ 处 < 基线 9999/)
})

test('S-014 数的是直写形状：字段初始化 / 赋值 / get_or_insert', () => {
  const shapes = [
    'role: Some(NodeRole::Assistant),',
    'role: Some(cm::NodeRole::Tool),',
    'node.role = Some(NodeRole::User);',
    'node.role.get_or_insert(NodeRole::Assistant);',
  ]
  const src = shapes.map((s) => `fn f() {\n    ${s}\n}\n`).join('')
  const r = roleAudit(src, shapes.length)
  assert.equal(r.status, 0, r.stdout)
})

test('S-014 不判注释行，豁免必须写理由（本行或紧邻上一行）', () => {
  const hit = '    node.role = Some(NodeRole::Tool);\n'
  const asComment = `/// 例：${hit.trim()}\n`
  assert.equal(roleAudit(asComment, 0).status, 0, '注释里的形状是反例展示，不是债务')
  for (const waived of [
    hit.replace('\n', '  // grep-audit-allow S-014: 读侧兼容出口，随 d 收口清\n'),
    `// grep-audit-allow S-014: 读侧兼容出口，随 d 收口清\n${hit}`,
  ]) {
    assert.equal(roleAudit(waived, 0).status, 0, roleAudit(waived, 0).stdout)
  }
  const empty = hit.replace('\n', '  // grep-audit-allow S-014:   \n')
  assert.equal(roleAudit(empty, 0).status, 1, '空理由不算豁免')
})

test('S-014 真实仓库的存量与基线持平（读数漂了就红）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    // 这条预算**不是判据**（判据是下面那句 stdout 匹配），它只防「扫描挂死」。
    // 原先 30s：本机空闲时扫全仓约 2–6s，但门禁里 `docs` 与 cargo 同批跑，冷全量
    // 编译那一次（backend 10m58s、8 逻辑核全压满）实测扫了 **>30s**，于是 `stdout`
    // 为空、报「读数漂了」——一次纯粹的负载假红，而它长得和真漂移一模一样。
    // 预算短于它要等的工作，就是一条会自己制造假红的判据。见 `SCAN_TIMEOUT_MS`。
    timeout: SCAN_TIMEOUT_MS,
  })
  assert.match(r.stdout, /S-014 通过：role 格直写存量 \d+ 处，与基线持平/)
})

// ── S-015: 收束闸单一 owner 且调用当场处理 Err ───────────────────────────
// 规则判的是**零出现**（不是棘轮）：布尔壳 `closure_granted` 复活、或闸被压回
// 布尔形态，「拒绝」就又是「什么都没写」——日志里与「没跑完」同形，而不会有任何
// 东西变红。
const S015_FILE = 'crates/symbio-plugin-loop/src/sample.rs'

function gateAudit(source, name = S015_FILE) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-s015-'))
  try {
    const target = path.join(root, name)
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(target, source)
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
    return spawnSync(process.execPath, [script], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 10000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('S-015 布尔壳 `closure_granted` 复活即红，并点名改法', () => {
  const src = 'fn f() {\n    if !closure_granted(&actor, ClosureKind::Speech) {\n        return;\n    }\n}\n'
  const r = gateAudit(src)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-015 违例：收束闸被压回布尔形态（1 处）/)
  assert.match(r.stdout, /sample\.rs:2/)
  assert.match(r.stdout, /布尔壳 `closure_granted` 已退役/)
  assert.match(r.stdout, /if let Err\(why\) = closure_gate\(\.\.\)/)
})

test('S-015 闸被压成布尔或存起来另判即红', () => {
  const boolShape = 'fn f() {\n    if closure_gate(&actor, ClosureKind::Speech).is_ok() {\n        land();\n    }\n}\n'
  const r = gateAudit(boolShape)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /闸的调用没当场处理 `Err`/)
  // 拆行写法骗得过「不许 `.is_ok()`」那种行级正则——这条判据按上一行同看挡住它。
  const split = 'fn f() {\n    if let Err(why) =\n        closure_gate(&actor, ClosureKind::Speech)\n    {\n        deny(why);\n    }\n}\n'
  assert.equal(gateAudit(split).status, 0, gateAudit(split).stdout)
})

test('S-015 放行当场处理 Err 的调用与闸的定义行', () => {
  const gate = 'fn f() {\n    if let Err(why) = closure_gate(&actor, ClosureKind::Speech) {\n        deny(store, &actor, 1, 2, ClosureKind::Speech, "k", &why);\n    }\n}\n'
  assert.equal(gateAudit(gate).status, 0, gateAudit(gate).stdout)
  const def = 'fn closure_gate(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {\n    Ok(())\n}\n'
  assert.equal(gateAudit(def).status, 0, gateAudit(def).stdout)
})

test('S-015 不判注释行，测试文件豁免，豁免必须写理由', () => {
  const asComment = '/// 旧写法（反例）：if !closure_granted(&actor, kind) { … }\n'
  assert.equal(gateAudit(asComment).status, 0, '注释里的形状是反例展示，不是拒绝路径')
  // 测试文件里断言谓词本身不是拒绝路径（判据用例正是它）。
  const inTest = 'fn t() {\n    assert!(!closure_granted(&spec, ClosureKind::Speech));\n}\n'
  assert.equal(
    gateAudit(inTest, 'crates/symbio-plugin-loop/src/sample.test.rs').status,
    0,
    '测试文件豁免',
  )
  const hit = '    if !closure_granted(&a, k) {\n'
  for (const waived of [
    hit.replace('\n', '  // grep-audit-allow S-015: 只读探针，不落格\n'),
    `// grep-audit-allow S-015: 只读探针，不落格\n${hit}`,
  ]) {
    assert.equal(gateAudit(waived).status, 0, gateAudit(waived).stdout)
  }
  const empty = hit.replace('\n', '  // grep-audit-allow S-015:   \n')
  assert.equal(gateAudit(empty).status, 1, '空理由不算豁免')
})

test('S-015 真实仓库零处（存量已全部迁完）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: SCAN_TIMEOUT_MS,
  })
  assert.match(r.stdout, /S-015 通过：布尔壳零处/)
})

// ── S-016: 停机原因单形状 ────────────────────────────────────────────
// 判据盯的是三个构造点文件（`turn_runner.rs` / `facts.rs` / `settle.rs`）：
// 停机格载荷只从这三处出生。扫整个目录会把 `memory_events` 那个格子的 `why`
// （没有竞争的 `reason` 字段）一起卷进来——范围错了的规则会教人学会忽略它。
const S016_TARGET = 'crates/symbio-plugin-loop/src/turn_runner/settle.rs'
const S016_OTHER_CELL = 'crates/symbio-plugin-loop/src/memory_events.rs'

function gateAudit16(source, name = S016_TARGET) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-s016-'))
  try {
    const target = path.join(root, name)
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(target, source)
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
    return spawnSync(process.execPath, [script], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 10000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('S-016 已退役的两个形状各自即红', () => {
  const terminated = 'fn f() {\n    payload["terminated"] = json!({ "reason": "aborted", "at": 1 });\n}\n'
  const r1 = gateAudit16(terminated)
  assert.equal(r1.status, 1, r1.stdout)
  assert.match(r1.stdout, /S-016 违例：停机原因出现已退役的形状（1 处）/)
  assert.match(r1.stdout, /settle\.rs:2/)

  const why = 'fn f() {\n    payload["why"] = "上游 402".to_string();\n}\n'
  const r2 = gateAudit16(why)
  assert.equal(r2.status, 1, r2.stdout)
  assert.match(r2.stdout, /settle\.rs:2/)
})

test('S-016 放行新形状与合法组合', () => {
  // 新形状：顶层 `reason` + `detail`。
  const fresh = 'fn f() {\n    payload["reason"] = "fallback".to_string();\n    payload["detail"] = "上游 402".to_string();\n}\n'
  assert.equal(gateAudit16(fresh).status, 0, gateAudit16(fresh).stdout)
  // 合法组合：`with_terminated(with_reason(..))` 里 `terminated` 与 `reason`
  // 出现在同一行，但那是两个不同的键——误报会教人学会忽略这条规则。
  const composed = 'fn f() {\n    .with_payload(with_terminated(with_reason(p, ClosureReason::Interrupted)))\n}\n'
  assert.equal(gateAudit16(composed).status, 0, gateAudit16(composed).stdout)
  // 时刻仍在：`terminated.at` 是保留的形状。
  const at = 'fn f() {\n    payload["terminated"] = json!({ "at": clock_now_ms() });\n}\n'
  assert.equal(gateAudit16(at).status, 0, gateAudit16(at).stdout)
})

test('S-016 只管停机格：别的格子的 `why` 不误伤', () => {
  // `memory_events` 的 `"why": "consolidated"` 没有竞争的 `reason` 字段，
  // 不在本规则范围内——范围写大了的规则比没有规则更糟。
  const other = 'fn f() {\n    payload["why"] = "consolidated".to_string();\n}\n'
  assert.equal(
    gateAudit16(other, S016_OTHER_CELL).status,
    0,
    '非构造点文件不判',
  )
})

test('S-016 注释行不判，豁免必须写理由', () => {
  const asComment = '/// 旧形状（反例）：terminated: { reason, at }\n'
  assert.equal(gateAudit16(asComment).status, 0, '注释里的反例不是写入')
  const hit = '    payload["why"] = "x".to_string();\n'
  const waived = `${hit.replace('\n', '  // grep-audit-allow S-016: 兼容旧 WAL 回放，只读不写\n')}`
  assert.equal(gateAudit16(waived).status, 0, gateAudit16(waived).stdout)
  const empty = hit.replace('\n', '  // grep-audit-allow S-016:   \n')
  assert.equal(gateAudit16(empty).status, 1, '空理由不算豁免')
})

test('S-016 真实仓库零处（两个形状已全部迁完）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: SCAN_TIMEOUT_MS,
  })
  assert.match(r.stdout, /S-016 通过/)
})

// ── S-017: 恢复路径不许再有旁路 ───────────────────────────────────────
// 判据是**零出现**：恢复 / 审批 / 问答三条唤醒路径曾经走 `resume.rs` 的私有旁路
// （自己写协议事实、自己跑执行体，绕开行表）。旁路退役后唯一入口是**事实**。
// 放任它回来的代价不是风格问题——恢复路径会绕过行表写事实，于是「这一轮欠了
// 什么」与「这一轮做了什么」对不上账，而没有任何东西会变红。
const S017_LOOP = 'crates/symbio-plugin-loop/src'

/** 多文件临时树（这条规则要同时看「文件在不在」与「文件里写了什么」）。 */
function gateAudit17(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-s017-'))
  try {
    for (const [rel, body] of Object.entries(files)) {
      const target = path.join(root, rel)
      fs.mkdirSync(path.dirname(target), { recursive: true })
      fs.writeFileSync(target, body)
    }
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
    return spawnSync(process.execPath, [script], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 10000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('S-017 `resume_anchor` 与 `resumed` 分支回来即红', () => {
  const anchor = gateAudit17({
    [`${S017_LOOP}/loop_rows.rs`]: 'fn pick() -> u64 {\n    resume_anchor(&facts, 0)\n}\n',
  })
  assert.equal(anchor.status, 1, anchor.stdout)
  assert.match(anchor.stdout, /S-017 违例：恢复路径的私有旁路回来了/)
  assert.match(anchor.stdout, /loop_rows\.rs:2/)

  const resumed = gateAudit17({
    [`${S017_LOOP}/turn_exec.rs`]: 'fn f() {\n    if resumed {\n        return;\n    }\n}\n',
  })
  assert.equal(resumed.status, 1, resumed.stdout)
  assert.match(resumed.stdout, /turn_exec\.rs:2/)
})

test('S-017 `resume.rs` 回来即红（文件存在本身就是违例）', () => {
  const r = gateAudit17({
    [`${S017_LOOP}/resume.rs`]: 'pub fn resume() {}\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /S-017 违例：恢复路径的私有旁路回来了/)
  assert.match(r.stdout, /resume\.rs/)
})

test('S-017 注释行不判；视图字段名 `turn_head.resumed` 不在本规则范围内', () => {
  const asComment = gateAudit17({
    [`${S017_LOOP}/turn_exec.rs`]: '/// 旧形状（反例）：if resumed { … }\n',
  })
  assert.equal(asComment.status, 0, asComment.stdout)
  // `loop_rows.rs` 里的 `resumed` 是**视图字段名**（`turn_head.resumed`），
  // 不是旁路分支——范围写大了的规则比没有规则更糟。
  const field = gateAudit17({
    [`${S017_LOOP}/loop_rows.rs`]: 'fn cond() {\n    ("resumed", Lit::Bool(false))\n}\n',
  })
  assert.equal(field.status, 0, field.stdout)
})

test('S-017 真实仓库零处（旁路已全部退役）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: SCAN_TIMEOUT_MS,
  })
  assert.match(r.stdout, /S-017 通过/)
})

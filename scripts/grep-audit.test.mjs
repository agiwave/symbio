import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./grep-audit.mjs', import.meta.url))
function audit(source, { waiver = '', strict = false, scope = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-'))
  try {
    const plugins = path.join(root, 'symbio/src/plugins')
    fs.mkdirSync(path.join(plugins, 'other'), { recursive: true })
    fs.writeFileSync(path.join(plugins, 'other/sample.rs'), source.replace('WAIVER', waiver))
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
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
    fs.mkdirSync(path.join(root, 'symbio', 'src', 'plugins'), { recursive: true })
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
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
    ['docs/guides/x.md', S010_SUSPECT_MD],
    ['symbio/src/plugins/session/memory.rs', 'pub const A: &str = ".vdfs/session";\n'],
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
    'symbio/src/plugins/vdfs/fs.rs': 'pub const VDFS_ADDR_ROOT: &str = ".vdfsv2";\n',
    'symbio/src/plugins/vdfs/README.md': '根目录 `.vdfs/session` 说明。\n',
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /S-010 通过/)
})

test('S-010 exempts historical records (archive)', () => {
  // `docs/CHANGELOG.md` 曾与 archive 并列豁免；该机制已废除（变更历史以 `git log` 为准），
  // 文件也随之删除，故这里只保留 archive 这一条豁免。
  const r = s010Audit({
    'symbio/src/plugins/vdfs/fs.rs': '', // 标记：这是一个仓库树
    'docs/archive/old-design.md': S010_SUSPECT_MD,
  })
  assert.equal(r.status, 0)
})

test('S-010 is not fooled by lookalike tokens (browser route / import paths)', () => {
  const r = s010Audit({
    'symbio/src/plugins/vdfs/fs.rs': '', // 标记：这是一个仓库树
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
    'symbio/src/plugins/vdfs/fs.rs': '', // 标记：这是一个仓库树
    '.symbio/session/09d74431/MEMORY.md': '会话落在 `.vdfs/session/<id>` 之下。\n',
  })
  assert.equal(r.status, 0, '运行期数据目录不应参与源码审计')
})

test('S-010 still fires inside the repo tree when runtime dirs are present', () => {
  // 反例守卫：跳过 `.symbio` 不能顺手把整棵树放过去。
  const r = s010Audit({
    'symbio/src/plugins/vdfs/fs.rs': '', // 标记：这是一个仓库树
    '.symbio/session/09d74431/MEMORY.md': '会话落在 `.vdfs/session/<id>` 之下。\n',
    'docs/guides/x.md': S010_SUSPECT_MD,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-010/)
})

test('S-010 waiver requires a reason (same line or the line above)', () => {
  // 仓库树标记：symbio/src/plugins（同时是其它规则的扫描范围）
  const mark = { 'symbio/src/plugins/vdfs/fs.rs': '' }
  const r1 = s010Audit({ ...mark, 'docs/x.md': `<!-- grep-audit-allow S-010: 历史快照示例 -->\n.vdfs/session\n` })
  assert.equal(r1.status, 0)
  const r2 = s010Audit({ ...mark, 'docs/x.md': `.vdfs/session <!-- grep-audit-allow S-010: 引用旧文 -->\n` })
  assert.equal(r2.status, 0)
  const r3 = s010Audit({ ...mark, 'docs/x.md': `.vdfs/session <!-- grep-audit-allow S-010:   -->\n` })
  assert.equal(r3.status, 1)
})

// ── S-011：补充抽干点必须夹在 gate_turn 与 prepare_turn_inputs 之间 ──────
// 夹具与 S-010 共用（`s010Audit` 建的就是「cwd 下有 symbio/ 的仓库树」），
// 落点由文件相对路径决定——S-011 只认 `symbio/src/plugins/session/chat_loop.rs`。
const s011Audit = s010Audit
const S011_FILE = 'symbio/src/plugins/session/chat_loop.rs'

/** 位置正确的形态：gate_turn 调用 → 抽干 → prepare_turn_inputs 调用 */
const S011_OK = `fn run() {
    match gate_turn(&turn_req, &turn) {}
    if let Some(d) = orchestrator.supplements.as_ref() {
        d.drain().await;
    }
    let inputs = prepare_turn_inputs(&ctx);
}
`

test('S-011 passes when the drain sits between gate_turn and prepare_turn_inputs', () => {
  const r = s011Audit({ [S011_FILE]: S011_OK })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /S-011 通过：抽干点夹在 gate_turn\(2\) 与 prepare_turn_inputs\(6\) 之间/)
})

test('S-011 fires when the drain moves before gate_turn', () => {
  // 前移的真实后果：闸门 Exit / WaitForTools 时队列已被消费却没人用——
  // 用户那句补充进了转写，模型从没见过它，**没有任何测试会变红**。
  const r = s011Audit({
    [S011_FILE]: `fn run() {
    orchestrator.supplements.as_ref();
    match gate_turn(&turn_req, &turn) {}
    prepare_turn_inputs(&ctx);
}
`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-011 抽干点位置错/)
})

test('S-011 fires when the drain moves after prepare_turn_inputs', () => {
  // 后移的真实后果：合并消息不在本轮请求里，白折一轮（请求视图已经建好了）。
  const r = s011Audit({
    [S011_FILE]: `fn run() {
    match gate_turn(&turn_req, &turn) {}
    prepare_turn_inputs(&ctx);
    orchestrator.supplements.as_ref();
}
`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-011 抽干点位置错/)
})

test('S-011 errors when an anchor disappears (guard must not silently no-op)', () => {
  // 锚点缺失 = 抽干点被删或被改名。守卫此时**必须响亮**：让它悄悄退化成空转，
  // 它会以绿灯的形式提供虚假保证——比没有守卫更糟。故"锚点缺失"不接受豁免。
  const renamed = s011Audit({
    [S011_FILE]: S011_OK.replace('orchestrator.supplements', 'orchestrator.supplement_drain'),
  })
  assert.equal(renamed.status, 1)
  assert.match(renamed.stdout, /S-011 锚点缺失/)

  const gateGone = s011Audit({
    [S011_FILE]: `fn run() {
    orchestrator.supplements.as_ref();
    prepare_turn_inputs(&ctx);
}
`,
  })
  assert.equal(gateGone.status, 1)
  assert.match(gateGone.stdout, /gate_turn/)

  // 豁免通道对"锚点缺失"无效——无从判断的事不能靠签名放行
  const waivedButMissing = s011Audit({
    [S011_FILE]: `fn run() {
    orchestrator.supplement_drain.as_ref(); // grep-audit-allow S-011: 已确认
    prepare_turn_inputs(&ctx);
}
`,
  })
  assert.equal(waivedButMissing.status, 1)
})

test('S-011 counts only code lines (a commented-out drain is "missing", not "fine")', () => {
  const r = s011Audit({
    [S011_FILE]: `fn run() {
    match gate_turn(&turn_req, &turn) {}
    // orchestrator.supplements.as_ref();  ← 注释掉抽干调用
    prepare_turn_inputs(&ctx);
}
`,
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /S-011 锚点缺失/)
})

test('S-011 ignores the `fn gate_turn` definition (only the call site anchors)', () => {
  // 真实文件里 `fn gate_turn(...)` 定义在**最后**，但规则不能依赖这个巧合：
  // 定义在调用点之后时，若把定义当成锚点就会误判成"位置错"。
  const r = s011Audit({
    [S011_FILE]: `fn run() {
    match gate_turn(&req, &turn) {}
    orchestrator.supplements.as_ref();
    prepare_turn_inputs(&ctx);
}
fn gate_turn(a: &A, b: &B) -> Gate { Gate::Proceed }
`,
  })
  assert.equal(r.status, 0)
})

test('S-011 waiver requires a reason', () => {
  const bad = `fn run() {
    orchestrator.supplements.as_ref(); WAIVER
    match gate_turn(&turn_req, &turn) {}
    prepare_turn_inputs(&ctx);
}
`
  const withReason = s011Audit({
    [S011_FILE]: bad.replace('WAIVER', '// grep-audit-allow S-011: 抽干点由外层调度器统一处理'),
  })
  assert.equal(withReason.status, 0)
  const emptyReason = s011Audit({ [S011_FILE]: bad.replace('WAIVER', '// grep-audit-allow S-011:   ') })
  assert.equal(emptyReason.status, 1)
})

test('S-011 is skipped (not failed) when chat_loop.rs is outside the scope', () => {
  const r = audit('fn unrelated() {}\n')
  assert.equal(r.status, 0)
  assert.match(r.stdout, /S-011 跳过/)
})

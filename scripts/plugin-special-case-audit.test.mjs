/**
 * plugin-special-case-audit 回归测试 —— 证明它**会红**，且**只在该红的地方红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。但「写宽了」同样是坏守卫：误报一片 ⇒ 人把豁免写到
 * 满屏 ⇒ 判据被喂到失效。故每个码都钉两头：注入违规必须红，真实形态的**近邻**不得误报。
 *
 *   ① 干净夹具 → exit 0；
 *   ② SC-001 `if name == "agent_run"` → 红；
 *   ③ SC-001 不误报：`unwrap_or("ask_user")`（缺省值）与 `.get("heartbeat")`（键名）→
 *      不是 SC-001，但它们是 SC-006 的命中（架构面出现工具名）——两条判据各判一层；
 *   ④ SC-001 不误报：注释里的比较、`.test.rs` 里的比较 → 绿；
 *   ⑤ SC-002 预算键直接跟数字 → 红；读档案表 → 绿；
 *   ⑥ SC-003 绕开档案表直接读思考字段 → 红；走 `first_str` + 表键 → 绿；
 *   ⑦ SC-004 供应商名进 `contains` 判据 → 红；`match` 臂（别名登记表）与
 *      `"anthropic-version"` 头名 → 绿；
 *   ⑧ SC-005 豁免悬空（登记了而那一行没命中）→ 红；
 *   ⑨ 带理由的豁免 → 绿；**空理由**不算豁免 → 红；
 *   ⑩ 工具名集合读不出来 ⇒ 判据失效，exit 1（不许「读空即全绿」）；
 *   ⑪ SC-006：工具名落在 core / kernel / loop / session 的非测试代码 → 红（不限于比较位置）；
 *   ⑫ SC-006 不误报：前缀 / 后缀变体、`.test.rs` / `tests.rs` / 内联测试模块 → 绿；
 *   ⑬ SC-006 基线：在册且处数相符 → 绿；处数下降而基线不改 → 红（只许降）；
 *      基线缺决策 / 引用不存在的决策 → 红；基线读不出来 → 判据失效；
 *   ⑭ SC-006 豁免：必须挂一条**能打开的** `### D-NNN`；悬空 → SC-005。
 *
 * 用法：node --test scripts/plugin-special-case-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./plugin-special-case-audit.mjs', import.meta.url))

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
 * 工具名的**最小可提取**夹具。
 *
 * 为什么不直接用本仓真实的工具名：那样这条判据会随一次无关改名一起红（`agent_run`
 * 若被改名，红的理由却是「守卫坏了」）。工具名从代码提取这件事本身，由 `facts/_capability`
 * 的测试守；这里只要求「提取接口给得出一个名字」。
 */
const TOOLS_FIXTURE = {
  'crates/symbio-plugin-fake/src/tools.rs': [
    'pub fn table() -> Vec<ToolSpec> {',
    '    vec![',
    '        tool("agent_run", "跑子智能体", schema(), CapabilityRiskLevel::High),',
    '        tool("ask_user", "问用户", schema(), CapabilityRiskLevel::Low),',
    '        tool("heartbeat", "定时心跳", schema(), CapabilityRiskLevel::Low),',
    '    ]',
    '}',
    '',
  ].join('\n'),
}

/** 决策夹具：SC-006 的基线与豁免都要挂在它身上。 */
const DOCS_FIXTURE = { 'design.md': '### D-076 心跳片的八个宿主触点\n\n正文。\n' }

/**
 * 跑一次守卫：五个根 + 基线 + 决策目录都是夹具；`REL_ROOT` 指夹具根，基线键才写成
 * `session/…` 这种稳定形状。`baseline: null` ⇒ 不写基线文件（判「基线缺失」）。
 */
function run({
  session = {},
  protocols = {},
  tools = TOOLS_FIXTURE,
  core = {},
  kernel = {},
  loop = {},
  docs = DOCS_FIXTURE,
  baseline = {},
} = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'plugin-special-case-'))
  try {
    const write = (base, files) => {
      for (const [rel, content] of Object.entries(files)) {
        const abs = path.join(root, base, rel)
        fs.mkdirSync(path.dirname(abs), { recursive: true })
        fs.writeFileSync(abs, content)
      }
    }
    write('session', session)
    write('protocols', protocols)
    write('tools', tools)
    write('core', core)
    write('kernel', kernel)
    write('loop', loop)
    write('docs', docs)
    const baselineFile = path.join(root, 'tool-names.json')
    if (baseline !== null) fs.writeFileSync(baselineFile, JSON.stringify(baseline))
    for (const dir of ['session', 'protocols', 'tools', 'core', 'kernel', 'loop', 'docs']) {
      fs.mkdirSync(path.join(root, dir), { recursive: true })
    }
    const result = spawnSync(
      process.execPath,
      [
        script,
        `REL_ROOT=${root}`,
        `SESSION_ROOT=${path.join(root, 'session')}`,
        `PROTOCOLS_ROOT=${path.join(root, 'protocols')}`,
        `TOOLS_ROOT=${path.join(root, 'tools')}`,
        `CORE_ROOT=${path.join(root, 'core')}`,
        `KERNEL_ROOT=${path.join(root, 'kernel')}`,
        `LOOP_ROOT=${path.join(root, 'loop')}`,
        `DOCS_ROOT=${path.join(root, 'docs')}`,
        `SC006_BASELINE=${baselineFile}`,
      ],
      { cwd: root, env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 30_000 },
    )
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

/** 档案表夹具：SC-003 的名单来源就是它里面的字符串字面量。 */
const PROFILE_FIXTURE = {
  'profile.rs': [
    'pub(crate) const OPENAI_CHAT_REASONING_KEYS: &[&str] = &["reasoning_content", "reasoning"];',
    'pub(crate) fn first_str(v: &Value, keys: &[&str]) -> Option<&str> {',
    '    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str))',
    '}',
    '',
  ].join('\n'),
}

/** 一处 SC-006 存量：基线 + 决策夹具都齐。 */
const SC006_BASELINE = {
  'session/autonomous.rs': {
    heartbeat: { count: 1, adr: 'D-076', reason: '读心跳插件的 metadata 命名空间，不是按工具名分流' },
  },
}

test('干净夹具：无违规时退出 0', () => {
  const r = run({
    session: { 'executor.rs': 'fn f(name: &str) -> bool { name.starts_with("local/") }\n' },
    protocols: { 'profile.rs': PROFILE_FIXTURE['profile.rs'], 'chat.rs': 'let x = 1;\n' },
  })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /✓ 插件层特判审计/)
})

test('SC-001：工具名进比较位置 ⇒ 红', () => {
  const r = run({ session: { 'executor.rs': 'if name == "agent_run" {\n    go();\n}\n' } })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-001/)
})

test('SC-001 不误报、SC-006 接住：缺省值与键名不是「按名分流」，但仍不许落进架构面', () => {
  // 这两处是真实仓库里的现成写法（resume.rs 的 `unwrap_or("ask_user")`、
  // autonomous.rs 的 `.get("heartbeat")`）。SC-001 只判比较位置，故不报；
  // SC-006 判「出现」，故必须由基线/豁免接住——两层判据各判一件事。
  const r = run({
    session: {
      'resume.rs':
        'let tool_name = prompt.get("tool_name").and_then(|v| v.as_str()).unwrap_or("ask_user");\n',
      'autonomous.rs': 'let h = meta.get("heartbeat").and_then(|v| v.get("include_history"));\n',
    },
  })
  assert.equal(r.status, 1)
  assert.doesNotMatch(r.stderr, /SC-001/)
  assert.match(r.stderr, /SC-006/)
})

test('SC-006 基线与决策都在册 ⇒ 绿（存量占额度）', () => {
  const r = run({
    session: { 'autonomous.rs': 'let h = meta.get("heartbeat").and_then(|v| v.get("include_history"));\n' },
    baseline: SC006_BASELINE,
  })
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /SC-006 存量 1 处/)
})

test('SC-006：核心树里出现工具名 ⇒ 红（不限于比较位置）', () => {
  const r = run({ core: { 'lib.rs': 'const DEFAULT_TOOL: &str = "agent_run";\n' } })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-006/)
})

test('SC-006 不误报：变体名与测试文件', () => {
  const r = run({
    core: { 'lib.rs': 'const A: &str = "agent_run_v2";\nconst B: &str = "my-agent_run";\n' },
    kernel: {
      'lib.rs': [
        '#[cfg(test)]',
        'mod tests {',
        '    #[test]',
        '    fn t() { assert_eq!("agent_run", "agent_run"); }',
        '}',
        '',
      ].join('\n'),
      'tests.rs': 'fn t() { let _ = "agent_run"; }\n',
      'shape.test.rs': 'fn t() { let _ = "agent_run"; }\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-006 基线只许降：代码里少了一处而基线没改 ⇒ 红', () => {
  const r = run({
    session: {
      'autonomous.rs': [
        'let h = meta.get("heartbeat");',
        '// special-case-allow SC-006: D-076 读心跳插件的 metadata 命名空间',
        '',
      ].join('\n'),
    },
    baseline: {
      'session/autonomous.rs': {
        heartbeat: { count: 2, adr: 'D-076', reason: '读心跳插件的 metadata 命名空间' },
      },
    },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /基线已过期/)
})

test('SC-006 基线资格：没有决策 / 决策不在册 ⇒ 红', () => {
  const missing = run({
    session: { 'autonomous.rs': 'let h = meta.get("heartbeat");\n' },
    baseline: { 'session/autonomous.rs': { heartbeat: { count: 1, reason: '没有挂决策' } } },
  })
  assert.equal(missing.status, 1)
  assert.match(missing.stderr, /没有挂决策/)

  const bogus = run({
    session: { 'autonomous.rs': 'let h = meta.get("heartbeat");\n' },
    baseline: {
      'session/autonomous.rs': {
        heartbeat: { count: 1, adr: 'D-999', reason: '指向不存在的决策' },
      },
    },
  })
  assert.equal(bogus.status, 1)
  assert.match(bogus.stderr, /没有 `### D-999` 条目/)
})

test('SC-006 豁免：带能打开的决策 ⇒ 绿；缺决策 / 决策不在册 / 悬空 ⇒ 红', () => {
  const waived = run({
    session: {
      'autonomous.rs': [
        'let h = meta.get("heartbeat");',
        '// special-case-allow SC-006: D-076 读心跳插件的 metadata 命名空间，不是按工具名分流',
        '',
      ].join('\n'),
    },
  })
  assert.equal(waived.status, 0, waived.stderr)
  assert.match(waived.stdout, /代码内豁免 1 处/)

  const noAdr = run({
    session: {
      'autonomous.rs': [
        'let h = meta.get("heartbeat");',
        '// special-case-allow SC-006: 这处应该留下',
        '',
      ].join('\n'),
    },
  })
  assert.equal(noAdr.status, 1)
  assert.match(noAdr.stderr, /没有挂决策/)

  const badAdr = run({
    session: {
      'autonomous.rs': [
        'let h = meta.get("heartbeat");',
        '// special-case-allow SC-006: D-999 指向不存在的决策',
        '',
      ].join('\n'),
    },
  })
  assert.equal(badAdr.status, 1)
  assert.match(badAdr.stderr, /没有 `### D-999` 条目/)

  const dangling = run({
    core: { 'lib.rs': 'fn f() {}\n// special-case-allow SC-006: D-076 这里早就没有工具名了\n' },
  })
  assert.equal(dangling.status, 1)
  assert.match(dangling.stderr, /SC-005/)
})

test('SC-006 基线读不出来 ⇒ 判据失效，不许「读空即全绿」', () => {
  const r = run({ session: { 'executor.rs': 'fn f() {}\n' }, baseline: null })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /失去判据/)
})

test('SC-001 不误报：注释与 .test.rs 里的比较不算', () => {
  const r = run({
    session: {
      'executor.rs': '// 判据同批首读一次：这里曾经有 `if name == "agent_run"` 的特判\nfn f() {}\n',
      'executor.test.rs': 'if name == "agent_run" {\n    assert!(true);\n}\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-002：预算键直接跟数字 ⇒ 红', () => {
  const r = run({
    protocols: { 'chat.rs': 'req["max_tokens"] = json!(4096);\n' },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-002/)
})

test('SC-002 不误报：读档案表 ⇒ 绿', () => {
  const r = run({
    protocols: {
      'chat.rs': 'req["max_tokens"] = json!(cfg.max_tokens.unwrap_or(profile::OPENAI_CHAT.default));\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-003：绕开档案表直接读思考字段 ⇒ 红', () => {
  const r = run({
    protocols: {
      'profile.rs': PROFILE_FIXTURE['profile.rs'],
      'chat.rs': 'if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {}\n',
    },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-003/)
})

test('SC-003 不误报：走 first_str + 表键 ⇒ 绿', () => {
  const r = run({
    protocols: {
      'profile.rs': PROFILE_FIXTURE['profile.rs'],
      'chat.rs': 'let r = first_str(delta, OPENAI_CHAT_REASONING_KEYS);\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-004：供应商名进 if 式判据 ⇒ 红', () => {
  const r = run({ protocols: { 'chat.rs': 'if cfg.model.contains("glm") {\n    clamp();\n}\n' } })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-004/)
})

test('SC-004 不误报：match 臂（别名登记表）与头名不算分支', () => {
  const r = run({
    protocols: {
      'mod.rs': 'let id = match alias {\n    "anthropic_messages" | "anthropic" => MODEL_PROTOCOL_ANTHROPIC_MESSAGES,\n    _ => MODEL_PROTOCOL_OPENAI_CHAT,\n};\n',
      'messages.rs': 'h.insert("anthropic-version", v);\n',
    },
  })
  assert.equal(r.status, 0, r.stderr)
})

test('SC-005：豁免悬空 ⇒ 红（登记了而那一行没命中）', () => {
  const r = run({
    session: { 'executor.rs': 'fn f() {}\n// special-case-allow SC-001: 这一行早就没有比较了\n' },
  })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /SC-005/)
})

test('带理由的豁免放行；空理由不算豁免', () => {
  // 同一行同时是 SC-001（比较位置）与 SC-006（架构面出现）的命中：两条判据各自要一条豁免。
  const waived = run({
    session: {
      'executor.rs': [
        'if name == "agent_run" { // special-case-allow SC-001: 这里的名字来自外部清单',
        '// special-case-allow SC-006: D-076 这处名字来自外部清单，不是注册表里的工具',
        '    go();',
        '}',
        '',
      ].join('\n'),
    },
  })
  assert.equal(waived.status, 0, waived.stderr)
  assert.match(waived.stdout, /代码内豁免 2 处/)

  const empty = run({
    session: {
      'executor.rs': [
        'if name == "agent_run" {',
        '    // special-case-allow SC-001:',
        '    go();',
        '}',
        '',
      ].join('\n'),
    },
  })
  assert.equal(empty.status, 1, '空理由视为未承认')
  assert.match(empty.stderr, /SC-001/)
})

test('工具名集合读不出来 ⇒ 判据失效，不许「读空即全绿」', () => {
  const r = run({ session: { 'executor.rs': 'fn f() {}\n' }, tools: {} })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /失去判据/)
})

/**
 * prompt-shape-audit 回归测试 —— 证明它**会红**，且**不误报**
 *
 * 这个守卫的失效形态有两头，与 `core-shape-audit` 同族：
 * ① **提取写宽**：结构载体（`tool_calls` / `tool_call_id`）没进形状 ⇒ ADR-048a 判过的
 *    「拍平」这件事红不了，基线退化成一份没人信的文档；
 * ② **提取写严**：把负载决定的字节数当成判据 ⇒ 改一句用例措辞就红，
 *    红到没人再看——本仓为这种结局写过一整节（`core-export-audit` 的 W1 注）。
 * 所以**每个判据都配一条反向用例**：该红的红、不该红的不得红。
 *
 * 覆盖：
 *   1. `sectionsOf` 的「行首才算标签」口径（正文里引用别的【标签】不得切出假小节）；
 *   2. PS-001 各类形状漂移各自变红：拍平 / 请求数 / 小节标签顺序 / 工具面 / 信封；
 *   3. 反向：正文措辞与规模变化**不得**红（规模不是判据）；
 *   4. PS-002 基线缺失 / JSON 块读不出 ⇒ 退出 2（不是 1，也不得静默通过）；
 *   5. PS-003 现场请求数为 0 ⇒ 退出 2 ——「空形状与空基线一致」是本仓最坏的假绿灯；
 *   6. 渲染 ↔ 解析往返：基线文件里的 JSON 块与提取出的形状逐字段相等；
 *   7. 一致 ⇒ 退出 0（三条判据都在结论行里出现，缺一说明有一格没在工作）；
 *   8. 平台变体（`cmd` / `sh`）归一成记号 `<shell>`，且名单**逐字钉** `shell.rs` 的
 *      `tool_name`（第二份名单不许漂移）；真名叫 `shell` 的工具**不得**被误归一；
 *   9. schema 版本不一致 ⇒ `diffShapes` 直接判漂移（旧基线得到「重生成」而不是一串字段差）。
 *
 * 用法：node --test scripts/prompt-shape-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import {
  extractShape,
  extractObservations,
  diffShapes,
  sectionsOf,
  renderBaselineDoc,
  parseBaselineDoc,
  PLATFORM_VARIANT_TOOL,
  PLATFORM_VARIANT_TOOL_NAMES,
  SCHEMA_VERSION,
} from './prompt-shape-audit.mjs'
import { stripAnsi } from './color.mjs'

const script = fileURLToPath(new URL('./prompt-shape-audit.mjs', import.meta.url))
const repoRoot = fileURLToPath(new URL('..', import.meta.url))

function tmpDir (tag) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `symbio-ps-${tag}-`))
}

/** 出厂 full 路径的**最小真实形状**：system 前缀（无标签段 + 两个小节）+ user + 工具面。 */
function baseRequests () {
  return [
    {
      body: {
        model: 'mock-model',
        stream: true,
        temperature: 0.1,
        tool_choice: 'auto',
        max_tokens: 1024,
        messages: [
          { role: 'system', content: '【工作区记忆】（与【智能体记忆】相互独立）\n内容\n\n【会话记忆】\n（暂无内容）\n' },
          { role: 'system', content: '- 委派判定：未启动 worker\n【能力目录】\n- Chat: agent_run\n' },
          { role: 'user', content: '帮我回显' },
        ],
        tools: [{ function: { name: 'zzz_tool', description: 'd' } }, { function: { name: 'aaa_tool', description: 'e' } }],
      },
    },
    {
      body: {
        model: 'mock-model',
        stream: true,
        temperature: 0.1,
        tool_choice: 'auto',
        max_tokens: 1024,
        messages: [
          { role: 'system', content: '【工作区记忆】x\n【会话记忆】y\n' },
          { role: 'system', content: '- 委派判定\n【能力目录】z\n' },
          { role: 'user', content: '帮我回显' },
          { role: 'assistant', content: '我先回显一下。', tool_calls: [{ id: 'call_1', function: { name: 'mcp__mockserv__echo', arguments: '{}' } }] },
          { role: 'tool', tool_call_id: 'call_1', content: '{"echo":1}' },
          { role: 'assistant', content: '回显成功。' },
        ],
        tools: [{ function: { name: 'zzz_tool' } }, { function: { name: 'aaa_tool' } }],
      },
    },
  ]
}

// ================= 提取口径 =================

test('sectionsOf：只有行首的【…】算小节标签（正文里引用别的标签不得切出假小节）', () => {
  const secs = sectionsOf('【工作区记忆】（全工作区共享，与【智能体记忆】相互独立；改写前先读）\n（暂无内容）\n\n【会话记忆】\n（本会话私有）\n')
  assert.deepEqual(secs.map((s) => s.tag), ['工作区记忆', '会话记忆'])
})

test('sectionsOf：首个标签之前的残留是一节无标签正文（出厂前缀里的委派判定那段）', () => {
  const secs = sectionsOf('- 委派判定：未启动 worker\n【能力目录】\n- Chat: agent_run\n')
  assert.deepEqual(secs.map((s) => s.tag), [null, '能力目录'])
})

test('sectionsOf：整段没有标签 ⇒ 一节 null；空正文 ⇒ 零节', () => {
  assert.deepEqual(sectionsOf('一段没有任何标签的正文').map((s) => s.tag), [null])
  assert.deepEqual(sectionsOf(''), [])
})

test('extractShape：结构载体进形状（assistant 的 tool_calls 条数、tool 结果的 tool_call_id）', () => {
  const shape = extractShape(baseRequests(), { scenario: 'unit' })
  const second = shape.requests[1].message_shape
  assert.deepEqual(
    second.map((m) => `${m.role}:${m.tool_calls}:${m.tool_call_id}`),
    ['system:0:false', 'system:0:false', 'user:0:false', 'assistant:1:false', 'tool:0:true', 'assistant:0:false'],
  )
})

test('extractShape：工具名按**送出的原始顺序**入形状（顺序就是装配的产物，换序必须红）', () => {
  const a = extractShape(baseRequests())
  // 两轮的同一条判据因此能读：注册序稳定 ⇒ 每一轮的 `tools` 顺序一致（工具面是 prompt
  // 里最大的一段，两轮各换一次序等于每轮都把这段前缀作废）。
  assert.deepEqual(a.requests[0].tools.names, ['zzz_tool', 'aaa_tool'])
  assert.deepEqual(a.requests[1].tools.names, ['zzz_tool', 'aaa_tool'])

  const shuffled = baseRequests()
  shuffled[0].body.tools.reverse()
  const b = extractShape(shuffled)
  const diffs = diffShapes(a, b)
  assert.ok(diffs.length > 0, '同一套工具、顺序不同 ⇒ 必须红（判据不靠排序兜住顺序）')
})

// ================= PS-001：该红的红 =================

test('PS-001 拍平（ADR-048a 判过的那次退步）⇒ 红：system 小节合成一条 user 消息', () => {
  const live = baseRequests()
  // 拍平的形态：两条 system 消息并成一条 user 纯文本，工具结果也不再是 role: tool
  live[0].body.messages = [{ role: 'user', content: live[0].body.messages.map((m) => m.content).join('\n') }]
  const d = diffShapes(extractShape(baseRequests()), extractShape(live))
  assert.ok(d.some((x) => x.includes('消息序列')), d.join('\n'))
  assert.ok(d.some((x) => x.includes('system 小节标签')), d.join('\n'))
})

test('PS-001 请求数变化 ⇒ 红（多一轮 / 少一轮都算）', () => {
  const live = baseRequests().slice(0, 1)
  const d = diffShapes(extractShape(baseRequests()), extractShape(live))
  assert.ok(d.some((x) => /请求数：基线 2 ≠ 现场 1/.test(x)), d.join('\n'))
})

test('PS-001 小节标签序列漂移 ⇒ 红（新增一节 / 换顺序）', () => {
  const live = baseRequests()
  live[0].body.messages[1].content = '【新小节】\n【能力目录】\n'
  const d = diffShapes(extractShape(baseRequests()), extractShape(live))
  assert.ok(d.some((x) => x.includes('system 小节标签')), d.join('\n'))
})

test('PS-001 工具面漂移 ⇒ 红：新增 / 移除各自说清', () => {
  const live = baseRequests()
  live[0].body.tools.push({ function: { name: 'new_tool' } })
  live[0].body.tools = live[0].body.tools.filter((t) => t.function.name !== 'aaa_tool')
  const d = diffShapes(extractShape(baseRequests()), extractShape(live))
  assert.ok(d.some((x) => x.includes('新增 [new_tool]') && x.includes('移除 [aaa_tool]')), d.join('\n'))
})

test('PS-001 信封参数漂移 ⇒ 红（装配侧改 temperature 这类取值是有影响的改动）', () => {
  const live = baseRequests()
  live[0].body.temperature = 0.9
  const d = diffShapes(extractShape(baseRequests()), extractShape(live))
  assert.ok(d.some((x) => x.includes('req0 信封')), d.join('\n'))
})

test('PS-001 结构载体消失 ⇒ 红：工具结果被降级成正文（tool_call_id 不再带）', () => {
  const live = baseRequests()
  const msgs = live[1].body.messages
  const i = msgs.findIndex((m) => m.role === 'tool')
  msgs[i] = { role: 'user', content: `工具结果：${msgs[i].content}` }
  const d = diffShapes(extractShape(baseRequests()), extractShape(live))
  assert.ok(d.some((x) => x.includes('消息序列')), d.join('\n'))
})

// ================= PS-001 的反向：不该红的不得红 =================

test('反向：正文措辞与规模变化不得红（规模由负载决定，不是判据）', () => {
  const live = baseRequests()
  // 标签序列**保持不变**，只把字变长——这正是「改用例措辞」的形状：结构没动。
  live[0].body.messages[0].content =
    '【工作区记忆】（这一节的字比基线长了很多很多，因为用例文本被改了；与【智能体记忆】相互独立）\n内容更多更更长\n\n【会话记忆】\n（本会话私有，现在字也长了）\n'
  live[0].body.messages[2].content = '帮我把这段话改得更长一些再回显'
  assert.deepEqual(diffShapes(extractShape(baseRequests()), extractShape(live)), [])
})

test('反向：观测列如实带规模，但它不参与比对', () => {
  const obs = extractObservations(baseRequests())
  assert.equal(obs.length, 2)
  assert.ok(obs[0].message_bytes > 0 && obs[0].request_bytes > obs[0].message_bytes)
  // 同一形状、正文更长 ⇒ 观测值必须变大（观测确实反映规模），但判据不得红
  const longer = baseRequests()
  longer[0].body.messages[2].content = '帮我把这段话改得更长一些再回显'.repeat(8)
  const obsLonger = extractObservations(longer)
  assert.ok(obsLonger[0].message_bytes > obs[0].message_bytes, '观测列必须随正文规模变大')
  assert.deepEqual(diffShapes(extractShape(baseRequests()), extractShape(longer)), [])
})

// ================= 基线文件往返 =================

test('渲染 ↔ 解析往返：基线文件里的 JSON 块与提取形状逐字段相等', () => {
  const shape = extractShape(baseRequests(), { scenario: 'unit' })
  const doc = renderBaselineDoc(shape, extractObservations(baseRequests()), { capturedAt: '2026-10-12' })
  assert.deepEqual(parseBaselineDoc(doc), shape)
  assert.match(doc, /不手改/)
  assert.match(doc, /规模观测（\*\*不进判据/)
})

test('解析读不出 ⇒ null（缺块 / JSON 坏 / requests 不是数组），由调用方判 PS-002', () => {
  assert.equal(parseBaselineDoc('# 没有围栏的文档\n'), null)
  assert.equal(parseBaselineDoc('```json:prompt-shape\n{坏掉的\n```'), null)
  assert.equal(parseBaselineDoc('```json:prompt-shape\n{"schema":1}\n```'), null)
})

// ================= CLI：退出码三态 =================

function runCli (args) {
  const r = spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' })
  return { code: r.status, out: stripAnsi(r.stdout ?? '') + stripAnsi(r.stderr ?? '') }
}

function writeLive (dir, requests) {
  const p = path.join(dir, 'live.json')
  fs.writeFileSync(p, JSON.stringify(requests))
  return p
}

function writeBaseline (dir, requests) {
  const shape = extractShape(requests, { scenario: 'unit' })
  const p = path.join(dir, 'PROMPT_SHAPE.md')
  fs.writeFileSync(p, renderBaselineDoc(shape, extractObservations(requests), { capturedAt: '2026-10-12' }))
  return p
}

test('一致 ⇒ 退出 0，且 PS-001 / PS-002 / PS-003 三条结论都打出来', () => {
  const dir = tmpDir('ok')
  const reqs = baseRequests()
  const r = runCli([`--from=${writeLive(dir, reqs)}`, `--baseline=${writeBaseline(dir, reqs)}`, '--scenario=unit'])
  assert.equal(r.code, 0, r.out)
  for (const code of ['PS-001', 'PS-002', 'PS-003']) assert.match(r.out, new RegExp(code), `${code} 没出现在结论里 ⇒ 那一格没在工作`)
  fs.rmSync(dir, { recursive: true, force: true })
})

test('PS-001 漂移 ⇒ 退出 1 并列出差异', () => {
  const dir = tmpDir('drift')
  const baseline = writeBaseline(dir, baseRequests())
  const mutated = baseRequests()
  mutated[0].body.messages.shift() // 少一条 system 小节
  const r = runCli([`--from=${writeLive(dir, mutated)}`, `--baseline=${baseline}`, '--scenario=unit'])
  assert.equal(r.code, 1, r.out)
  assert.match(r.out, /PS-001/)
  fs.rmSync(dir, { recursive: true, force: true })
})

test('PS-002 基线缺失 / 块读不出 ⇒ 退出 2（不是 1，也不得静默通过）', () => {
  const dir = tmpDir('nobase')
  const live = writeLive(dir, baseRequests())
  const missing = path.join(dir, 'gone.md')
  const r1 = runCli([`--from=${live}`, `--baseline=${missing}`, '--scenario=unit'])
  assert.equal(r1.code, 2, r1.out)
  assert.match(r1.out, /PS-002/)

  const broken = path.join(dir, 'broken.md')
  fs.writeFileSync(broken, '# 生成物\n')
  const r2 = runCli([`--from=${live}`, `--baseline=${broken}`, '--scenario=unit'])
  assert.equal(r2.code, 2, r2.out)
  assert.match(r2.out, /PS-002/)
  fs.rmSync(dir, { recursive: true, force: true })
})

test('PS-003 现场零请求 ⇒ 退出 2：空形状与空基线「一致」是本脚本最坏的假绿灯', () => {
  const dir = tmpDir('empty')
  const reqs = baseRequests()
  const baseline = writeBaseline(dir, reqs)
  const r = runCli([`--from=${writeLive(dir, [])}`, `--baseline=${baseline}`, '--scenario=unit'])
  assert.equal(r.code, 2, r.out)
  assert.match(r.out, /PS-003/)
  fs.rmSync(dir, { recursive: true, force: true })
})

test('--write ⇒ 用现场形状重写基线，随后再判即一致', () => {
  const dir = tmpDir('write')
  const reqs = baseRequests()
  const target = path.join(dir, 'PROMPT_SHAPE.md')
  const w = runCli([`--from=${writeLive(dir, reqs)}`, `--baseline=${target}`, `--scenario=unit`, '--write'])
  assert.equal(w.code, 0, w.out)
  assert.ok(fs.existsSync(target))
  const r = runCli([`--from=${writeLive(dir, reqs)}`, `--baseline=${target}`, '--scenario=unit'])
  assert.equal(r.code, 0, r.out)
  fs.rmSync(dir, { recursive: true, force: true })
})

// ── 平台变体归一（2026-10-13，T-093）──────────────────────────────────────
//
// 出厂工具面里 shell 工具的名字**由宿主平台决定**（`shell.rs` 的 `get_os_info()` 按
// `target_os` 返回 `cmd` / `sh`）。把它原样入判据的后果实测过：基线在 Windows 上重生成 ⇒
// CI（Linux）恒红，报的是「形状漂移」，真因是平台。归一化是朝 ADR-057 决策 2 的既有原则靠
// ——「进判据的只有**由装配代码决定**的结构」。

test('平台变体名单 == shell.rs 声明的 tool_name（第二份名单不许漂移）', () => {
  const src = fs.readFileSync(
    path.join(repoRoot, 'symbio/src/plugins/local/shell.rs'),
    'utf8',
  )
  const fn = src.match(/fn get_os_info\(\)[\s\S]*?\n\}/)
  assert.ok(fn, 'shell.rs 里找不到 get_os_info —— 结构变了，跟着改这条钉')
  // 每个 cfg 分支返回 4 元组 (os_name, shell_name, tool_name, examples)，第 3 个即 tool_name。
  const names = [...fn[0].matchAll(/\(\s*"[^"]*",\s*"[^"]*",\s*"([^"]+)"/g)].map((m) => m[1])
  assert.ok(names.length >= 2, `只解析出 ${names.length} 个 tool_name —— 正则没跟上元组形状`)
  assert.deepEqual(
    [...new Set(names)].sort(),
    [...PLATFORM_VARIANT_TOOL_NAMES].sort(),
    '归一化名单与 shell.rs 的 tool_name 集合不一致：要么漏了平台（该平台会红），要么多收了名字（会误归一）',
  )
})

test('extractShape：平台变体名归一成记号，其余名字与顺序原样保留', () => {
  const reqs = baseRequests()
  // 同一份工具面，只把 shell 工具的名字换成两个平台各自的写法。
  for (const n of PLATFORM_VARIANT_TOOL_NAMES) {
    reqs[0].body.tools = [
      { function: { name: 'agent_run' } },
      { function: { name: n } },
      { function: { name: 'shell' } }, // 反向：真有一个工具就叫 shell，它**不得**被归一
    ]
    const names = extractShape(reqs).requests[0].tools.names
    assert.deepEqual(
      names,
      ['agent_run', PLATFORM_VARIANT_TOOL, 'shell'],
      `平台名 ${n} 未按预期归一`,
    )
  }
})

test('diffShapes：schema 不一致 ⇒ 直接判漂移（不逐字段比，避免一串看不懂的新增/移除）', () => {
  const reqs = baseRequests()
  const live = extractShape(reqs, { scenario: 'unit' })
  const stale = JSON.parse(JSON.stringify(live))
  stale.schema = SCHEMA_VERSION - 1
  const diffs = diffShapes(stale, live)
  assert.equal(diffs.length, 1, `schema 不一致应只报一条，实际 ${diffs.length} 条：${diffs.join('；')}`)
  assert.match(diffs[0], /schema/, diffs[0])
  assert.match(diffs[0], /重生成/, diffs[0])
})

// S-end③ 的 runner：**门外**实验，真实模型上的「结构化 vs 拍平」对照。
//
// 为什么是**回放**而不是「跑两次端到端会话」
// --------------------------------------
// 两次端到端会话无法把差异归因给形状：模型第 1 步选错工具，第 2 步的历史就换了，
// 两组比的根本不是同一道题。这里的做法是把**同一份结构化输入**送两次 —— 臂 B 由
// `flattenRequest` 从臂 A 的输入派生，除工具面与工具交换的载体外逐字节相同。
// 于是「两臂读数之差」只有一个解释：形状。
//
// 出厂上下文从哪来
// --------------
// system 前缀与 `tools` 数组整体取自一次**真实链路捕获**（`capture-corpus.mjs`：
// CLI × mock-llm × mock-mcp，出厂 `full` 档，对话面关闭 —— 与
// [ADR-057](../../docs/registry/adr/ADR-057.md) 形状基线同一条链路同一个场景名）。
// 题目只替换尾部消息。缺 `--corpus=` 就现场跑一次捕获。
//
// 判据（全部机械可读回，口径归 `tasks.mjs`）
// ---------------------------------------
// - `tool_name_hit`：select 题是否选了真值那个工具（必填键取自该工具 Schema 的 `required`，不手写）
// - `args_json_ok` / `required_keys_hit`：参数是不是合法 JSON、必填键齐不齐
// - `hallucinated_tool`：调了一个工具面上不存在的名字
// - `spurious_call`：control 题（不该调用）却调了
// - `attributed_correctly`：attribution 题——结果↔调用的对应关系还读得回来（答对且没串到另一条）
// - 臂 B 独有一条 `protocol_followed`：文本协议那一行 JSON 是否被遵守（这是拍平自己新引入的失败面）
//
// **不进判据的**：时延、token 数、字节数——它们进报告的观测列。理由与 ADR-057 同源：
// 规模同时由夹具与负载决定，钉进结论会让读数解释不了自己。
//
// 用法
// ----
//     node e2e/ab/run.mjs --provider=LMStudio [--corpus=<file>] [--tasks=id1,id2]
//                         [--repeat=1] [--temperature=0] [--json=<out>] [--dry-run]
// `--dry-run` 只打印两臂的请求体形状与字节数，**一个请求都不发**（不花任何预算）。
// 报告默认写 `docs/reference/PROMPT_AB.md`（生成物，唯一写者是本脚本）。
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TASKS, structuredBodyFor } from './tasks.mjs';
import { flattenRequest, parseFlatCalls, structuredCalls } from './flatten.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..', '..');
const REPORT_PATH = join(REPO, 'docs', 'reference', 'PROMPT_AB.md');

function arg(name, dflt = undefined) {
  const hit = process.argv.slice(2).find((a) => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : dflt;
}
const flag = (name) => process.argv.slice(2).includes(`--${name}`);

// ---------- 输入：语料 + provider ----------
function loadCorpus() {
  const given = arg('corpus');
  if (given) return JSON.parse(readFileSync(resolve(given), 'utf8'));
  const dst = join(mkdtempSync(join(tmpdir(), 'symbio-ab-')), 'corpus.json');
  const r = spawnSync(process.execPath, [join(HERE, 'capture-corpus.mjs'), `--out=${dst}`], {
    encoding: 'utf8',
    cwd: REPO,
  });
  if (r.status !== 0) {
    throw new Error(`现场捕获出厂请求体失败：\n${r.stdout ?? ''}${r.stderr ?? ''}`);
  }
  return JSON.parse(readFileSync(dst, 'utf8'));
}

function loadProvider(id) {
  const path = join(REPO, '.symbio', 'model', id, 'provider.json');
  let cfg;
  try {
    cfg = JSON.parse(readFileSync(path, 'utf8'));
  } catch {
    throw new Error(`读不到 provider 配置：${path}\n（实验只认本机已配好的 provider；可用的名字见 .symbio/model/）`);
  }
  // ⚠️ 这里刻意**不把配置整体打印出去**：里面是活的 api_key。
  return {
    id,
    api_base: cfg.api_base,
    api_key: cfg.api_key,
    model: arg('model') ?? cfg.model,
    max_tokens: Number(cfg.max_tokens ?? 4096),
    timeout_secs: Number(cfg.timeout_secs ?? 300),
    // 配置里写了节流就照节流走：这是本仓对这个 provider 的既有承诺，实验不该绕过它。
    min_interval_ms: Number(cfg.rate_limit_ms ?? 0),
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let lastCallAt = 0;
async function throttle(provider) {
  const wait = provider.min_interval_ms - (Date.now() - lastCallAt);
  if (wait > 0) await sleep(wait);
  lastCallAt = Date.now();
}

async function callModel(provider, body) {
  await throttle(provider);
  const t0 = Date.now();
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), provider.timeout_secs * 1000);
  try {
    const res = await fetch(`${provider.api_base}/chat/completions`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${provider.api_key}` },
      body: JSON.stringify(body),
      signal: ctrl.signal,
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text.slice(0, 400)}`);
    const json = JSON.parse(text);
    const choice = json.choices?.[0] ?? {};
    return {
      message: choice.message ?? {},
      finish: choice.finish_reason ?? null,
      usage: json.usage ?? null,
      cost: json.usage?.cost ?? json.usage?.total_cost ?? null,
      ms: Date.now() - t0,
      request_bytes: JSON.stringify(body).length,
    };
  } finally {
    clearTimeout(timer);
  }
}

// ---------- 判据 ----------
/** 一臂的请求体：臂 B 永远是臂 A 输入的**派生**，不存在手抄两份题目的偏差源。 */
function buildArmBody(label, structuredBody) {
  return label === 'structured' ? structuredBody : flattenRequest(structuredBody);
}

function requiredKeysOf(toolByName, name) {
  const t = toolByName.get(name);
  return t?.function?.parameters?.required ?? t?.parameters?.required ?? null;
}

function grade(task, arm, calls, result, toolByName) {
  const g = {
    tool_name_hit: null,
    args_json_ok: null,
    required_keys_hit: null,
    hallucinated_tool: false,
    spurious_call: null,
    protocol_followed: null,
    attributed_correctly: null,
    answered: false,
  };
  const first = calls[0];
  const called = calls.length > 0;

  if (task.face === 'select') {
    g.tool_name_hit = called && first.name === task.tool;
    g.hallucinated_tool = called && !toolByName.has(first.name);
    if (called) {
      const args = first.arguments;
      const ok = args && typeof args === 'object' && !Array.isArray(args);
      g.args_json_ok = arm === 'structured' ? first.json_ok && ok : ok;
      const req = requiredKeysOf(toolByName, task.tool);
      if (req && req.length > 0 && ok) g.required_keys_hit = req.every((k) => k in args);
    }
  } else if (task.face === 'control') {
    g.spurious_call = called;
  } else if (task.face === 'attribution') {
    // 判据只读**回答正文**：思维链里模型会把上下文整段复述出来（含另一条结果的标记），
    // 把它算进「答对没有」会让判据变成「它想到过什么」——那不是用户读得到的东西。
    const answer = String(result.message?.content ?? '');
    g.answered = answer.includes(task.expected_marker);
    g.reasoning_len = String(result.message?.reasoning_content ?? '').length;
    if (called) g.spurious_call = true;
    g.attributed_correctly =
      !called && answer.includes(task.expected_marker) && !answer.includes(task.forbidden_marker);
    if (!g.attributed_correctly && !called && answer.includes(task.expected_marker)) g.misattributed = true;
  }
  if (arm === 'flat') g.protocol_followed = called ? calls.every((c) => typeof c.raw === 'string') : null;
  // **本轮没跑完，不是模型没做到**：`finish_reason=length` 时回答没有收束（实测撞到过
  // 一整段 `<unused32>` 复读——那是本地服务进入坏状态的一段尾巴）。把它计成「失败」会让
  // 环境噪声进判据分母，读数和现状无法区分；单独计一列「未收束」，判据只认收束过的题。
  if (result.finish === 'length') {
    g.truncated = true;
    g.inconclusive = true;
    g.tool_name_hit = null;
    g.args_json_ok = null;
    g.required_keys_hit = null;
    g.spurious_call = null;
    g.attributed_correctly = null;
    g.protocol_followed = null;
    g.hallucinated_tool = null;
  }
  return g;
}

// ---------- 一臂跑一道题 ----------
async function runOneArm({ label, task, provider, corpus, toolByName, temperature, repeat }) {
  const body = buildArmBody(label, structuredBodyFor(corpus, task));
  body.temperature = temperature;
  body.stream = false;
  body.model = provider.model;
  if (body.max_tokens == null || body.max_tokens < provider.max_tokens) body.max_tokens = provider.max_tokens;
  let result;
  try {
    result = await callModel(provider, body);
  } catch (e) {
    const row = { task: task.id, face: task.face, arm: label, repeat, error: String(e.message).slice(0, 200) };
    console.log(`  [${label}] ${task.id} → 失败：${String(e.message).slice(0, 120)}`);
    return row;
  }
  const calls = label === 'structured' ? structuredCalls(result.message) : parseFlatCalls(result.message?.content).calls;
  const g = grade(task, label, calls, result, toolByName);
  const asked =
    label === 'structured'
      ? calls.map((c) => `${c.name}(${c.arguments_raw ?? ''})`).join(' + ')
      : calls.map((c) => c.raw).join(' + ');
  console.log(
    `  [${label === 'structured' ? 'A' : 'B'} · ${repeat}] ${task.id} → ${calls.length ? calls.map((c) => c.name).join(',') : '（不调用）'} ` +
      `· hit=${g.tool_name_hit ?? '–'} spurious=${g.spurious_call ?? '–'} attr=${g.attributed_correctly ?? '–'} ` +
      `· ${result.ms}ms · ${result.usage?.prompt_tokens ?? '?'}tok${result.cost ? ` · $${result.cost}` : ''}${g.inconclusive ? ' · 未收束（不计入判据）' : ''}`,
  );
  return {
    task: task.id,
    face: task.face,
    arm: label,
    repeat,
    why: task.why ?? null,
    candidates: task.candidates ?? null,
    finish: result.finish,
    latency_ms: result.ms,
    prompt_tokens: result.usage?.prompt_tokens ?? null,
    completion_tokens: result.usage?.completion_tokens ?? null,
    cost: result.cost,
    request_bytes: result.request_bytes,
    cost: result.cost,
    calls: calls.map((c) => c.name),
    emitted: asked.slice(0, 300),
    answer_head: String(result.message?.content ?? '').replace(/\s+/g, ' ').trim().slice(0, 160),
    ...g,
  };
}

// ---------- 一道题在两个臂上各跑一次（交替进行，见主流程的注释） ----------
async function runTask({ task, i, provider, corpus, toolByName, temperature }) {
  const a = await runOneArm({ label: 'structured', task, provider, corpus, toolByName, temperature, repeat: i });
  const b = await runOneArm({ label: 'flat', task, provider, corpus, toolByName, temperature, repeat: i });
  return [a, b];
}

// ---------- 汇总与报告 ----------
function tally(rows, arm) {
  const pick = (face) => rows.filter((r) => r.arm === arm && r.face === face);
  const rate = (list, key, want = true) => {
    const hit = list.filter((r) => r[key] === want).length;
    const n = list.filter((r) => r[key] != null).length;
    return n === 0 ? '—' : `${hit}/${n}`;
  };
  const sel = pick('select'), ctl = pick('control'), attr = pick('attribution');
  const sum = (list, key) => list.reduce((a, r) => a + (typeof r[key] === 'number' ? r[key] : 0), 0);
  return {
    select_hit: rate(sel, 'tool_name_hit'),
    select_args: rate(sel, 'args_json_ok'),
    select_required: rate(sel, 'required_keys_hit'),
    select_hallucinated: rate(sel, 'hallucinated_tool'),
    control_spurious: rate(ctl, 'spurious_call'),
    attribution: rate(attr, 'attributed_correctly'),
    protocol: rate(sel.concat(attr), 'protocol_followed'),
    prompt_tokens: sum(sel.concat(ctl, attr), 'prompt_tokens'),
    bytes: sum(sel.concat(ctl, attr), 'request_bytes'),
    ms: sum(sel.concat(ctl, attr), 'latency_ms'),
    cost: sum(sel.concat(ctl, attr), 'cost'),
    truncated: sel.concat(ctl, attr).filter((r) => r.truncated).length,
    errored: sel.concat(ctl, attr).filter((r) => r.error).length,
  };
}

function renderReport({ corpus, provider, temperature, rows, repeat, generatedAt }) {
  const A = tally(rows, 'structured');
  const B = tally(rows, 'flat');
  const slots = new Map();
  for (const r of rows) {
    const key = `${r.task}#${r.repeat}`;
    const e = slots.get(key) ?? { task: r.task, face: r.face, repeat: r.repeat };
    e[r.arm] = r;
    slots.set(key, e);
  }
  const byTask = new Map();
  for (const r of rows) if (!byTask.has(r.task)) byTask.set(r.task, r.face);
  // 重复之间是否给出**同一个判据结论**——不等价于「跑两次取其一」，而是这道读数能不能重复的尺子。
  const stability = (() => {
    let comparable = 0;
    const flips = [];
    for (const [task, face] of byTask) {
      const rs = rows.filter((r) => r.task === task);
      const key = face === 'select' ? 'tool_name_hit' : face === 'control' ? 'spurious_call' : 'attributed_correctly';
      for (const arm of ['structured', 'flat']) {
        const vals = rs.filter((r) => r.arm === arm && r[key] != null).map((r) => r[key]);
        if (vals.length < 2) continue;
        comparable++;
        if (new Set(vals).size > 1) flips.push(`${task}（${arm === 'structured' ? 'A' : 'B'}：${vals.join(' / ')}）`);
      }
    }
    return { comparable, flips };
  })();
  const cell = (r, key) => (r ? (r[key] === null || r[key] === undefined ? '–' : r[key] === true ? '✓' : r[key] === false ? '✗' : String(r[key])) : '–');
  const lines = [];
  lines.push('# 结构化 vs 拍平：真实模型上的对照读数');
  lines.push('');
  lines.push('> ⚠️ **生成物，不手改。** 唯一写者是 `node e2e/ab/run.mjs`。');
  lines.push('> 判据口径归 `e2e/ab/tasks.mjs` 与 `e2e/ab/flatten.mjs`，');
  lines.push('> **结论与边界**归 [`docs/registry/task/S-end③.md`](../registry/task/S-end③.md)（本文件不写结论）。');
  lines.push('');
  lines.push(`生成于 ${generatedAt} · 模型 \`${provider.model}\`（provider \`${provider.id}\`）· 题 ${byTask.size} 道 × 两臂 × ${repeat} 次`);
  lines.push(
    `温度 ${temperature}（两臂同值，实验只认「两臂一样」不认「出厂默认」）· max_tokens ${provider.max_tokens} · 两臂的信封差别只有 ` +
      '`tools` / `tool_choice` 是否存在',
  );
  lines.push(`上下文来源：真实链路捕获（场景「${corpus.scenario}」，captured ${corpus.captured_at}）· ` +
    `system 前缀 ${corpus.request.messages.filter((m) => m.role === 'system').length} 条 · tools ${corpus.request.tools?.length ?? 0} 个`);
  lines.push('');
  lines.push('## 汇总（判据列）');
  lines.push('');
  lines.push('| 判据 | 臂 A 结构化（出厂） | 臂 B 拍平 |');
  lines.push('|---|---|---|');
  const row = (label, key) => lines.push(`| ${label} | ${A[key]} | ${B[key]} |`);
  row('select：选对工具（`tool_name_hit`）', 'select_hit');
  row('select：参数是合法 JSON（`args_json_ok`）', 'select_args');
  row('select：必填键齐全（`required_keys_hit`）', 'select_required');
  row('select：调了不存在的工具名（越少越好）', 'select_hallucinated');
  row('control：不该调用却调用（越少越好）', 'control_spurious');
  row('attribution：结果↔调用对应得回来', 'attribution');
  row('拍平侧：文本协议被遵守（臂 A 不适用）', 'protocol');
  lines.push('');
  lines.push('## 观测列（**不进判据**，与 ADR-057 同一取舍）');
  lines.push('');
  lines.push('| | 臂 A | 臂 B |');
  lines.push('|---|---|---|');
  lines.push(`| prompt token 合计 | ${A.prompt_tokens} | ${B.prompt_tokens} |`);
  lines.push(`| 请求体字节合计 | ${A.bytes} | ${B.bytes} |`);
  lines.push(`| 墙钟合计（ms） | ${A.ms} | ${B.ms} |`);
  const money = (v) => (v > 0 ? v.toFixed(6) : '—（provider 未报账单）');
  lines.push(`| provider 自报账单（usage.cost，合计） | ${money(A.cost)} | ${money(B.cost)} |`);
  lines.push(`| 未收束的题数（已从判据分母剔除） | ${A.truncated} | ${B.truncated} |`);
  lines.push(`| 请求出错的题数 | ${A.errored} | ${B.errored} |`);
  lines.push('');
  lines.push(
    `**样本健康度**：两臂合计 ${A.truncated + B.truncated} 道题未收束、${A.errored + B.errored} 道请求出错；` +
      `重复之间可比对 ${stability.comparable} 组，其中 ${stability.flips.length} 组换一次时刻就跑出了不同结论。` +
      '「未收束」不计入上面任何一条判据的分母——本机服务坏过一整段时会把回答刷成 `<unused32>` 复读，' +
      '把它读成「模型没做到」就是拿环境噪声当读数。',
  );
  lines.push('');
  lines.push('## 逐题（同一道题两臂并排；`×k` 是第 k 次重复）');
  lines.push('');
  lines.push('| 题（臂 A vs 臂 B） | A 调用 | A hit | A 参数 | A 关联 | A 答 | B 调用 | B hit | B 参数 | B 关联 | B 答 | B 协议 |');
  lines.push('|---|---|---|---|---|---|---|---|---|---|---|---|');
  for (const e of slots.values()) {
    const a = e.structured, b = e.flat;
    const showCalls = (r) => (r ? (r.calls && r.calls.length ? r.calls.join('+') : r.error ? '（出错）' : '—') : '–');
    const showAnswer = (r) => (r ? (r.inconclusive ? '（未收束）' : r.answer_head ? `\`${r.answer_head.replace(/\|/g, '\\|').slice(0, 40)}\`` : '—') : '–');
    lines.push(
      `| ${e.task}${repeat > 1 ? ` ×${e.repeat}` : ''}（${e.face}） | ${showCalls(a)} | ${cell(a, 'tool_name_hit')}${a?.hallucinated_tool ? '⚠' : ''} | ` +
        `${cell(a, 'args_json_ok')}/${cell(a, 'required_keys_hit')} | ${cell(a, 'attributed_correctly')} | ${showAnswer(a)} | ` +
        `${showCalls(b)} | ${cell(b, 'tool_name_hit')}${b?.hallucinated_tool ? '⚠' : ''} | ` +
        `${cell(b, 'args_json_ok')}/${cell(b, 'required_keys_hit')} | ${cell(b, 'attributed_correctly')} | ${showAnswer(b)} | ${cell(b, 'protocol_followed')} |`,
    );
  }
  lines.push('');
  lines.push('`类`：`select` = 该调哪个工具；`control` = 不该调用；`attribution` = 结果与调用的对应关系。');
  lines.push('`hit` 后的 ⚠ = 调了工具面上不存在的名字。`参数` 列是 `合法JSON/必填键齐`。`—` = 未调用/未作答，`–` = 该题不适用。');
  if (stability.flips.length > 0) {
    lines.push('');
    lines.push(`重复之间的判据翻转：${stability.flips.join('、')}——同一道题同一臂换个时刻跑，结论会变，读数请按「${stability.comparable} 组可比对里有 ${stability.flips.length} 组翻过」来看，不要当逐题定论。`);
  }
  lines.push('');
  lines.push('## 逐题原文（臂各自实际送出与收到的东西）');
  lines.push('');
  for (const e of slots.values()) {
    lines.push(`### ${e.task}（${e.face}）· 第 ${e.repeat} 次`);
    const meta = e.structured ?? e.flat;
    if (meta?.why) lines.push(`- 测的是：${meta.why}`);
    if (meta?.candidates != null && meta.candidates > 1) {
      lines.push(`- 候选条数 ${meta.candidates} ⇒ 不读关联、纯按到达顺序或瞎挑的基线约 ${meta.candidates} 分之一`);
    }
    for (const [label, r] of [['A 结构化', e.structured], ['B 拍平', e.flat]]) {
      if (!r) continue;
      lines.push(`- **${label}**：finish=${r.finish ?? '–'} · ${r.latency_ms ?? '–'}ms · prompt ${r.prompt_tokens ?? '–'}tok · ${r.request_bytes ?? '–'}B`);
      if (r.emitted) lines.push(`  - 送出：\`${r.emitted}\``);
      if (r.answer_head) lines.push(`  - 正文：\`${r.answer_head}\``);
      if (r.error) lines.push(`  - 错误：\`${r.error}\``);
    }
  }
  lines.push('');
  return lines.join('\n');
}

// ---------- 主流程 ----------
const provider = loadProvider(arg('provider', 'LMStudio'));
const corpus = loadCorpus();
const toolByName = new Map((corpus.request.tools ?? []).map((t) => [t.function?.name ?? t.name, t]));
const filter = arg('tasks') ? new Set(arg('tasks').split(',')) : null;
const tasks = TASKS.filter((t) => !filter || filter.has(t.id));
const temperature = Number(arg('temperature', '0'));
const repeat = Number(arg('repeat', '1'));

console.log(`provider ${provider.id} · ${provider.model} · ${provider.api_base}（api_key 不外印）`);
console.log(`题集 ${tasks.length} 道 × 两臂 × ${repeat} 次 · temperature ${temperature}`);

if (flag('dry-run')) {
  let bytesA = 0, bytesB = 0, toolsA = 0;
  for (const task of tasks) {
    const s = structuredBodyFor(corpus, task);
    s.temperature = temperature; s.stream = false;
    const f = flattenRequest(s);
    bytesA += JSON.stringify(s).length;
    bytesB += JSON.stringify(f).length;
    toolsA += s.tools.reduce((a, t) => a + JSON.stringify(t).length, 0);
    console.log(`  ${task.id}（${task.face}）：A ${JSON.stringify(s).length}B / tools ${s.tools.length} 个 → B ${JSON.stringify(f).length}B / tools ${f.tools ? '还在' : '已移出'}`);
  }
  console.log(`\n合计：臂 A ${bytesA}B · 臂 B ${bytesB}B（工具面正文化净增 ${bytesB - bytesA}B，其中 tools 原样 ${toolsA}B）`);
  console.log('（--dry-run：未发出任何请求）');
  process.exit(0);
}

const rows = [];
// 两臂**逐题交替**（同一道题的 A 与 B 紧挨着发）：按臂分块的跑法一旦本机服务在
// 中途变慢 / 变坏，那段漂移会**整块落在后跑的那一臂**上——实测撞到过一次
// （一整段 `<unused32>` 复读把臂 B 的一轮尾巴全打掉）。交替之后环境漂移对两臂同等。
for (let i = 0; i < repeat; i++) {
  console.log(`\n── 第 ${i + 1}/${repeat} 轮 ──`);
  for (const task of tasks) {
    rows.push(...(await runTask({ task, i: i + 1, provider, corpus, toolByName, temperature })));
  }
}

const generatedAt = new Date().toISOString();
const report = renderReport({ corpus, provider, temperature, rows, repeat, generatedAt });
writeFileSync(REPORT_PATH, report);
console.log(`\n报告已写 ${REPORT_PATH.replace(/\\/g, '/')}`);

const jsonOut = arg('json');
if (jsonOut) writeFileSync(resolve(jsonOut), JSON.stringify({ generatedAt, provider: { id: provider.id, model: provider.model }, temperature, repeat, rows }, null, 2));
console.log(`原始行 ${rows.length} 条${jsonOut ? ` · json 已写 ${resolve(jsonOut)}` : '（加 --json=<路径> 落盘）'}`);

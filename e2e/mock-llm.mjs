//! Mock LLM 服务（OpenAI Chat Completions 兼容，SSE 流式）。
//!
//! 用法：
//!   node e2e/mock-llm.mjs --port 18081 --scenarios ./scenarios.json
//!
//! 能力：
//! - `POST /v1/chat/completions`（stream=true SSE / stream=false JSON）
//! - `GET  /v1/models` 上下文探测（context_probe 走这里，返回 max_model_len）
//! - `GET  /_health`   健康检查
//! - `GET  /_requests` 读回已记录的请求（测试断言用）
//! - `POST /_reset`    清空请求记录与场景游标
//!
//! 场景编排：`--scenarios` 指向一个 JSON 文件，形如
//! ```json
//! {
//!   "scenarios": [
//!     { "id": "echo",       "match": "你好",   "content": "你好，我是 mock。" },
//!     { "id": "tool-call",  "toolCalls": [{ "id": "call_1", "name": "vdfs_read", "arguments": {"path": "/a"} }],
//!                           "content": "工具调用完成。" },
//!     { "id": "sse-flood",  "chunks": ["分", "片", "输", "出"], "chunkDelayMs": 15 },
//!     { "id": "http-500",   "status": 500, "error": "mock 注入的服务端错误" },
//!     { "id": "flaky",      "match": "Distill", "failTimes": 2, "content": "第 3 个逻辑请求起正常" }
//!   ],
//!   "fallback": { "id": "default", "content": "（默认回复）" }
//! }
//! ```
//! 匹配规则：按数组顺序，`match` 是对**最后一条 user 消息**的子串匹配；
//! 不带 `match` 的场景可设 `once: true`（只用一次，适合「第 N 轮调工具」的编排）。
//! `failTimes: N`：前 N 次**逻辑请求**返回 `errorStatus`（默认 500）错误，之后同场景正常应答。
//! 无匹配时走 `fallback`；都没有则回复固定占位文本。
//!
//! ## 「逻辑请求」= 一个请求体，不是一次 HTTP 命中
//!
//! 客户端对 5xx 会退避重发（`model/http.rs::execute_post_with_abort` 的
//! `MAX_RETRIES`），重发**复用同一份序列化字节**——于是同一次摘要请求会命中 mock
//! 5 次。按 HTTP 命中计数的话，「前 3 次压缩失败」会在第 1 次压缩里就烧光名额
//! （第 4 次重发拿到成功响应），熔断、重试这些**按逻辑请求计数**的契约全部对不上。
//!
//! 因此 `failTimes` 按**请求体指纹**计数：首次见到的请求体消耗一个名额并定下
//! 结论，同一请求体的重放**沿用该结论**（仍失败 ⇒ 继续 500，直到客户端重试耗尽
//! 拿到 `Err`）。这与「服务端这一段时间对这类请求就是坏的」直觉一致，也让用例
//! 不必知道 `MAX_RETRIES` 是几。
//!
//! 注入的错误附带 `retry-after: 0`（服务端明示可立即重试），否则用例要空等整段
//! 指数退避（3 次逻辑失败 ≈ 22s）。要验退避节奏就用静态 `status`，或显式给
//! `retryAfterSec`。

import http from 'node:http';
import { setTimeout as sleep } from 'node:timers/promises';

// ---------- 参数 ----------
const args = process.argv.slice(2);
function argOf(name, dflt) {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] ? args[i + 1] : dflt;
}
const PORT = Number(argOf('port', '18081'));
const SCENARIOS_PATH = argOf('scenarios', null);

// ---------- 状态 ----------
/** @type {{scenarios: any[], fallback: any|null}} */
let plan = { scenarios: [], fallback: null };
if (SCENARIOS_PATH) {
  plan = JSON.parse(await import('node:fs/promises').then((fs) => fs.readFile(SCENARIOS_PATH, 'utf8')));
}
const requests = []; // 每次调用的完整请求体 + 时间戳
const onceUsed = new Set();
// `failTimes` 的名额按**逻辑请求**消耗：键 = 场景 id + 请求体指纹，值 = 该请求
// 定下的结论（fail / ok）。同一请求体的退避重放沿用首次结论（见文件头说明）。
const failOutcome = new Map();
const failLogicalHits = new Map(); // 场景 id → 已消耗的逻辑名额

/**
 * 本轮是否刚拿到工具结果、还没作答（**按消息序列**判，不看文本）。
 *
 * ## 为什么这个函数从「扫文本」变成「读 role」
 *
 * 它连同它的 `</对话历史>` 切段逻辑，是为**拍平形态**写的：`full` 档曾把整段
 * 窗口渲染成一条 user 消息，于是「有没有工具结果」只能扫文本里的 `工具结果(` 行。
 *
 * ADR-048a 落地后**那种形态不存在了**——送进模型的是消息数组，工具结果就是
 * `role: "tool"` 的消息。判据于是回到最简形式，也回到最不容易写错的形式：
 * 「末条是不是 tool 消息」在消息数组上**直接可读**；文本形态要靠「切标签 / 找
 * 前缀行」，而那两版判据都栽过（见 git log：本函数的前两版分别导致「无限工具循环」
 * 与「新一轮根本不请求工具」）。
 *
 * @param {Array<{role?: string}>} msgs 本轮请求的消息数组
 * @returns {boolean} 末条是工具结果消息
 */
function endsWithToolResult(msgs) {
  return msgs[msgs.length - 1]?.role === 'tool';
}

/** 逻辑请求指纹：场景 id + 请求体（重发复用同一份字节 ⇒ 指纹相同）。 */
function requestFingerprint(scenarioId, body) {
  return `${scenarioId}\n${JSON.stringify(body)}`;
}

function loadScenarios() {
  return plan.scenarios ?? [];
}

function pickScenario(body) {
  const msgs = body.messages ?? [];
  // 本轮用户文本：**最后一条 role=user 的内容**。
  //
  // 结构化（ADR-048a）之后这就是本轮那句话本身——历史里的用户发言是**独立**的
  // user 消息，不会混进这一条。拍平形态下它曾包含整段历史，于是下面的「取位置最靠后」
  // 才是必需的；现在它是**语义正确**而不只是权宜之计，两者恰好重合。
  const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
  const userText =
    typeof lastUser?.content === 'string'
      ? lastUser.content
      : Array.isArray(lastUser?.content)
        ? lastUser.content.map((p) => p.text ?? '').join('')
        : '';

  // 工具结果轮：判「**末条是不是 `role: tool` 消息**」。结构化之后这就是直接可读的
  // 一行判断——先前那两版（切 `</对话历史>` 标签 / 锚 `工具结果(` 行首）都是在给
  // 拍平形态打补丁，各自的误判症状完全不同且都指不到这里：只认 v1 ⇒ `afterTool`
  // 永不命中 ⇒ 无限工具循环（撞 120s 超时，不是断言红）；判宽了 ⇒ 新一轮的第一次
  // 请求被误认成工具结果轮 ⇒ 根本不请求工具（报「应恰好一格熔断，实得 0 格」）。
  const isToolResultTurn = endsWithToolResult(msgs);

  // ⚠️ `afterTool` 是**优先**而不是**硬过滤**。
  //
  // 原先这里是硬过滤（`filter(s => isToolResultTurn ? s.afterTool : !s.afterTool)`），
  // 于是一个**没声明任何 `afterTool` 场景**的用例，在工具结果轮上池子被过滤成空 ⇒
  // 直接落 `plan.fallback`（默认回复）。实测 t38 报「轮 1 历史应含轮 0 的收束正文」，
  // 而真因在 mock 里：它要的 `r0-final`（无 `match` 的普通场景）本该在工具结果轮
  // 继续可用。这类误判的杀伤力在于**症状落在下游的历史断言上**，离真因隔两层。
  //
  // 硬过滤唯一说得通的场景是「本轮必须用 `afterTool` 回应，否则带 `match` 的工具
  // 调用场景会反复命中 → 无限工具循环」——而那件事由**下面的回退**已经兜住了：
  // 优先池选不出东西时再退回全池，选中的场景若带 `match` 也照样按位置匹配。
  const all = loadScenarios();
  const pool = isToolResultTurn
    ? all.filter((s) => s.afterTool)
    : all.filter((s) => !s.afterTool);

  // ⚠️ **取位置最靠后的那个匹配，不是第一个匹配的**。
  //
  // 起因：拍平形态下 `userText` 是**整段历史**（含此前每一轮），于是「先匹配
  // `普通提问`（第 1 轮）、后匹配 `触发故障`（第 2 轮）」这种场景表会命中**第一条**
  // ⇒ 故障注入静默不发生、那一轮假成功、CLI 退出 0。症状出现在**被测系统之外**
  // （用例断言「失败轮应非零退出」失败），指向却像「v2 把失败吞了」，极易误诊成
  // 产品缺陷。
  //
  // 结构化之后 `userText` 已经是本轮那句话，所以「最靠后」此时**恰好**等于「唯一」——
  // 但**判据本身要留着**：它防的是「又有人让 userText 变回整段历史」这类改动，
  // 而那种改动的症状（故障注入静默失效）离真因隔两层，很难现场查。
  //
  // `match` 缺失的规则不参与位置比较（位置对它没有意义），只在还没有任何位置命中时兜底。
  const choose = (candidates) => {
    let best = null;
    let bestAt = -1;
    for (const s of candidates) {
      if (s.once && onceUsed.has(s.id)) continue;
      if (s.match) {
        const at = userText.lastIndexOf(s.match);
        if (at < 0) continue;
        if (at > bestAt) {
          best = s;
          bestAt = at;
        }
      } else if (best === null) {
        best = s;
      }
    }
    return best;
  };

  // 优先池（工具结果轮 = 只认 `afterTool`；否则只认非 `afterTool`）⇒ 选不出就
  // **退回全池**。退回而不是直接落 fallback：前者让「没声明 afterTool 的用例」在
  // 工具结果轮上照常用它声明过的普通场景，后者把整轮的回答换成默认回复——症状是
  // 下游断言「历史里没有上一轮收束」，离真因隔两层。
  const best = choose(pool) ?? (isToolResultTurn ? choose(all) : null);
  if (best !== null) {
    if (best.once) onceUsed.add(best.id);
    return { scenario: best, userText };
  }
  return { scenario: plan.fallback ?? { id: 'default', content: '（mock 默认回复）' }, userText };
}

// ---------- 故障注入：逻辑请求的判定与响应头 ----------

/**
 * `failTimes` 的判定：名额按**逻辑请求**（场景 id + 请求体指纹）消耗，同一请求体的
 * 退避重放沿用首次结论。客户端对 5xx 会拿同一份序列化字节重发（见文件头「逻辑请求」），
 * 一次逻辑失败因此命中 1 + MAX_RETRIES 次，按 HTTP 命中计数会把名额烧光。
 *
 * @returns {{ hit: number, fail: boolean }} `hit` = 该请求是本场景的第几个逻辑请求
 */
function failVerdict(scenario, body) {
  const fp = requestFingerprint(scenario.id, body);
  const seen = failOutcome.get(fp);
  if (seen) return seen;
  const hit = (failLogicalHits.get(scenario.id) ?? 0) + 1;
  failLogicalHits.set(scenario.id, hit);
  const verdict = { hit, fail: hit <= scenario.failTimes };
  failOutcome.set(fp, verdict);
  return verdict;
}

/**
 * 注入错误的响应头。`failTimes` 路径默认 `retry-after: 0`：服务端明示可立即重试，
 * 用例因此不必空等客户端的指数退避（3 次逻辑失败约为 22s）；要验退避节奏就用静态
 * `status`，或显式给 `retryAfterSec`。
 */
function errorHeaders(scenario) {
  const headers = { 'content-type': 'application/json' };
  const retryAfter = scenario.retryAfterSec ?? (scenario.failTimes > 0 ? 0 : null);
  if (retryAfter != null) headers['retry-after'] = String(retryAfter);
  return headers;
}

// ---------- OpenAI 线格式 ----------
function chunkOf(id, model, delta, finish = null, usage = null) {
  const c = {
    id,
    object: 'chat.completion.chunk',
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [{ index: 0, delta, finish_reason: finish }],
  };
  if (usage) c.usage = usage;
  return c;
}

/** 把场景编译成 [delta, finish] 事件序列（不区分 stream/非 stream，最后统一折算）。 */
function* buildEvents(scenario, model, requestId) {
  // ⓪ 思考（reasoning_content，真实模型在正文之前输出）
  for (const piece of scenario.reasoning ?? []) {
    yield chunkOf(requestId, model, { reasoning_content: piece });
  }
  // ① 工具调用（先于正文，与真实模型行为一致）
  for (const [i, tc] of (scenario.toolCalls ?? []).entries()) {
    const argsStr = typeof tc.arguments === 'string' ? tc.arguments : JSON.stringify(tc.arguments ?? {});
    yield chunkOf(requestId, model, {
      tool_calls: [
        { index: i, id: tc.id ?? `call_${i}`, type: 'function', function: { name: tc.name, arguments: '' } },
      ],
    });
    // 参数分两片流式吐出，逼出后端「参数流式聚合」路径
    const mid = Math.max(1, Math.floor(argsStr.length / 2));
    yield chunkOf(requestId, model, { tool_calls: [{ index: i, function: { arguments: argsStr.slice(0, mid) } }] });
    yield chunkOf(requestId, model, { tool_calls: [{ index: i, function: { arguments: argsStr.slice(mid) } }] });
  }
  // ② 正文分片
  const chunks = scenario.chunks ?? (scenario.content != null ? [scenario.content] : []);
  for (const piece of chunks) yield chunkOf(requestId, model, { content: piece });
  // ③ 终止帧
  const finish = (scenario.toolCalls ?? []).length > 0 ? 'tool_calls' : 'stop';
  yield chunkOf(requestId, model, {}, finish, { prompt_tokens: 64, completion_tokens: 32 });
}

// ---------- HTTP 处理 ----------
const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${PORT}`);

  // 管控面（connection: close —— 避免 Windows 上 Node fetch keep-alive 的 libuv 退出断言噪音）
  const noKeepAlive = { 'content-type': 'application/json', connection: 'close' };
  if (url.pathname === '/_health') {
    res.writeHead(200, noKeepAlive);
    return res.end(JSON.stringify({ ok: true, scenarios: loadScenarios().length }));
  }
  if (url.pathname === '/_requests') {
    res.writeHead(200, noKeepAlive);
    return res.end(JSON.stringify({ requests }));
  }
  if (url.pathname === '/_reset') {
    requests.length = 0;
    onceUsed.clear();
    failOutcome.clear();
    failLogicalHits.clear();
    res.writeHead(200, noKeepAlive);
    return res.end(JSON.stringify({ ok: true }));
  }

  // 上下文探测：GET /v1/models → data[].max_model_len
  if (url.pathname === '/v1/models') {
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ data: [{ id: 'mock-model', max_model_len: 262144 }] }));
  }

  if (url.pathname.endsWith('/chat/completions')) {
    // 请求体必须**先攒 Buffer 再整体解码**（`body += part` 是逐块解码：一个 3 字节
    // 中文字符被 socket 分块切在中间时，两半各解出一个 U+FFFD —— 同一份字节的重发
    // 会因分块边界不同而解出**差 1 个字符**的请求体 ⇒ 指纹分裂 ⇒ failTimes 名额与
    // 逻辑请求数被虚增，e2e 按「逻辑请求」计数的用例随之偶发飘红）。
    const parts = [];
    for await (const part of req) parts.push(part);
    const body = Buffer.concat(parts).toString('utf8');
    let parsed = {};
    try { parsed = JSON.parse(body); } catch { /* 容错：空体 */ }
    requests.push({ at: Date.now(), path: url.pathname, body: parsed, contentType: req.headers['content-type'] ?? null });

    const { scenario } = pickScenario(parsed);
    const model = parsed.model ?? 'mock-model';
    const requestId = `chatcmpl-${Date.now().toString(16)}-${requests.length}`;

    // 故障注入：非 2xx（静态）
    if (scenario.status && scenario.status >= 400) {
      res.writeHead(scenario.status, errorHeaders(scenario));
      return res.end(JSON.stringify({ error: { message: scenario.error ?? 'mock 注入的错误', type: 'mock_error' } }));
    }

    // 故障注入：前 `failTimes` 次**逻辑请求**失败，之后同场景正常应答——
    // 支撑「连续失败 → 熗断 → 重试成功」这类**按时间变化**的编排（静态 status 做不到）。
    // 名额按请求体指纹消耗、同一请求体的重放沿用首次结论：见文件头「逻辑请求」。
    if (scenario.failTimes > 0) {
      const verdict = failVerdict(scenario, parsed);
      if (verdict.fail) {
        res.writeHead(scenario.errorStatus ?? 500, errorHeaders(scenario));
        return res.end(
          JSON.stringify({
            error: {
              message: scenario.error ?? `mock 第 ${verdict.hit} 个逻辑请求注入的错误`,
              type: 'mock_error',
            },
          }),
        );
      }
    }

    const events = [...buildEvents(scenario, model, requestId)];

    // 非 stream：一次性 JSON
    if (parsed.stream === false) {
      const text = (scenario.chunks ?? [scenario.content ?? '']).join('');
      const toolMsgs = (scenario.toolCalls ?? []).map((tc, i) => ({
        id: tc.id ?? `call_${i}`,
        type: 'function',
        function: { name: tc.name, arguments: typeof tc.arguments === 'string' ? tc.arguments : JSON.stringify(tc.arguments ?? {}) },
      }));
      res.writeHead(200, { 'content-type': 'application/json' });
      return res.end(
        JSON.stringify({
          id: requestId,
          object: 'chat.completion',
          created: Math.floor(Date.now() / 1000),
          model,
          choices: [{
            index: 0,
            message: { role: 'assistant', content: text || null, ...(toolMsgs.length ? { tool_calls: toolMsgs } : {}) },
            finish_reason: toolMsgs.length ? 'tool_calls' : 'stop',
          }],
          usage: { prompt_tokens: 64, completion_tokens: 32 },
        }),
      );
    }

    // stream：SSE
    res.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache',
      connection: 'keep-alive',
    });
    const delay = scenario.chunkDelayMs ?? 0;
    for (const ev of events) {
      res.write(`data: ${JSON.stringify(ev)}\n\n`);
      if (delay) await sleep(delay);
    }
    res.write('data: [DONE]\n\n');
    return res.end();
  }

  res.writeHead(404, { 'content-type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `mock-llm: no route ${req.method} ${url.pathname}` } }));
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[mock-llm] listening http://127.0.0.1:${PORT} (scenarios: ${loadScenarios().length})`);
});

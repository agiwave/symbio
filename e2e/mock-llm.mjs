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
 * 「这条请求是工具结果轮」的**第二种**判据（v2 prompt 形状专用）。
 *
 * v2 形状下 `Reasoner::render_prompt` = `transcript` 投影的 `to_prompt`，它把多轮
 * 历史包在 `<对话历史> … </对话历史>` 里、**当前轮平铺在标签之后**；本轮刚发生的
 * 工具交换由运行器就地追加在这后面（`render_tool_exchange`）。
 *
 * 所以判据是：**`</对话历史>` 之后那段里有没有 `工具结果(` 行**。那一段在「本轮
 * 第一次请求」时只有当前用户发言一行（不可能含工具结果），在「工具结果轮」时多出
 * 整段 exchange。
 *
 * ## 试过两种都错的写法（记下来，因为它们的症状都不指这儿）
 *
 * 1. `/^工具结果\(…\)/m` —— `m` 让 `^` 匹配任意行首 ⇒ **历史里**上一轮
 *    `artifact.added` 渲染出的那条也算命中 ⇒ 新一轮第一次请求被误判成工具结果轮 ⇒
 *    直接返回收尾正文、**根本不请求工具**。症状落在下游「应恰好一格熔断，实得 0 格」。
 * 2. 「最后一次 `助手请求工具` 之后有没有 `工具结果`」（次序判据）—— 也不行：
 *    **投影路径的 `工具结果` 行前面没有 `助手请求工具` 行**（`transcript` 投影只渲染
 *    调用结果、不渲染调用，见 `transcript.rs` 的 `EVENT_ARTIFACT_ADDED` 分支），
 *    于是「最后一次调用行」恒为 −1，判据恒真 ⇒ 与第 1 种同样的误判。
 *
 * 顺带说明为什么**不能锚「末行是工具结果」**：`outcome.text` 与投影的 `entry.text`
 * 都可能是多行正文（工具结果是 JSON 形态）⇒ 末行只是那段正文的一半 ⇒ 判据恒假 ⇒
 * 退回「永不命中 `afterTool`」⇒ 无限工具循环，撞 `runCli` 的 120s 超时。
 * 用标签切段则与正文是否多行无关。
 *
 * @param {string} prompt v2 形状下的整条 user 消息内容
 * @returns {boolean} 本轮是否刚拿到工具结果、还没作答
 */
function endsWithToolResult(prompt) {
  const text = String(prompt ?? '');
  const cut = text.lastIndexOf('</对话历史>');
  // 无标签 ⇒ 投影只有**一条**条目（`to_prompt` 的单条目分支返回裸文本）⇒ 整段就是
  // 「当前轮 + 就地追加的 exchange」，扫全段。
  //
  // ⚠️ 这里正是先前两版判据栽跟头的地方，区别在**为什么**扫全段是安全的：只有
  // 「投影 ≥ 2 条」才会带标签，而带标签时历史里必然可能有 `artifact.added` 渲染出的
  // 工具结果行——那种情况一律被切段排除在外。零标签时投影里根本没有第二条，扫到的
  // `工具结果(` 只可能来自运行器追加的 exchange。
  const current = cut < 0 ? text : text.slice(cut + '</对话历史>'.length);
  return current.split('\n').some((l) => l.startsWith('工具结果('));
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
  const last = msgs[msgs.length - 1];
  // 本轮用户文本：**最后一条 role=user 的内容**（v1 形状下就是本轮那句话）。
  const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
  const userText =
    typeof lastUser?.content === 'string'
      ? lastUser.content
      : Array.isArray(lastUser?.content)
        ? lastUser.content.map((p) => p.text ?? '').join('')
        : '';

  // ⚠️ **工具结果轮的判定必须同时认两种 prompt 形状**（出厂档位是 `full`，
  // 不认它的后果是工具用例整体失效，且症状指不到这里）：
  //
  // - v1（`v2_mode` 非 `full`）：对话拆成 messages 数组，工具结果是独立的
  //   `role=tool` 消息 ⇒ 判 `last.role === 'tool'`；
  // - v2（`full` 档）：`ProviderLlmAdapter::generate_turn` 把
  //   `Reasoner::render_prompt` 渲染的**整段窗口转写**作为**一条** user 消息发出
  //   ⇒ **永远没有 `role=tool` 消息** ⇒ 只能判 prompt 的末行。
  //
  // 两种误判各自表现为：只认 v1 ⇒ `afterTool` 永不命中 ⇒ 带 `match` 的工具调用
  // 场景反复命中 ⇒ **无限工具循环**（实测 t33 撞 `runCli` 120s 超时，节点转「挂起」，
  // 不是断言红）；判宽了（只锚行首）⇒ 新一轮的第一次请求被误认成工具结果轮 ⇒
  // 直接返回收尾正文、**根本不请求工具**（实测 t33 报「应恰好一格熔断，实得 0 格」，
  // 而真因在 mock 里）。
  const isToolResultTurn = last?.role === 'tool' || endsWithToolResult(lastUser?.content);

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

  // ⚠️ **取位置最靠后的那个匹配，不是第一个匹配的**——同样因为两种形状下
  // `userText` 的含义不同：
  //
  // v1 下 `userText` = 本轮那句话（短）；v2 下它是**整段历史**（长，含此前每一轮）。
  // 于是「先匹配 `普通提问`（第 1 轮）、后匹配 `触发故障`（第 2 轮）」这种场景表
  // 在 v1 下按顺序命中第 2 轮的规则，在 v2 下会命中**第一条** ⇒ 故障注入静默不发生，
  // 那一轮假成功、CLI 退出 0。症状出现在**被测系统之外**（用例断言「失败轮应非零退出」
  // 失败），指向却像「v2 把失败吞了」，极易误诊成产品缺陷。
  //
  // 转写按事件序渲染、当前发言在**末尾**，所以「匹配位置最靠后」= 「匹配本轮发言」，
  // 两种形状下同一条规则都指向同一轮。`match` 缺失的规则不参与位置比较（位置对它
  // 没有意义），只在还没有任何位置命中时兜底。
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

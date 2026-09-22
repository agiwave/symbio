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
//!     { "id": "http-500",   "status": 500, "error": "mock 注入的服务端错误" }
//!   ],
//!   "fallback": { "id": "default", "content": "（默认回复）" }
//! }
//! ```
//! 匹配规则：按数组顺序，`match` 是对**最后一条 user 消息**的子串匹配；
//! 不带 `match` 的场景可设 `once: true`（只用一次，适合「第 N 轮调工具」的编排）。
//! 无匹配时走 `fallback`；都没有则回复固定占位文本。

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

function loadScenarios() {
  return plan.scenarios ?? [];
}

function pickScenario(body) {
  const msgs = body.messages ?? [];
  const last = msgs[msgs.length - 1];
  // 工具结果轮：请求的最后一条是 role=tool（模型刚收到工具结果继续生成）。
  // 这类请求必须用 afterTool 场景回应，否则带 match 的工具调用场景会反复命中 → 无限工具循环。
  const isToolResultTurn = last?.role === 'tool';
  const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
  const userText =
    typeof lastUser?.content === 'string'
      ? lastUser.content
      : Array.isArray(lastUser?.content)
        ? lastUser.content.map((p) => p.text ?? '').join('')
        : '';
  const pool = loadScenarios().filter((s) => (isToolResultTurn ? s.afterTool : !s.afterTool));
  for (const s of pool) {
    if (s.once && onceUsed.has(s.id)) continue;
    if (s.match && !userText.includes(s.match)) continue;
    if (s.once) onceUsed.add(s.id);
    return { scenario: s, userText };
  }
  return { scenario: plan.fallback ?? { id: 'default', content: '（mock 默认回复）' }, userText };
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
    res.writeHead(200, noKeepAlive);
    return res.end(JSON.stringify({ ok: true }));
  }

  // 上下文探测：GET /v1/models → data[].max_model_len
  if (url.pathname === '/v1/models') {
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ data: [{ id: 'mock-model', max_model_len: 262144 }] }));
  }

  if (url.pathname.endsWith('/chat/completions')) {
    let body = '';
    for await (const part of req) body += part;
    let parsed = {};
    try { parsed = JSON.parse(body); } catch { /* 容错：空体 */ }
    requests.push({ at: Date.now(), path: url.pathname, body: parsed });

    const { scenario } = pickScenario(parsed);
    const model = parsed.model ?? 'mock-model';
    const requestId = `chatcmpl-${Date.now().toString(16)}-${requests.length}`;

    // 故障注入：非 2xx
    if (scenario.status && scenario.status >= 400) {
      res.writeHead(scenario.status, { 'content-type': 'application/json' });
      return res.end(JSON.stringify({ error: { message: scenario.error ?? 'mock 注入的错误', type: 'mock_error' } }));
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

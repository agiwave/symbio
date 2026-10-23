#!/usr/bin/env node
// Mock HTTP 端点（web 外取工具的本地靶子）。
//
// ## 为什么需要一个「本地 HTTP 服务」当靶子
//
// `web_fetch` / `http_request` 是**真的会发网络请求**的工具。让 e2e 打真外网有
// 三个致命问题：CI 无网（红成一片且与代码无关）、内容会变（断言今天绿明天红）、
// 以及**被测系统之外**的故障（对方限流、DNS 抖动）会被误诊成产品缺陷。
// 本地靶子把「网络」这一层换成**确定性的**读数：状态码、`content-type`、正文、
// 以及「请求真的发出来了没有」（`/_requests` 留痕）。
//
// ## 边界在哪（为什么不违反「不许给 mock 写判据」的纪律）
//
// 本夹具扮演的是**真实 HTTP 服务器**（被测系统对面那一端），不是被测系统自己。
// 判据仍然从用户视角取：工具返回了什么、模型看到了什么、转写里落了什么。
// `/_requests` 只用来回答「请求有没有发出」这一件事——它是**外部世界**的观察，
// 与「读被测系统的内部状态」是两回事。
//
// ## 端点
//
// | 路径 | 行为 |
// |---|---|
// | `GET /page` | 200 `text/html`，正文含随机标记（每次启动生成） |
// | `GET /json` | 200 `application/json`，回 `{ok:true,marker}` |
// | `GET /big` | 200，正文 > 1 MiB（逼出 `web_fetch` 的截断分支） |
// | `GET /slow` | 延迟 `--slow-ms`（默认 3000）后 200 —— 配合 `web_timeout` 逼出超时 |
// | `GET /missing` | 404 `text/plain`，正文含标记 |
// | `ANY /echo` | 200，回 `{method, path, contentType, body, headers}`（验请求真的带上去了） |
// | `ANY /bot<token>/sendMessage` | 200，回 Telegram Bot API 形状 `{ok:true,result:{message_id:4210}}` |
// | `ANY /bot<token>/getUpdates` | 200，回 `{ok:true,result:[{update_id:100100}]}` |
// | `POST /search` | 200，回 Tavily 形状（正文带请求里的 query/api_key） |
// | `GET /html/` | 200，回 DuckDuckGo 结果页 HTML 形状（`result__a` / `result__snippet`） |
// | `GET /_requests` | 控制面：本进程收到过的请求留痕 |
// | `GET /_health` | 控制面：就绪探针 |
//
// 用法：`node e2e/mock-http.mjs --port 18090 [--slow-ms 3000] [--marker XXX]`

import http from 'node:http';

const args = process.argv.slice(2);
function argOf(name, dflt) {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] ? args[i + 1] : dflt;
}

const PORT = Number(argOf('port', '18090'));
const SLOW_MS = Number(argOf('slow-ms', '3000'));
// 标记词默认随机：用例侧通常显式传一个，好让断言里出现的就是「本次这一份」。
const MARKER = argOf('marker', `MOCKHTTP_${Math.random().toString(36).slice(2, 10).toUpperCase()}`);
const BIG_BYTES = Number(argOf('big-bytes', String(1024 * 1024 + 4096)));

/** 已收到的请求留痕（控制面 `/_requests` 读它）。 */
const requests = [];

/** `/big` 的正文：ASCII 填充，保证字节数可预期（多字节字符会让「> 1 MiB」算不准）。 */
function bigBody() {
  return `BEGIN_BIG ${MARKER}\n` + 'x'.repeat(BIG_BYTES) + `\nEND_BIG ${MARKER}\n`;
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${PORT}`);

  // 攒完整个请求体再解码（理由同 mock-llm：逐块解码会把多字节字符切坏）。
  const parts = [];
  for await (const part of req) parts.push(part);
  const rawBody = Buffer.concat(parts).toString('utf8');
  requests.push({
    at: Date.now(),
    method: req.method,
    path: url.pathname,
    contentType: req.headers['content-type'] ?? null,
    body: rawBody,
  });

  const noKeepAlive = { connection: 'close' };

  // ---------- 控制面 ----------
  if (url.pathname === '/_health') {
    res.writeHead(200, { 'content-type': 'application/json', ...noKeepAlive });
    return res.end(JSON.stringify({ ok: true, marker: MARKER }));
  }
  if (url.pathname === '/_requests') {
    res.writeHead(200, { 'content-type': 'application/json', ...noKeepAlive });
    return res.end(JSON.stringify({ marker: MARKER, requests }));
  }

  // ---------- 被取的目标 ----------
  if (url.pathname === '/page') {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', ...noKeepAlive });
    return res.end(`<html><body><h1>mock 页面</h1><p>${MARKER}</p></body></html>`);
  }

  if (url.pathname === '/json') {
    res.writeHead(200, { 'content-type': 'application/json', ...noKeepAlive });
    return res.end(JSON.stringify({ ok: true, marker: MARKER }));
  }

  if (url.pathname === '/big') {
    res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8', ...noKeepAlive });
    return res.end(bigBody());
  }

  if (url.pathname === '/slow') {
    // 不关连接地拖住：被测侧的 `web_timeout` 到点后自行放弃——这正是要验的那条路径。
    await new Promise((r) => setTimeout(r, SLOW_MS));
    res.writeHead(200, { 'content-type': 'text/plain; charset=utf-8', ...noKeepAlive });
    return res.end(`slow 端点终于回了 ${MARKER}`);
  }

  if (url.pathname === '/missing') {
    res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8', ...noKeepAlive });
    return res.end(`没有这个页面 ${MARKER}`);
  }

  if (url.pathname === '/echo') {
    res.writeHead(200, { 'content-type': 'application/json', ...noKeepAlive });
    return res.end(
      JSON.stringify({
        method: req.method,
        path: url.pathname,
        contentType: req.headers['content-type'] ?? null,
        body: rawBody,
        headers: { 'x-mock-probe': req.headers['x-mock-probe'] ?? null },
        marker: MARKER,
      }),
    );
  }

  // ---------- Telegram Bot API 形状（ 的本地靶子） ----------
  //
  // 真实端点是 `POST {api_base}/bot<token>/sendMessage|getUpdates`，路径里带
  // token ⇒ 不能按字面量匹配，按前缀模式认。回包形状对齐 Bot API：
  // `sendMessage` → `{ok:true,result:{message_id:…}}`；`getUpdates` →
  // `{ok:true,result:[{update_id:…}]}`（update_id 固定，好让 offset 推进可断言；
  // 不带 message ⇒ 监听器侧「无消息」即忽略，不会触发任何会话轮）。
  const tg = url.pathname.match(/^\/bot([^/]+)\/(sendMessage|getUpdates)$/);
  if (tg) {
    res.writeHead(200, { 'content-type': 'application/json', ...noKeepAlive });
    if (tg[2] === 'sendMessage') {
      return res.end(JSON.stringify({ ok: true, result: { message_id: 4210 } }));
    }
    return res.end(JSON.stringify({ ok: true, result: [{ update_id: 100100 }] }));
  }

  // ---------- 搜索服务形状（ ⑤ 的本地靶子） ----------
  //
  // `web_search` 的三家上游路径是供应商契约（Tavily/Serper `POST /search`、
  // DuckDuckGo HTML `GET /html/?q=…`），配置只替换基址 ⇒ mock 按路径认：
  // `POST /search` 回 Tavily 形状（`results[].{title,url,content}`），正文里带
  // 请求里的 query 与 api_key，好让「载荷真的上去了」可断言；`GET /html/` 回
  // DuckDuckGo 结果页 HTML 形状（`result__a` / `result__snippet` 两类锚点）。
  if (url.pathname === '/search') {
    const body = JSON.parse(rawBody || '{}');
    res.writeHead(200, { 'content-type': 'application/json', ...noKeepAlive });
    return res.end(
      JSON.stringify({
        results: [
          { title: `tavily 首条:${body.query}`, url: 'https://mock.example/first', content: `key=${body.api_key}` },
          { title: 'tavily 次条', url: 'https://mock.example/second', content: '第二条摘要' },
        ],
      }),
    );
  }
  if (url.pathname === '/html/') {
    const q = url.searchParams.get('q') ?? '';
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', ...noKeepAlive });
    return res.end(`<!doctype html><html><body>
      <a class="result__a" href="https://mock.example/ddg1">ddg 首条:${q}</a>
      <a class="result__snippet">ddg 首条摘要</a>
      <a class="result__a" href="https://mock.example/ddg2">ddg 次条</a>
      <a class="result__snippet">ddg 次条摘要</a>
    </body></html>`);
  }

  res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8', ...noKeepAlive });
  res.end(`mock-http: no route ${req.method} ${url.pathname}`);
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[mock-http] listening http://127.0.0.1:${PORT} (marker ${MARKER})`);
});

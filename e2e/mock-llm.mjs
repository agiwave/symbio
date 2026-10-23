//! Mock LLM 服务（**四协议线格式**：openai_chat / openai_responses /
//! anthropic_messages / gemini_api）。
//!
//! 用法：
//!   node e2e/mock-llm.mjs --port 18081 --scenarios ./scenarios.json
//!
//! 能力：
//! - `POST /v1/chat/completions` → openai_chat 线格式（SSE）
//! - `POST /v1/responses`        → openai_responses 线格式（SSE，`response.*` 事件）
//! - `POST /v1/messages`         → anthropic_messages 线格式（`event:` 行 + `data:` 行）
//! - `POST /v1/models/{m}:streamGenerateContent` → gemini_api 线格式（**裸 JSON 数组流**）
//! - `GET  /v1/models` 上下文探测（context_probe 走这里，返回 max_model_len）
//! - `GET  /_health`   健康检查
//! - `GET  /_requests` 读回已记录的请求（测试断言用）
//! - `POST /_reset`    清空请求记录与场景游标
//! - `POST /_plan`     **换一份 plan**（场景表 + 游标一起重置）——用例用它表达
//!   「服务恢复了」：坏的那段时间用静态 `status` 注入，恢复时刻换成正常应答场景。
//!   有了这个入口，"失败要停在哪一次"就不必再靠请求体指纹的名额去猜（见下「逻辑请求」）。
//!
//! ## 四协议的分派与「同一份场景」
//!
//! 路径决定线格式（[`resolveFormat`]），**场景定义（`toolCalls` / `chunks` /
//! `content`）只有一份**——由各格式的 frames 构造器翻译成各自的线上形状。
//! 于是「同一句用户发言 + 同一次工具调用，换协议后链路还通不通」可以用**一条
//! 用例按参数展开**来问（`t66-multi-protocol`），而不是复制四份场景表。
//!
//! 四格式的**请求体形状也各不相同**（工具结果轮的判定尤其如此）：openai_chat 末条
//! `role: "tool"`、openai_responses `input[]` 里末项 `function_call_output`、
//! anthropic 末条 user 的 content 含 `tool_result`、gemini `contents[]` 末项 parts
//! 含 `functionResponse`。[`readTurn`] 把这件事收成一个**格式无关**的判定——否则
//! 某个协议下 `afterTool` 永不命中，症状是**无限工具循环**（撞用例超时，不是断言红）。
//!
//! 故障注入与流控制（`dropAfter` / `stallAfter` / `malformed`）**四格式共用一套**：
//! 它们是「网络层」的事，与线格式无关；各写一份必然漂移。
//!
//! **只说不流式**：四个协议实现全部硬编码 `"stream": true`，不存在非流式分支
//! （gemini 的 `streamGenerateContent` 本身就是流式端点）。
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
//! 不带 `match` 的场景可设 `once: true`（只用一次，适合「第 N 轮调工具」的编排）
//! 或 `times: N`（**最多被选中 N 次**，`once` 就是 `times: 1` 的别名）。
//! `finishReason`：**原样覆盖**终止帧里的结束原因（缺省按有没有工具调用推 `stop` /
//! `tool_calls`）。写的是该协议的**原始值**——`length`（openai 系）/ `max_tokens`
//! （anthropic）/ `MAX_TOKENS`（gemini），后端协议层归一化成同一个 `ModelFinishReason::Length`。
//! 存在它是为了能表达「被 `max_tokens` 掐断」这一格：真实模型会说它没说完，而 mock 照例
//! 只说 stop 就永远复现不了截断（ 的判据要的正是这一格）。
//! `times` 是为**一长串连续工具轮**准备的：`{ afterTool: true, times: 24, toolCalls: [...] }`
//! 让同一轮里连调 24 次工具，第 25 次选中时它已耗尽，池子里下一条（通常就是收束那条）接上。
//! 名额按**选中次数**计，与 `failTimes`（按**逻辑请求**计）是两套计数器，别混。
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
//!
//! ## 异常注入：网络环境与流级故障
//!
//! HTTP 层之上，真实 LLM 服务 / 网络还有一整套**流级与级别的异常**，用例据此
//! 钉「被测系统在真实故障下按纪律收敛」——机制都是场景字段，配在场景上：
//!
//! | 字段 | 语义 |
//! |---|---|
//! | `latencyMs` | 首字节延迟（写响应头前停顿）；模拟真实 provider 的首 token 延迟 |
//! | `chunkJitterMs` | 每帧延迟叠加 `0..N` 随机抖动（真网络不是恒定 `chunkDelayMs`） |
//! | `stallAfter` + `stallMs` | 第 N 个事件后**停顿一次**再接着吐（不关连接）；小于被测系统的流空闲超时才验「容忍停顿」 |
//! | `dropAfter` | 第 N 个事件后**半途断开连接**：不写终止帧、不写 `[DONE]`；`0` = 只发响应头就断 |
//! | `malformed` | 第 N 个事件后混入一帧**不规范内容**再继续正常收尾：`badJson`（data 行坏 JSON）/ `truncated`（chunk JSON 半途截断）/ `garbage`（非 SSE 字节） |
//! | `malformedAfter` | `malformed` 的位次（默认中间）；位次按正常事件计，malformed 帧不占位 |
//!
//! plan 级 `rateLimit: { limit, windowMs }` 造 **429 限流语义**（不是别的场景字段，
//! 因为限流是**整个账号**的，不是单条回答的）：滑动窗口配额，超额请求拿 429 +
//! `retry-after`（窗口冷却秒数）+ `x-ratelimit-*` 头；被拒请求不占配额（与主流provider 一致），窗口冷却后自动恢复。四个流级字段都**改变不了匹配/名额口径**——
//! 接入同一套场景选择规则，正常形态逐字节不变。

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
/**
 * 场景已消耗次数（`once` ≡ `times: 1`，`times: N` ≡ 至多 N 次）。
 *
 * 计的是**选中次数**（一次 HTTP 命中算一次），与 `failTimes` 的「逻辑请求」口径
 * **故意不同**：`failTimes` 要判的是「这类请求在这段时间是坏的」，重发必须沿用同一结论；
 * 而 `times` 是**编排名额**——它存在的意义就是「这条场景最多再被吐 N 次」，用来给工具
 * 循环收尾。按请求体指纹去重的话，静态参数的工具循环（每轮读同一份文件 ⇒ 滑窗内的
 * 消息数组逐字节相同 ⇒ 指纹相同）会一轮名额都不消耗，症状是**无限工具循环**跑到
 * 几千次请求后撞用例超时。
 */
const useCounts = new Map();
// `failTimes` 的名额按**逻辑请求**消耗：键 = 场景 id + 请求体指纹，值 = 该请求
// 定下的结论（fail / ok）。同一请求体的退避重放沿用首次结论（见文件头说明）。
const failOutcome = new Map();
const failLogicalHits = new Map(); // 场景 id → 已消耗的逻辑名额

/**
 * plan 级 `rateLimit`（429 限流语义）的滑动窗口：已获准请求的时间戳队列。
 * key = 整个 mock 实例（限流是账号级的，不按场景分）；被拒请求不占配额。
 */
const granted = [];

/**
 * 路径 → 线格式 id（与 model 插件的 `MODEL_PROTOCOL_*` 同词）。
 *
 * 四条路径各由对应协议的 `get_api_url` 拼出，互不重叠：
 * - openai_chat → `{api_base}/chat/completions`
 * - openai_responses → `{api_base}/responses`
 * - anthropic_messages → `{api_base}/messages`
 * - gemini_api → `{api_base}/models/{model}:streamGenerateContent`（**无 query**，
 *   key 走 `x-goog-api-key` 头——`gemini_api.rs::get_api_url` 只拼到动作名）
 */
function resolveFormat(pathname) {
  if (pathname.endsWith('/chat/completions')) return 'openai_chat';
  if (pathname.endsWith('/responses')) return 'openai_responses';
  if (pathname.endsWith('/messages')) return 'anthropic_messages';
  if (pathname.includes(':streamGenerateContent')) return 'gemini_api';
  return null;
}

/**
 * 从请求体读出**格式无关**的两件事：本轮用户文本、是否工具结果轮。
 *
 * ## 为什么必须按格式判
 *
 * 四协议的请求体形状各不相同，工具结果的承载位置尤其如此：
 *
 * | 协议 | 用户文本 | 工具结果轮 |
 * |---|---|---|
 * | `openai_chat` | `messages[]` 末条 role=user 的 content | 末条 `role: "tool"` |
 * | `openai_responses` | `input[]` 末个 `type:"message",role:"user"` 的 `content[].text` | 末项 `type: "function_call_output"` |
 * | `anthropic_messages` | `messages[]` 末条 role=user 的 `content[].text` | 末条 role=user 的 `content[]` 含 `type: "tool_result"` |
 * | `gemini_api` | `contents[]` 末条 role=user 的 `parts[].text` | 末项 `parts[]` 含 `functionResponse` |
 *
 * 只认 `messages[].role === 'tool'` 的老写法（openai_chat 一家的形状）在其余三个
 * 协议下**恒判 false** ⇒ `afterTool` 场景永不命中 ⇒ 工具结果轮落到普通池 ⇒ 无限
 * 工具循环。症状是用例撞超时而不是断言红，离真因隔很远。
 *
 * @returns {{ userText: string, isToolResultTurn: boolean }}
 */
function readTurn(format, body) {
  const arr = (x) => (Array.isArray(x) ? x : []);
  if (format === 'openai_responses') {
    const items = arr(body.input);
    const lastUser = [...items].reverse().find((it) => it.type === 'message' && it.role === 'user');
    const userText = arr(lastUser?.content)
      .filter((c) => c.type === 'input_text')
      .map((c) => c.text ?? '')
      .join('');
    return { userText, isToolResultTurn: items[items.length - 1]?.type === 'function_call_output' };
  }
  if (format === 'gemini_api') {
    const contents = arr(body.contents);
    const lastUser = [...contents].reverse().find((c) => c.role === 'user');
    const userText = arr(lastUser?.parts)
      .filter((p) => typeof p.text === 'string')
      .map((p) => p.text)
      .join('');
    const last = contents[contents.length - 1];
    return { userText, isToolResultTurn: arr(last?.parts).some((p) => p.functionResponse != null) };
  }
  const msgs = arr(body.messages);
  const last = msgs[msgs.length - 1];
  if (format === 'anthropic_messages') {
    const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
    const userText = arr(lastUser?.content)
      .filter((c) => c.type === 'text')
      .map((c) => c.text ?? '')
      .join('');
    const isToolResultTurn =
      last?.role === 'user' && arr(last.content).some((c) => c.type === 'tool_result');
    return { userText, isToolResultTurn };
  }
  // openai_chat（也是缺省）：末条 role=user 的 content；工具结果轮 = 末条 role=tool。
  const lastUser = [...msgs].reverse().find((m) => m.role === 'user');
  const userText =
    typeof lastUser?.content === 'string'
      ? lastUser.content
      : arr(lastUser?.content).map((p) => p.text ?? '').join('');
  return { userText, isToolResultTurn: last?.role === 'tool' };
}

/** 逻辑请求指纹：场景 id + 请求体（重发复用同一份字节 ⇒ 指纹相同）。 */
function requestFingerprint(scenarioId, body) {
  return `${scenarioId}\n${JSON.stringify(body)}`;
}

function loadScenarios() {
  return plan.scenarios ?? [];
}

/** 本场景还可被选中几次（`once` = 1，`times: N` = N，都没写 = 不限）。 */
function usageCap(scenario) {
  if (scenario.times != null) return scenario.times;
  return scenario.once ? 1 : Infinity;
}

function pickScenario(format, body) {
  // 用户文本与「是否工具结果轮」都**按格式**读（四协议承载形状不同，见 `readTurn`）。
  // 误判的两端都栽过：判窄 ⇒ `afterTool` 永不命中 ⇒ 无限工具循环（撞超时，不是
  // 断言红）；判宽 ⇒ 新一轮的第一次请求被误认成工具结果轮 ⇒ 根本不请求工具。
  const { userText, isToolResultTurn } = readTurn(format, body);

  // ⚠️ `afterTool` 是**优先**而不是**硬过滤**。
  //
  // 硬过滤会把「没声明任何 `afterTool` 场景」的用例在工具结果轮上清空池子 ⇒ 直接落
  // `plan.fallback`，而它要的普通场景本该在工具结果轮继续可用（症状落在下游的历史断言上，
  // 离真因隔两层）。所以下面「优先池选不出就退回全池」——回退同时兜住了「带 `match` 的
  // 工具场景反复命中 → 无限工具循环」。
  const all = loadScenarios();
  const pool = isToolResultTurn
    ? all.filter((s) => s.afterTool)
    : all.filter((s) => !s.afterTool);

  // ⚠️ **取位置最靠后的那个匹配，不是第一个匹配的**。
  //
  // 场景表里可能有多个 `match` 都命中 `userText`——取最靠后的才让「后一轮的故障注入」
  // 不被前一轮的普通场景抢走（否则故障静默不发生、那一轮假成功、CLI 退出 0，而症状出现在
  // **被测系统之外**，极易误诊成产品缺陷）。
  //
  // `match` 缺失的规则不参与位置比较（位置对它没有意义），只在还没有任何位置命中时兜底。
  const choose = (candidates) => {
    let best = null;
    let bestAt = -1;
    for (const s of candidates) {
      if (usageCap(s) <= (useCounts.get(s.id) ?? 0)) continue;
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
    // 名额按**选中次数**扣（每 HTTP 命中一次；口径与 `failTimes` 不同，理由见 `useCounts`）。
    let useIndex = 0;
    if (Number.isFinite(usageCap(best))) {
      useCounts.set(best.id, (useCounts.get(best.id) ?? 0) + 1);
      useIndex = useCounts.get(best.id);
    }
    return { scenario: best, userText, useIndex };
  }
  return { scenario: plan.fallback ?? { id: 'default', content: '（mock 默认回复）' }, userText, useIndex: 0 };
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

/**
 * 把场景编译成 [delta, finish] 事件序列（SSE 流式唯一形态）。
 *
 * `toolCallId` 对 **`times` 复用型场景**加选中次数后缀：一条场景被连选 N 次就是模型
 * 连发 N 次工具调用，id 相同的话后端按 id 归并 ⇒ 转写里只剩一格、请求视图里只剩一个
 * 工具结果（症状：长轮的字节曲线是一条水平线，「45 步」压不出任何累积）。
 * 其余场景（`once` / 无限）逐字节保持原样——用例里写死的 `call_1` 之类不该被改动。
 */
function buildToolCallId(scenario, tc, i, useIndex) {
  const base = tc.id ?? `call_${i}`;
  return scenario.times != null && useIndex > 0 ? `${base}#${useIndex}` : base;
}

function* buildEvents(scenario, model, requestId, useIndex = 0) {
  // ⓪ 思考（reasoning_content，真实模型在正文之前输出）
  for (const piece of scenario.reasoning ?? []) {
    yield chunkOf(requestId, model, { reasoning_content: piece });
  }
  // ① 工具调用（先于正文，与真实模型行为一致）
  for (const [i, tc] of (scenario.toolCalls ?? []).entries()) {
    const argsStr = typeof tc.arguments === 'string' ? tc.arguments : JSON.stringify(tc.arguments ?? {});
    yield chunkOf(requestId, model, {
      tool_calls: [
        { index: i, id: buildToolCallId(scenario, tc, i, useIndex), type: 'function', function: { name: tc.name, arguments: '' } },
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
  const finish = finishFor(scenario, ['tool_calls', 'stop']);
  yield chunkOf(requestId, model, {}, finish, { prompt_tokens: 64, completion_tokens: 32 });
}

/**
 * 不规范 SSE 帧（`malformed` 场景字段）：真实 provider 偶尔吐出的脏帧形态。
 * 全部以完整行写出（后端按行切分：`'\n'` 即成行），否则只进「未结束行」缓冲。
 */
function malformedFrame(kind) {
  if (kind === 'badJson') return 'data: {not-json,这不是合法的JSON}\n\n';
  if (kind === 'truncated') {
    return 'data: {"id":"chatcmpl-truncated","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"con\n\n';
  }
  // 'garbage' 与缺省：非 SSE 字节（无 data: 前缀的杂散行）
  return 'mock-llm keepalive: 这一行不是 SSE 帧\n\n';
}

// ---------- 四协议 frames：把同一份场景翻译成各自的线上字节 ----------
//
// frames 的每一项是「一段要写出的原始字节」（可含多行）；流循环逐项写出，故障注入
// 的位次锚点即 frame 下标。`terminator` 是流末尾单独写的收尾字节（openai 系的
// `data: [DONE]`），`dropAfter === frames.length` 的语义因此仍是「全部数据帧之后、
// 收尾帧之前断开」——与改造前 openai_chat 的 `events` / `[DONE]` 口径逐字一致。

/** SSE 数据帧：`data: <json>\n\n`（openai 系两协议共用）。 */
const SSE = (obj) => `data: ${JSON.stringify(obj)}\n\n`;

/** 场景的正文分片（`chunks` 优先，退 `content` 单片）。 */
function contentChunks(scenario) {
  return scenario.chunks ?? (scenario.content != null ? [scenario.content] : []);
}

/**
 * 场景的结束原因：`finishReason` **原样覆盖**，否则按「有没有工具调用」推缺省值。
 * 参数 `[有工具的缺省, 没工具的缺省]` 是**本协议的原始写法**（四协议各不同），
 * 覆盖值同样按协议写——后端 `ModelFinishReason::from_provider` 认全部变体。
 */
function finishFor(scenario, [withTools, plain]) {
  return scenario.finishReason ?? ((scenario.toolCalls ?? []).length > 0 ? withTools : plain);
}

/** 工具参数序列化（对象 → JSON 串；已是字符串则原样）。 */
function argsString(tc) {
  return typeof tc.arguments === 'string' ? tc.arguments : JSON.stringify(tc.arguments ?? {});
}

/** openai_chat：复用 `buildEvents` 的 chunk 形状，序列化为 SSE 数据帧。 */
function openaiChatFrames(scenario, model, requestId, useIndex) {
  return [...buildEvents(scenario, model, requestId, useIndex)].map(SSE);
}

/**
 * openai_responses：`response.*` 事件。
 *
 * 工具名 / call_id 由 `response.output_item.added` 给出（后端读 `item.name` /
 * `item.call_id`），参数由 `response.function_call_arguments.delta` 分两片流式吐；
 * 结束原因与用量在 `response.completed` 的 `response` 对象里（`parse_line` 从那里读）。
 */
function openaiResponsesFrames(scenario, model, requestId, useIndex) {
  const frames = [];
  for (const [i, tc] of (scenario.toolCalls ?? []).entries()) {
    const callId = buildToolCallId(scenario, tc, i, useIndex);
    frames.push(
      SSE({
        type: 'response.output_item.added',
        output_index: i,
        item: { type: 'function_call', call_id: callId, name: tc.name, arguments: '' },
      }),
    );
    const a = argsString(tc);
    const mid = Math.max(1, Math.floor(a.length / 2));
    frames.push(SSE({ type: 'response.function_call_arguments.delta', output_index: i, call_id: callId, delta: a.slice(0, mid) }));
    frames.push(SSE({ type: 'response.function_call_arguments.delta', output_index: i, call_id: callId, delta: a.slice(mid) }));
  }
  for (const piece of contentChunks(scenario)) {
    frames.push(SSE({ type: 'response.output_text.delta', delta: piece }));
  }
  const finish = finishFor(scenario, ['tool_calls', 'stop']);
  frames.push(
    SSE({
      type: 'response.completed',
      response: {
        id: `resp_${requestId}`,
        finish_reason: finish,
        usage: { input_tokens: 64, output_tokens: 32 },
      },
    }),
  );
  return { frames, terminator: 'data: [DONE]\n\n' };
}

/**
 * anthropic_messages：`event:` 行 + `data:` 行。
 *
 * 工具名 / id 由 `content_block_start`（`content_block.type == "tool_use"`）给出，
 * 参数由 `content_block_delta`（`delta.type == "input_json_delta"`）的 `partial_json`
 * 分两片流式吐；正文是 `text_delta`。结束原因是 `message_delta.delta.stop_reason`，
 * `input_tokens` 在 `message_start`、`output_tokens` 在 `message_delta`（两处合并）。
 */
function anthropicFrames(scenario, model, requestId, useIndex) {
  const frames = [];
  const ev = (type, obj) => `event: ${type}\ndata: ${JSON.stringify(obj)}\n\n`;
  frames.push(
    ev('message_start', {
      type: 'message_start',
      message: { id: `msg_${requestId}`, usage: { input_tokens: 64 } },
    }),
  );
  const toolCount = (scenario.toolCalls ?? []).length;
  for (const [i, tc] of (scenario.toolCalls ?? []).entries()) {
    const callId = buildToolCallId(scenario, tc, i, useIndex);
    frames.push(
      ev('content_block_start', {
        type: 'content_block_start',
        index: i,
        content_block: { type: 'tool_use', id: callId, name: tc.name },
      }),
    );
    const a = argsString(tc);
    const mid = Math.max(1, Math.floor(a.length / 2));
    frames.push(ev('content_block_delta', { type: 'content_block_delta', index: i, delta: { type: 'input_json_delta', partial_json: a.slice(0, mid) } }));
    frames.push(ev('content_block_delta', { type: 'content_block_delta', index: i, delta: { type: 'input_json_delta', partial_json: a.slice(mid) } }));
  }
  // 正文块用工具块之后的 index（Anthropic 的块下标连续）。
  const textIndex = toolCount;
  for (const piece of contentChunks(scenario)) {
    frames.push(ev('content_block_delta', { type: 'content_block_delta', index: textIndex, delta: { type: 'text_delta', text: piece } }));
  }
  const stop = finishFor(scenario, ['tool_use', 'end_turn']);
  frames.push(
    ev('message_delta', {
      type: 'message_delta',
      delta: { stop_reason: stop },
      usage: { output_tokens: 32 },
    }),
  );
  return { frames, terminator: 'event: message_stop\ndata: {"type":"message_stop"}\n\n' };
}

/**
 * gemini_api：**裸 JSON 数组流**（`[\n{chunk}\n{chunk}\n]\n`），不是 SSE。
 *
 * `gemini_api.rs::parse_line` 直接对整行（trim 后）做 JSON 解析、并显式跳过 `[` /
 * `]` / `,` 行——它消费的是 `streamGenerateContent` 的数组包裹格式，**没有**
 * `data:` 前缀。每帧独占一行（完整 JSON，无尾随逗号），流循环按 `\n` 切行后逐行解析。
 *
 * `functionCall.args` 每次给**完整对象**（Gemini 无「参数在增长」这回事），一帧一个。
 */
function geminiFrames(scenario, model, requestId, useIndex) {
  const frames = ['[\n'];
  for (const tc of scenario.toolCalls ?? []) {
    const args = typeof tc.arguments === 'string' ? JSON.parse(tc.arguments) : (tc.arguments ?? {});
    frames.push(
      JSON.stringify({
        candidates: [{ content: { role: 'model', parts: [{ functionCall: { name: tc.name, args } }] } }],
      }) + '\n',
    );
  }
  for (const piece of contentChunks(scenario)) {
    frames.push(
      JSON.stringify({ candidates: [{ content: { role: 'model', parts: [{ text: piece }] } }] }) + '\n',
    );
  }
  frames.push(
    JSON.stringify({
      candidates: [{ content: { role: 'model', parts: [] }, finishReason: scenario.finishReason ?? 'STOP' }],
      usageMetadata: { promptTokenCount: 64, candidatesTokenCount: 32 },
    }) + '\n',
  );
  frames.push(']\n');
  return { frames, terminator: null };
}

/** 按线格式分派：返回 `{ frames, terminator }`。 */
function buildFrames(format, scenario, model, requestId, useIndex) {
  switch (format) {
    case 'openai_responses':
      return openaiResponsesFrames(scenario, model, requestId, useIndex);
    case 'anthropic_messages':
      return anthropicFrames(scenario, model, requestId, useIndex);
    case 'gemini_api':
      return geminiFrames(scenario, model, requestId, useIndex);
    default:
      return { frames: openaiChatFrames(scenario, model, requestId, useIndex), terminator: 'data: [DONE]\n\n' };
  }
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
    useCounts.clear();
    failOutcome.clear();
    failLogicalHits.clear();
    granted.length = 0;
    res.writeHead(200, noKeepAlive);
    return res.end(JSON.stringify({ ok: true }));
  }
  // 换 plan：场景表整份替换 + 游标清零。用例用它表达「服务恢复了」——
  // 故障时段用静态 `status` 注入（每次请求都坏），恢复时刻换成正常应答场景。
  // 计数器必须一起清：残留的 `useCounts` 会让新场景表里的 `times` 一上来就耗尽。
  if (url.pathname === '/_plan' && req.method === 'POST') {
    const parts = [];
    for await (const part of req) parts.push(part);
    let next = null;
    try {
      next = JSON.parse(Buffer.concat(parts).toString('utf8'));
    } catch {
      res.writeHead(400, noKeepAlive);
      return res.end(JSON.stringify({ error: { message: 'mock-llm: /_plan 需要合法 JSON plan' } }));
    }
    if (!Array.isArray(next?.scenarios)) {
      res.writeHead(400, noKeepAlive);
      return res.end(JSON.stringify({ error: { message: 'mock-llm: /_plan 缺 scenarios 数组' } }));
    }
    plan = { scenarios: next.scenarios, fallback: next.fallback ?? null, ...(next.rateLimit ? { rateLimit: next.rateLimit } : {}) };
    useCounts.clear();
    failOutcome.clear();
    failLogicalHits.clear();
    granted.length = 0;
    res.writeHead(200, noKeepAlive);
    return res.end(JSON.stringify({ ok: true, scenarios: plan.scenarios.length }));
  }

  // 上下文探测：GET /v1/models → data[].max_model_len
  if (url.pathname === '/v1/models') {
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ data: [{ id: 'mock-model', max_model_len: 262144 }] }));
  }

  // 四协议各占一条路径（见 `resolveFormat`）；命中即按该线格式应答。
  const format = resolveFormat(url.pathname);
  if (format) {
    // 请求体必须**先攒 Buffer 再整体解码**（`body += part` 是逐块解码：一个 3 字节
    // 中文字符被 socket 分块切在中间时，两半各解出一个 U+FFFD —— 同一份字节的重发
    // 会因分块边界不同而解出**差 1 个字符**的请求体 ⇒ 指纹分裂 ⇒ failTimes 名额与
    // 逻辑请求数被虚增，e2e 按「逻辑请求」计数的用例随之偶发飘红）。
    const parts = [];
    for await (const part of req) parts.push(part);
    const body = Buffer.concat(parts).toString('utf8');
    let parsed = {};
    try { parsed = JSON.parse(body); } catch { /* 容错：空体 */ }
    requests.push({ at: Date.now(), path: url.pathname, format, body: parsed, contentType: req.headers['content-type'] ?? null, status: null });
    const rec = requests[requests.length - 1];

    // 429 限流语义（plan 级 `rateLimit: { limit, windowMs }`）：滑动窗口配额，
    // 超额拿 429 + retry-after（窗口冷却秒）+ x-ratelimit-*；被拒不占配额（与
    // 主流 provider 一致），窗口冷却后自动恢复。限流先于场景匹配判（限流与回答内容无关）。
    const quota = plan.rateLimit;
    if (quota && Number.isFinite(quota.limit) && Number.isFinite(quota.windowMs)) {
      const now = Date.now();
      while (granted.length && now - granted[0] >= quota.windowMs) granted.shift();
      if (granted.length >= quota.limit) {
        const waitSec = Math.max(1, Math.ceil((granted[0] + quota.windowMs - now) / 1000));
        rec.status = 429;
        if (process.env.E2E_DEBUG) console.error(`[mock-llm] 429 限流注入（配额 ${quota.limit}/${quota.windowMs}ms，冷却 ${waitSec}s）`);
        res.writeHead(429, {
          'content-type': 'application/json',
          connection: 'close',
          'retry-after': String(waitSec),
          'x-ratelimit-limit': String(quota.limit),
          'x-ratelimit-remaining': '0',
          'x-ratelimit-reset': String(waitSec),
        });
        return res.end(JSON.stringify({
          error: {
            message: `mock 注入的 429 限流（配额 ${quota.limit}/${quota.windowMs}ms 内超额，${waitSec}s 后冷却）`,
            type: 'rate_limited',
            code: 'rate_limit_exceeded',
          },
        }));
      }
      granted.push(now);
    }

    const { scenario, useIndex } = pickScenario(format, parsed);
    const model = parsed.model ?? 'mock-model';
    const requestId = `chatcmpl-${Date.now().toString(16)}-${requests.length}`;

    // 故障注入：非 2xx（静态）
    if (scenario.status && scenario.status >= 400) {
      rec.status = scenario.status;
      res.writeHead(scenario.status, errorHeaders(scenario));
      return res.end(JSON.stringify({ error: { message: scenario.error ?? 'mock 注入的错误', type: 'mock_error' } }));
    }

    // 故障注入：前 `failTimes` 次**逻辑请求**失败，之后同场景正常应答——
    // 支撑「连续失败 → 熗断 → 重试成功」这类**按时间变化**的编排（静态 status 做不到）。
    // 名额按请求体指纹消耗、同一请求体的重放沿用首次结论：见文件头「逻辑请求」。
    if (scenario.failTimes > 0) {
      const verdict = failVerdict(scenario, parsed);
      if (verdict.fail) {
        rec.status = scenario.errorStatus ?? 500;
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

    const { frames, terminator } = buildFrames(format, scenario, model, requestId, useIndex);

    // ⚠️ 本 mock **只说流式**（与生产一致）：四个协议实现全部硬编码 `"stream": true`
    // （gemini 的 `streamGenerateContent` 本身就是流式端点），不存在非流式补全请求。
    // 曾经有个 `stream === false` 的一次性 JSON 分支，因无任何调用方（纯防御性死代码、
    // 误导「存在非流式链路」的读者）已删除。e2e 的信息收集同样只有两条流式通道：
    // gateway WS 实时帧 + 生产历史读出口（`readSessionMessages`）。

    // 异常注入的位次锚点（对称于 frames 的下标；收尾帧不占位）——
    // `dropAfter: 0` = 只发响应头就断；`dropAfter: frames.length` = 全部数据帧已吐完
    // 但收尾帧未写；`stallAfter` 停顿不关连接；`malformed` 帧不占位。语义见文件头。
    const malformedAt = scenario.malformed != null ? (scenario.malformedAfter ?? Math.floor(frames.length / 2)) : -1;
    const dropAt = scenario.dropAfter != null ? scenario.dropAfter : -1;
    const stallAt = scenario.stallAfter != null ? scenario.stallAfter : -1;

    if (scenario.latencyMs) await sleep(scenario.latencyMs);
    rec.status = 200;
    res.writeHead(200, {
      // gemini 的 `streamGenerateContent` 回的是裸 JSON 数组流（非 SSE）；其余三协议是
      // text/event-stream。
      'content-type': format === 'gemini_api' ? 'application/json' : 'text/event-stream',
      'cache-control': 'no-cache',
      connection: 'keep-alive',
    });
    const delay = scenario.chunkDelayMs ?? 0;
    const jitter = scenario.chunkJitterMs ?? 0;
    for (const [i, frame] of frames.entries()) {
      if (dropAt === i) {
        // 半途断开：不写收尾帧，socket 直接 destroy——
        // 被测系统视角 = 「流还在但数据表里最后一帧之后突然没了」
        if (process.env.E2E_DEBUG) console.error(`[mock-llm] 流中断注入（第 ${i} 个事件后 destroy）`);
        res.socket.destroy();
        return;
      }
      if (stallAt === i) {
        // 中期停顿：连接离著，数据停 `stallMs`（小于被测系统的流空闲超时才验「容忍停顿」）
        if (process.env.E2E_DEBUG) console.error(`[mock-llm] 流停顿注入（第 ${i} 个事件后停住 ${scenario.stallMs}ms）`);
        await sleep(scenario.stallMs ?? 1000);
      }
      if (malformedAt === i) res.write(malformedFrame(scenario.malformed));
      res.write(frame);
      if (delay || jitter) await sleep(delay + (jitter ? Math.random() * jitter : 0));
    }
    if (dropAt === frames.length) {
      if (process.env.E2E_DEBUG) console.error('[mock-llm] 流中断注入（全部事件后、收尾帧前 destroy）');
      res.socket.destroy();
      return;
    }
    if (terminator) res.write(terminator);
    return res.end();
  }

  res.writeHead(404, { 'content-type': 'application/json' });
  res.end(JSON.stringify({ error: { message: `mock-llm: no route ${req.method} ${url.pathname}` } }));
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[mock-llm] listening http://127.0.0.1:${PORT} (scenarios: ${loadScenarios().length})`);
});

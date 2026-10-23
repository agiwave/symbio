// LLM 调用契约（MockLLM 探测面）——「用户请求 → 系统是否正确调用模型」的系统化判据。
//
// ## 这个模块是什么
//
// 既有 e2e 的判据落在**响应用户**一侧（退出码 / stdout / 转写 / 实时面）；本模块把
// 判据延伸到**调用模型**一侧：mock-llm 的 `/_requests` 是「模型眼里的系统」——每一次
// 请求体就是「系统怎么理解用户请求」的完整投影。调用错了（信封丢字段、system 少一段、
// 历史丢一轮、工具结果不配对、工具面漂移），响应侧可能照样绿，而没有任何东西会红。
//
// 与既有判据的分工（一个事实只有一个 owner，见 ）：
// - 本模块判**语义正确性**（值对不对、段全不全、序对不对）；
// - `scripts/prompt-shape-audit.mjs`（PS-001…003）判出厂主线的**结构形状**；
// - t21/t22/t23/t29/t35/t41 判对话面 / 记忆 / 技能的**行为**。
//
// ## 使用方式
//
// ```js
// import { assertEnvelope, FACTORY_TOOLS, ... } from '../llm-contract.mjs';
// assertEnvelope(reqs[0], { model: 'mock-model', temperature: 0.1, max_tokens: 1024 }, 'T56');
// ```
//
// 每条断言失败都带**有诊断价值的**上下文（实际序列 / 实际集合），不写「与预期不符」。
// 命名编号 LC-001…：新增一条契约 = 加一个编号 + 一段「拦哪一种失效」的说明，
// 编号即契约的引用锚（与 PS-001 同形）。

import { PLATFORM_VARIANT_TOOL, PLATFORM_VARIANT_TOOL_NAMES } from '../scripts/prompt-shape-audit.mjs';

// ---------- 基础提取 ----------

/** 一条消息的正文（string / 分片数组两种形态都认；其余为空串）。 */
export function textOf(m) {
  if (m == null) return '';
  if (typeof m.content === 'string') return m.content;
  if (Array.isArray(m.content)) return m.content.map((p) => p?.text ?? '').join('');
  return '';
}

/** 请求的消息数组（缺省空数组——调用方拿到的永远是可迭代物）。 */
export function messagesOf(req) {
  return req?.body?.messages ?? [];
}

/**
 * 合并后的**唯一** system 消息（出厂只发一条：基础 system 与请求级前缀段由
 * `merge_leading_system` 合成；协议实现见 `openai_chat.rs`）。
 *
 * 返回 null = 这条请求没有 system 消息（协议/装配坏了的形态之一）。
 */
export function systemMessageOf(req) {
  const sys = messagesOf(req).filter((m) => m.role === 'system');
  return sys.length === 1 ? sys[0] : sys[0] ?? null;
}

/** system 消息正文（无 system 消息 ⇒ 空串）。 */
export function systemTextOf(req) {
  const m = systemMessageOf(req);
  return m ? textOf(m) : '';
}

/** 对话消息（user / assistant / tool）——system 是装配层，不属于对话流。 */
export function dialogOf(req) {
  return messagesOf(req).filter((m) => ['user', 'assistant', 'tool'].includes(m.role));
}

/** 对话消息的角色序列（如 `user,assistant,assistant,tool,user`）。 */
export function roleSeqOf(req) {
  return dialogOf(req).map((m) => m.role);
}

/** 信封：装配代码 / provider 配置决定的那几个键。 */
export function envelopeOf(req) {
  const b = req?.body ?? {};
  return {
    model: b.model ?? null,
    stream: b.stream ?? null,
    temperature: b.temperature ?? null,
    max_tokens: b.max_tokens ?? null,
    tool_choice: b.tool_choice ?? null,
    hasTools: Array.isArray(b.tools) && b.tools.length > 0,
  };
}

/** 请求的工具面（wire 名列表，保序；shell 平台变体已归一成 `<shell>`）。 */
export function toolNamesOf(req) {
  return (req?.body?.tools ?? [])
    .map((t) => t?.function?.name)
    .filter(Boolean)
    .map(normalizeShell);
}

/** wire 名里的 shell 平台变体归一（与 prompt-shape-audit 同一名单，不抄第二份规则）。 */
function normalizeShell(name) {
  return PLATFORM_VARIANT_TOOL_NAMES.includes(name) ? PLATFORM_VARIANT_TOOL : name;
}

/** 请求里全部 assistant 消息携带的 tool_calls 条目（拉平）。 */
export function toolCallsOf(req) {
  return dialogOf(req).flatMap((m) => m.tool_calls ?? []);
}

/** 请求里全部 role=tool 结果消息。 */
export function toolResultsOf(req) {
  return dialogOf(req).filter((m) => m.role === 'tool');
}

/** 末条 user 消息（mock 场景匹配读的就是它；置顶注入不得顶掉它）。 */
export function lastUserOf(req) {
  return [...messagesOf(req)].reverse().find((m) => m.role === 'user') ?? null;
}

// ---------- 出厂工具面 ----------

/**
 * 出厂内置工具的 **wire 名**集合（默认 homedir：无技能、无 MCP、无子智能体）。
 *
 * 来源：2026-10-14 真实链路探针（CLI × mock-llm 跑一轮，读 `/_requests` 的
 * `tools[].function.name`），与 `docs/004-ref/006-当前事实表.md` §2 逐条对账（ToolSpec 表模式
 * 补抽取后一致——那次修复正是本契约与文档对账发现的，见该契约的生成说明）。
 *
 * `cmd`/`sh` 归一成 `<shell>`：shell 族工具名按 target_os 取，与
 * `prompt-shape-audit` 同一记号（PLATFORM_VARIANT_TOOL 是唯一真相，不在这里
 * 抄第二份名单）。
 *
 * ⚠️ 与 CURRENT.md 的同步是**契约的一部分**：加一个出厂工具要同时改三处
 * （Rust 注册点 / CURRENT.md 生成物 / 本常量），漏一处就会在 t60 红——
 * 那正是这道判据存在的意义。
 */
export const FACTORY_TOOLS = Object.freeze([
  'agent_run',
  'ask_user',
  'codebase_search',
  'content_search',
  'todo_write',
  'heartbeat',
  'vdfs_list',
  'vdfs_tree',
  'vdfs_stat',
  'vdfs_read',
  'vdfs_edit',
  'vdfs_search',
  'vdfs_write',
  'vdfs_delete',
  'vdfs_mkdir',
  'web_fetch',
  'web_search',
  'http_request',
  PLATFORM_VARIANT_TOOL,
]);

// ---------- LC-001 信封契约 ----------

/**
 * LC-001 信封逐字段契约：provider 配置怎么配，请求就怎么带。
 *
 * 拦的失效：配了不用（温度/模型静默丢失）、`stream` 漂成非流式（本仓三个协议
 * 实现全部硬编码流式，非流式分支不存在——出现即回归）、工具面与 `tool_choice`
 * 不同步（有工具不给 auto ⇒ 模型不调工具；没工具给 auto ⇒ 部分网关 400）。
 *
 * @param req mock-llm 记录的一条请求（`{ body }`）
 * @param expected `{ model, temperature, max_tokens }`——provider 配置的期望值
 */
export function assertEnvelope(req, expected, label) {
  const env = envelopeOf(req);
  const bad = [];
  if (expected.model != null && env.model !== expected.model) bad.push(`model: ${env.model} ≠ ${expected.model}`);
  if (expected.temperature != null && env.temperature !== expected.temperature) {
    bad.push(`temperature: ${env.temperature} ≠ ${expected.temperature}`);
  }
  if (expected.max_tokens != null && env.max_tokens !== expected.max_tokens) {
    bad.push(`max_tokens: ${env.max_tokens} ≠ ${expected.max_tokens}`);
  }
  if (bad.length) {
    throw new Error(`${label}: 信封与 provider 配置不符（${bad.join('；')}）`);
  }
}

/** LC-001b 只走流式：`stream` 恒为 `true`（协议契约，见 openai_chat.rs 的硬编码）。 */
export function assertStreamOnly(req, label) {
  const env = envelopeOf(req);
  if (env.stream !== true) {
    throw new Error(`${label}: stream 必须恒为 true（实际: ${JSON.stringify(env.stream)}）`);
  }
}

/** LC-001c `tool_choice` 与工具面同步：有工具 ⇒ `auto`；无工具 ⇒ 键不出现。 */
export function assertToolChoiceSync(req, label) {
  const env = envelopeOf(req);
  if (env.hasTools && env.tool_choice !== 'auto') {
    throw new Error(`${label}: 有工具面时 tool_choice 必须为 auto（实际: ${JSON.stringify(env.tool_choice)}）`);
  }
  if (!env.hasTools && env.tool_choice != null) {
    throw new Error(`${label}: 无工具面时不得带 tool_choice（实际: ${JSON.stringify(env.tool_choice)}）`);
  }
}

// ---------- LC-002 system 装配契约 ----------

/**
 * LC-002 system 装配契约：注册段 + 请求级前缀合成**唯一一条** system 消息，
 * 置于消息数组头部，且请求级前缀排在注册段之后。
 *
 * 拦的失效：
 * - 段丢失（人格 / 记忆 / 智能体 AGENTS.md 注册了没送达）——模型「不知道它
 *   存在」，没有任何报错（`resolve_system_prompt` 的模块头点名这种静默面）；
 * - 多条 system（合并回归）——协议层看到两个 system，行为按实现漂移；
 * - 前缀置顶（抢在注册段前面）——「谁先谁后」被插队算法绑定，调一次视图次序
 *   就改掉模型读到的优先级（`request_view_prefix` 的顺序纪律）。
 *
 * @param req 一条请求
 * @param opts.segments 必须出现的段标记文本（如人格首句 / 记忆抬头），按序给出
 * @param opts.prefixMarkers 请求级前缀的标记（如召回抬头）——必须排在所有段标记之后
 */
export function assertSystemAssembly(req, { segments = [], prefixMarkers = [] } = {}, label) {
  const msgs = messagesOf(req);
  const sysCount = msgs.filter((m) => m.role === 'system').length;
  if (sysCount !== 1) {
    throw new Error(`${label}: 必须恰好一条 system 消息（实际 ${sysCount} 条：${JSON.stringify(msgs.map((m) => m.role))}）`);
  }
  if (msgs[0]?.role !== 'system') {
    throw new Error(`${label}: system 消息必须在数组头部（实际首条: ${msgs[0]?.role}）`);
  }
  const text = systemTextOf(req);
  if (!text.trim()) {
    throw new Error(`${label}: system 消息不得为空（显式段与注册段皆空时有硬编码兜底）`);
  }
  // 段标记全部在场
  let lastAt = -1;
  for (const seg of segments) {
    const at = text.indexOf(seg);
    if (at < 0) throw new Error(`${label}: system 缺少段「${seg}」（实际前 200 字符: ${text.slice(0, 200)}）`);
    if (at < lastAt) throw new Error(`${label}: system 段序错乱（「${seg}」排在前一个段之前）`);
    lastAt = at;
  }
  // 前缀标记排在所有段标记之后
  for (const mk of prefixMarkers) {
    const at = text.indexOf(mk);
    if (at < 0) throw new Error(`${label}: system 缺少请求级前缀标记「${mk}」`);
    if (at < lastAt) throw new Error(`${label}: 请求级前缀「${mk}」必须排在注册段之后（置顶会改模型读到的优先级）`);
  }
}

/**
 * LC-002b 末条 user 仍是用户自己的话。
 *
 * 拦的失效：置顶注入（记忆 / 前缀）把「最后一条 user 消息」顶掉——mock 的场景
 * 匹配与轮次窗口都按末条 user 读，顶掉即全线失真（t29 同形判据的契约化）。
 */
export function assertLastUserIs(req, expectedText, label) {
  const last = lastUserOf(req);
  const t = textOf(last);
  if (!last) throw new Error(`${label}: 请求里没有 user 消息`);
  if (!t.includes(expectedText)) {
    throw new Error(`${label}: 末条 user 应含用户原话「${expectedText}」（实际: ${JSON.stringify(t.slice(0, 120))}）`);
  }
}

// ---------- LC-003 多轮历史保真 ----------

/**
 * LC-003 历史保真：第 N 轮请求的对话消息 = 前 N-1 轮的完整重放 + 本轮发言，
 * 无重复、无丢失、序不错。
 *
 * 拦的失效：长轮失忆（与工具结果累进同族——工具结果不累进下一轮请求）、历史错位
 * （两轮发言合成一条 / 丢一条）、把本轮发言排到历史之前。
 *
 * @param reqs 该会话的请求序列（按时间序）
 * @param expected 每轮期望的**角色序列**数组（如 `[['user'], ['user','assistant','assistant','tool','user']]`）
 *   —— 期望序列由用例按场景推导（用例知道每轮模型答了什么、调了什么工具），
 *   本函数只做「实际 vs 期望」的机械比对，不内置任何场景知识。
 */
export function assertHistoryReplay(reqs, expected, label) {
  if (reqs.length !== expected.length) {
    throw new Error(`${label}: 请求数 ${reqs.length} ≠ 期望 ${expected.length}（场景没跑完或有多余请求）`);
  }
  for (const [i, want] of expected.entries()) {
    const got = roleSeqOf(reqs[i]);
    const g = got.join(',');
    const w = want.join(',');
    if (g !== w) {
      throw new Error(`${label}: 第 ${i + 1} 轮请求对话序列不符\n  期望: ${w}\n  实际: ${g}`);
    }
  }
}

/**
 * LC-003b 用户发言保真：每轮用户原话，从它**首次进场**的那条请求起，在后续每条
 * 请求里恰好出现一次；进场前一次都不得出现。
 *
 * 拦的失效：同一句话在一条请求里重复（重放 bug）、某轮原话在后续请求里丢了
 * （历史被截断 / 覆盖）、或原话「提前」出现（未来轮次泄漏——缓存锚点错位）。
 *
 * ⚠️ 请求与轮次**不是**一一对应的：一轮带工具轮的会话会产生多条请求
 * （工具轮 + 工具结果轮 + 下一轮）。所以调用方必须显式给出每轮原话的
 * **首次进场请求下标**（`introAt`），本函数不猜这个映射。
 *
 * @param reqs 请求序列（按时间序）
 * @param opts.utterances 各轮用户原话（按轮序）
 * @param opts.introAt 各轮原话首次进场的请求下标（与 utterances 等长）
 */
export function assertUtteranceReplay(reqs, { utterances, introAt }, label) {
  if (utterances.length !== introAt.length) {
    throw new Error(`${label}: utterances 与 introAt 不等长（${utterances.length} vs ${introAt.length}）`);
  }
  for (const [k, text] of utterances.entries()) {
    const from = introAt[k];
    for (const [j, req] of reqs.entries()) {
      const hits = dialogOf(req).filter((m) => m.role === 'user' && textOf(m).includes(text)).length;
      if (j < from && hits !== 0) {
        throw new Error(`${label}: 第 ${k + 1} 轮原话「${text}」不该出现在第 ${j + 1} 条请求里（出现 ${hits} 次）`);
      }
      if (j >= from && hits !== 1) {
        throw new Error(`${label}: 第 ${k + 1} 轮原话「${text}」在第 ${j + 1} 条请求里出现 ${hits} 次（应恰好 1 次）`);
      }
    }
  }
}

/**
 * LC-003c 工具轮结果跨轮存活：从指定请求下标起，每条请求都能看到该次工具
 * 调用的结果正文（同一轮内是 `role=tool` 消息；后续轮里塌成带标记的
 * assistant 消息——「（此前调用工具 X 的结果）」+ 结果正文）。
 *
 * 拦的失效：工具结果不累进的同族——工具结果不累进下一轮请求，长轮后模型「失忆」
 * （前端症状是一串思考没有正文）。转写侧判据在 t55；本函数判**请求包**侧，
 * 且钉的是塌缩形态本身（回归成原文重放 / 结果丢失都会红）。
 */
export function assertToolResultCarriedForward(
  reqs,
  { toolName, resultText, fromIndex },
  label,
) {
  for (const [j, req] of reqs.entries()) {
    if (j < fromIndex) continue;
    const body = JSON.stringify(req.body ?? {});
    if (!body.includes(resultText)) {
      throw new Error(`${label}: 第 ${j + 1} 条请求看不到工具结果正文「${resultText.slice(0, 40)}」`);
    }
    if (!body.includes(toolName)) {
      throw new Error(`${label}: 第 ${j + 1} 条请求看不到工具名「${toolName}」（结果与调用对不上）`);
    }
  }
}

// ---------- LC-004 工具反馈契约 ----------

/**
 * LC-004 工具反馈契约：请求里每个 `tool_calls` 条目都有配对的 `role=tool`
 * 结果（`tool_call_id` 一致、正文非空）——「有请求必有响应」在**模型眼前**的形态。
 *
 * 拦的失效：工具执行了但结果没回灌（模型看到自己调了工具、却不知道结果）、
 * `tool_call_id` 串格（A 的结果配到 B 的名下——多工具轮里最危险的一种）、
 * 空结果回灌（模型读到空响应该干什么全凭猜）。
 *
 * 转写侧的同形判据是 `assertTranscriptInvariants`（落盘后）；本函数判的是
 * **发给模型的请求包**——坏在回灌路上时，只有这里看得见。
 */
export function assertToolFeedback(req, label) {
  const calls = toolCallsOf(req);
  const results = toolResultsOf(req);
  for (const c of calls) {
    const id = c.id;
    const name = c.function?.name ?? '(无名)';
    const hit = results.find((r) => r.tool_call_id === id);
    if (!hit) {
      throw new Error(
        `${label}: 工具调用 ${name}(${id}) 在请求里没有配对的 role=tool 结果` +
          `（实际结果 id: ${JSON.stringify(results.map((r) => r.tool_call_id))}）`,
      );
    }
    if (!textOf(hit).trim()) {
      throw new Error(`${label}: 工具调用 ${name}(${id}) 的结果为空（空回灌）`);
    }
  }
}

/**
 * LC-004b 首轮（工具尚未执行）不得出现 role=tool。
 *
 * 拦的失效：把上一会话/上一进程的工具结果泄漏进首轮请求（跨会话串味）。
 */
export function assertNoToolResultsInFirstTurn(reqs, label) {
  const first = reqs[0];
  if (!first) throw new Error(`${label}: 没有请求可判`);
  const leaked = toolResultsOf(first);
  if (leaked.length) {
    throw new Error(`${label}: 首轮请求不得带 role=tool 结果（实际 ${leaked.length} 条）`);
  }
}

// ---------- LC-005 工具面契约 ----------

/**
 * LC-005 工具面契约：发给模型的工具集合 = 出厂集 + 用例声明的增量，**不多不少**。
 *
 * 拦的失效：
 * - 出厂工具漂移（加/删一个内置工具而没人知道——模型能力面静默变化）；
 * - 内部面泄漏（路由 / 插件管理 / 通道类名字混进工具面）；
 * - 条件工具不守条件（read_skill 在无技能时在场 = 空 enum 的工具；
 *   MCP 工具在未连接时在场 = 幽灵工具）。
 *
 * @param req 一条请求
 * @param opts.extra 额外应在场的 wire 名（MCP 桥接 / read_skill 等条件工具）
 * @param opts.absent 必须不在场的 wire 名（默认一张内部面黑名单）
 * @param opts.exact 严格集合相等（默认 true）；false 时只判「expected ⊆ 实际 ∧ absent ∩ 实际 = ∅」
 */
export function assertToolSurface(
  req,
  { extra = [], absent = INTERNAL_NAMES, exact = true } = {},
  label,
) {
  const actual = new Set(toolNamesOf(req));
  const expected = new Set([...FACTORY_TOOLS, ...extra]);
  const missing = [...expected].filter((t) => !actual.has(t));
  if (missing.length) {
    throw new Error(`${label}: 工具面缺 ${missing.length} 个（${missing.join(', ')}；实际: ${[...actual].join(', ')}）`);
  }
  const leaked = [...actual].filter((t) => absent.includes(t));
  if (leaked.length) {
    throw new Error(`${label}: 工具面泄漏内部名（${leaked.join(', ')}）`);
  }
  if (exact) {
    const extraActual = [...actual].filter((t) => !expected.has(t));
    if (extraActual.length) {
      throw new Error(`${label}: 工具面多出 ${extraActual.length} 个未声明工具（${extraActual.join(', ')}）`);
    }
  }
  // 形状：每个工具都是 function 形态、有描述、有参数 schema
  for (const t of req?.body?.tools ?? []) {
    const name = t?.function?.name ?? '(无名)';
    if (t?.type !== 'function') throw new Error(`${label}: 工具 ${name} 的 type 必须为 function（实际 ${t?.type}）`);
    if (!t?.function?.description?.trim()) throw new Error(`${label}: 工具 ${name} 缺描述`);
    if (t?.function?.parameters == null || typeof t.function.parameters !== 'object') {
      throw new Error(`${label}: 工具 ${name} 缺 parameters schema`);
    }
  }
}

/**
 * 内部面黑名单：这些名字**永远**不是 LLM 可见工具——它们是前端路由 / 宿主面。
 * 出现在工具面 = 能力广播把内部路由也注册了（`plugin-entry-audit` 的 E 族盯的
 * 就是这类越界，这里从模型侧再钉一道）。
 */
export const INTERNAL_NAMES = Object.freeze([
  'plugin_manager',
  'gateway',
  'gateway_status',
  'telegram',
  'telegram_send',
  'hook',
  'hook_fire',
  'event_bus',
  'event_bus_ping',
  'vdfs_root',
  'route',
  'session_chat_send',
]);

/**
 * LC-005b 子智能体挂载后的工具面：根工具一个不少，且每个根工具都有
 * `agent_<id>_<工具>` 的对偶（子空间能力经前缀寻址——t30 主体不串包的名字基础）。
 *
 * 拦的失效：挂了 agent 子树后根工具被挤掉（收集广播漏了兄弟插件）、
 * 前缀寻名不一致（一边 `agent_probe-agent_x` 一边 `agent/probe-agent/x`）。
 */
export function assertAgentPrefixedTwins(req, agentId, label) {
  const actual = new Set(toolNamesOf(req));
  const prefix = `agent_${agentId}_`;
  const missingRoot = FACTORY_TOOLS.filter((t) => !actual.has(t));
  if (missingRoot.length) {
    throw new Error(`${label}: 绑定智能体后根工具被挤掉（${missingRoot.join(', ')}）`);
  }
  const missingTwin = FACTORY_TOOLS.filter((t) => t !== PLATFORM_VARIANT_TOOL && !actual.has(`${prefix}${t}`));
  if (missingTwin.length) {
    throw new Error(`${label}: 根工具缺少 agent 前缀对偶（${missingTwin.map((t) => prefix + t).join(', ')}）`);
  }
}

// ---------- LC-006 无内部泄漏 ----------

/**
 * LC-006 用户消息纯度：user 消息正文 == 用户原话。
 *
 * 拦的失效：把系统前缀 / 时间上下文 / 路由信息包进用户发言正文（那会同时污染
 * 模型对「用户说了什么」的理解与 mock 的场景匹配）；时间上下文走的是
 * `prompt` 字段（`message_build.rs::temporal_context`），正文必须保持原话。
 */
export function assertUserTextPure(req, expectedText, label) {
  const users = dialogOf(req).filter((m) => m.role === 'user');
  const hit = users.find((m) => textOf(m) === expectedText);
  if (!hit) {
    throw new Error(
      `${label}: 找不到正文恰为用户原话的 user 消息（期望 ${JSON.stringify(expectedText)}；实际: ${JSON.stringify(users.map((m) => textOf(m).slice(0, 80)))}）`,
    );
  }
}

/**
 * LC-006b 环境不泄漏进 prompt：system 消息不得含 homedir / workdir / mock 端口
 * / 回环地址。
 *
 * 拦的失效：把测试/宿主环境细节（绝对路径、端口）拼进提示词——一是噪声，
 * 二是「在哪台机器上跑」会改变模型看到的内容（可复现性直接没了）。
 * 判据只覆盖**环境细节**：记忆地址用相对路径（`.vdfsv2/...`）是既定设计，
 * 不在本判据范围。
 */
export function assertNoEnvLeak(req, { homedir, workdir, port } = {}, label) {
  const text = systemTextOf(req);
  const bad = [];
  for (const [what, needle] of [
    ['homedir', homedir],
    ['workdir', workdir],
    ['mock 端口', port != null ? `127.0.0.1:${port}` : null],
    ['mock 端口(裸)', port != null ? `:${port}` : null],
  ]) {
    if (needle && text.includes(needle)) bad.push(what);
  }
  if (bad.length) {
    throw new Error(`${label}: system 提示词泄漏环境细节（${bad.join('、')}；实际前 200 字符: ${text.slice(0, 200)}）`);
  }
}

// ---------- LC-007 辅助请求形状 ----------

/**
 * 判「这是不是一次静默辅助请求」：**不带工具面**的是辅助请求（分类 / 措辞 /
 * 压缩都经 `execute_turn(_, _, &[], …)` 发），带工具面的是主线轮次。
 *
 * 这不是「无工具 = 辅助」的巧合判断，而是三条辅助路径的共同实现形态
 * （classify `decide.rs` / compose `wording.rs` / 压缩 `pipeline.rs` 全部传空工具），
 * 主线轮次恒带出厂工具面。二者在请求面上可判别，才有本文件的辅助形状判据。
 */
export function isSilentAuxRequest(req) {
  return !envelopeOf(req).hasTools;
}

/**
 * LC-007 分类请求形状：system 是分流器提示词、无工具面、末条 user 是用户原话。
 *
 * 拦的失效：对话面调优把分类请求改成带工具 / 换提示词 / 多带历史——
 * 「这一轮走哪条路」的判定输入被静默改变（t21 判判决后果，本函数判请求形态）。
 */
export function assertClassifyRequestShape(req, { systemMarker = '你是一个意图分流器', utterance }, label) {
  if (!isSilentAuxRequest(req)) {
    throw new Error(`${label}: 分类请求不得带工具面（实际 ${toolNamesOf(req).length} 个工具）`);
  }
  const sys = systemTextOf(req);
  if (!sys.includes(systemMarker)) {
    throw new Error(`${label}: 分类请求的 system 应为分流器提示词（实际前 120 字符: ${sys.slice(0, 120)}）`);
  }
  if (utterance) assertLastUserIs(req, utterance, `${label}（分类请求）`);
}

/**
 * LC-007b 措辞请求形状：system 带生成指令段（且指令段在注册段之后）、无工具面。
 *
 * 拦的失效：指令段被注册段盖住（`wording.rs` 的顺序纪律：指令放后面才压得住
 * 人格段的措辞习惯）、生成路径开始带工具（「不要调用任何工具」是指令内容，
 * 带上工具面就是让模型能违令）。
 */
export function assertComposeRequestShape(req, { systemMarker = '你是这个助手本人', utterance }, label) {
  if (!isSilentAuxRequest(req)) {
    throw new Error(`${label}: 措辞请求不得带工具面（实际 ${toolNamesOf(req).length} 个工具）`);
  }
  const sys = systemTextOf(req);
  if (!sys.includes(systemMarker)) {
    throw new Error(`${label}: 措辞请求的 system 应含生成指令段（实际前 120 字符: ${sys.slice(0, 120)}）`);
  }
  if (utterance) assertLastUserIs(req, utterance, `${label}（措辞请求）`);
}

/**
 * LC-007c 压缩（摘要）请求形状：system 是压缩提示词、无工具面、
 * user 消息带摘要指令标记。
 *
 * 拦的失效：摘要请求带上工具面（模型在压缩时调用工具 = 副作用发生在
 * 「只读历史」的请求里）、压缩提示词被换掉（快照结构随契约变）。
 */
export function assertCompressionRequestShape(req, { userMarker = 'Distill the conversation above' }, label) {
  if (!isSilentAuxRequest(req)) {
    throw new Error(`${label}: 压缩请求不得带工具面（实际 ${toolNamesOf(req).length} 个工具）`);
  }
  const sys = systemTextOf(req);
  if (!sys.includes('state_snapshot')) {
    throw new Error(`${label}: 压缩请求的 system 应为压缩提示词（含 state_snapshot 协议；实际前 120 字符: ${sys.slice(0, 120)}）`);
  }
  const hasMarker = dialogOf(req).some((m) => m.role === 'user' && textOf(m).includes(userMarker));
  if (!hasMarker) {
    throw new Error(`${label}: 压缩请求的 user 消息应含摘要指令标记「${userMarker}」`);
  }
}

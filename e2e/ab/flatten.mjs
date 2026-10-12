// T-048 的两个臂之间唯一的差别：**工具面与工具交换用什么形状送进模型**。
//
// - 臂 A（结构化 / 出厂现状）：`tools` 数组走原生协议字段， assistant 侧带
//   `tool_calls`，工具结果走 `role=tool` + `tool_call_id`。这就是
//   [`docs/ref/PROMPT_SHAPE.md`](../../docs/ref/PROMPT_SHAPE.md) 钉住的形状。
// - 臂 B（拍平 / 被否决的那条路）：删掉 `tools`，工具明细渲染进 system 正文；
//   `tool_calls` 渲染成正文里的 JSON 行；`role=tool` 的结果并进一条 user 消息的散文。
//   这正是 [ADR-048a](../../docs/adr/ADR-048a.md) 当年判为退步的那个形态。
//
// 拍平是**纯函数**：同样的输入必然同样的输出，两臂除这里以外没有任何差别
// （同一条 system 前缀、同一份 user 提问、同一个信封）。判据要的「只比形状」就靠这个。
//
// ⚠️ 本文件只服务门外实验（`node e2e/ab/run.mjs`），不进任何一条产品代码路径，
// 也不进门禁——[ADR-058](../../docs/adr/ADR-058.md) 被否决方案第 1 条明令
// 真实模型对照不能当门内判据。

/** 把一个工具渲染成正文里的一段（名称 / 描述 / 完整参数 Schema 原样进文本）。 */
export function renderToolEntry(tool) {
  const f = tool.function ?? tool;
  const schema = f.parameters ? JSON.stringify(f.parameters) : '{}'
  return `- ${f.name}\n  说明：${(f.description ?? '').trim()}\n  参数 Schema：${schema}`;
}

/** 工具目录 + 调用协议说明——臂 B 里「模型怎么知道有什么工具、怎么调用」。 */
export function renderToolCatalog(tools) {
  const lines = tools.map(renderToolEntry);
  return [
    '【工具明细（文本）】',
    ...lines,
    '',
    '【工具调用协议（文本）】',
    '需要调用工具时，只输出一行 JSON，不要输出别的内容：',
    '{"tool": "<工具名>", "arguments": { <参数对象> }}',
    '需要同时调用多个工具就输出多行，每行一个。',
    '不需要工具时，直接正常回答。',
  ].join('\n');
}

const textOf = (m) => (typeof m.content === 'string' ? m.content : (m.content ?? []).map((p) => p.text ?? '').join(''));

/** 一条 assistant 消息里的 `tool_calls` 渲染成正文里的 JSON 行。 */
function renderAssistantCalls(m) {
  const calls = (m.tool_calls ?? []).map((c) => {
    let args = c.function?.arguments;
    if (typeof args === 'string') {
      try { args = JSON.parse(args); } catch { args = args; }
    }
    return `{"tool": ${JSON.stringify(c.function?.name ?? c.name ?? '')}, "arguments": ${JSON.stringify(args ?? {})}}`;
  });
  const body = textOf(m).trim();
  return [body, ...calls].filter(Boolean).join('\n');
}

/**
 * 结构化请求体 → 拍平请求体。
 *
 * 三处变形，其余字节不动：
 * 1. `tools` 字段删除，其内容渲染进**最后一条 system 消息**的正文尾部；
 * 2. `assistant.tool_calls` → 该条正文里的 JSON 行；
 * 3. `role=tool` 的消息并进其后的一条 user 消息（`工具 <name> 的返回：…`），
 *    连续的 tool 结果合成同一条 user 消息——结果与调用的对应关系只靠散文里的名字。
 */
export function flattenRequest(body) {
  const out = structuredClone(body);
  const tools = out.tools ?? [];
  delete out.tools;
  delete out.tool_choice;

  const msgs = out.messages;
  let lastSystem = -1;
  msgs.forEach((m, i) => { if (m.role === 'system') lastSystem = i; });
  const catalog = `${renderToolCatalog(tools)}\n【/工具明细】`;
  if (lastSystem >= 0) {
    const s = structuredClone(msgs[lastSystem]);
    s.content = `${textOf(s).trimEnd()}\n\n${catalog}`;
    msgs[lastSystem] = s;
  } else {
    msgs.unshift({ role: 'system', content: catalog });
  }

  const flat = [];
  for (const m of msgs) {
    if (m.role === 'assistant') {
      flat.push({ role: 'assistant', content: renderAssistantCalls(m) });
      continue;
    }
    if (m.role === 'tool') {
      const head = `工具 ${m.name ?? ''} 的返回：`;
      const text = `${head}\n${textOf(m)}`;
      const prev = flat[flat.length - 1];
      if (prev && prev.role === 'user' && prev.content.startsWith('工具 ')) {
        prev.content = `${prev.content}\n\n${text}`;
      } else {
        flat.push({ role: 'user', content: text });
      }
      continue;
    }
    flat.push({ role: m.role, content: textOf(m) });
  }
  out.messages = flat;
  return out;
}

/** 从正文里扫出 JSON 对象（跨行也认：按花括号配对取，忽略字符串内的括号）。 */
function jsonObjects(text) {
  const found = [];
  for (let i = 0; i < text.length; i++) {
    if (text[i] !== '{') continue;
    let depth = 0;
    let inStr = false;
    let esc = false;
    for (let j = i; j < text.length; j++) {
      const ch = text[j];
      if (inStr) {
        if (esc) esc = false;
        else if (ch === '\\') esc = true;
        else if (ch === '"') inStr = false;
        continue;
      }
      if (ch === '"') inStr = true;
      else if (ch === '{') depth++;
      else if (ch === '}') {
        depth--;
        if (depth === 0) {
          const slice = text.slice(i, j + 1);
          try {
            const obj = JSON.parse(slice);
            if (obj && typeof obj === 'object' && !Array.isArray(obj)) found.push({ obj, raw: slice });
          } catch { /* 不是合法 JSON，继续找下一个起点 */ }
          i = j;
          break;
        }
      }
    }
  }
  return found;
}

const asCall = ({ obj, raw }) => {
  const name = obj.tool ?? obj.name ?? obj.function;
  if (typeof name !== 'string' || !name.trim()) return null;
  let args = obj.arguments ?? obj.args ?? obj.parameters ?? {};
  if (typeof args === 'string') {
    try { args = JSON.parse(args); } catch { args = { _raw: args }; }
  }
  return { name: name.trim(), arguments: args && typeof args === 'object' ? args : null, raw: raw.replace(/\s+/g, ' ') };
};

/**
 * 从正文里解析臂 B 期望的 JSON 调用。
 *
 * ⚠️ 只认「一行一个 JSON」会把**多行 pretty-print 的合法调用判成没调用**——
 * 那是解析器的偏见，不是臂 B 的失败（实测 MiMo 在臂 B 就给出过跨行的 JSON）。
 * 所以逐行之外再补一遍花括号配对的跨行扫描，两臂吃同一套宽松度。
 */
export function parseFlatCalls(content) {
  const text = String(content ?? '');
  const calls = [];
  const noise = [];
  for (const line of text.split(/\r?\n/)) {
    const s = line.trim().replace(/^```(?:json)?$/i, '');
    if (!s.startsWith('{')) { if (s) noise.push(s); continue; }
    try {
      const obj = JSON.parse(s);
      const call = asCall({ obj, raw: s });
      if (call) { calls.push(call); continue; }
    } catch { /* 跨行的 JSON，落到下面的配对扫描 */ }
  }
  if (calls.length === 0) {
    for (const hit of jsonObjects(text)) {
      const call = asCall(hit);
      if (call) calls.push(call);
    }
  }
  return { calls, prose: calls.length ? '' : noise.join('\n') };
}

/** 臂 A（原生协议字段）的调用取成与臂 B 同一形状，好让判据两臂共用。 */
export function structuredCalls(message) {
  return (message?.tool_calls ?? []).map((c) => {
    let args = null;
    let parsed = true;
    const raw = c.function?.arguments;
    if (typeof raw === 'string') {
      try { args = JSON.parse(raw); } catch { args = null; parsed = false; }
    } else if (raw && typeof raw === 'object') {
      args = raw;
    }
    return { name: c.function?.name ?? c.name ?? '', arguments: args, arguments_raw: raw, json_ok: parsed };
  });
}

// 模型自己训练形态里的调用标记（下面三行拼出来，避免在源码里写出会被模板吃掉的字面量）。
const OPEN_CALL = '<' + '|tool_call|' + '>';
const FN_OPEN = '<' + 'function=';
const PARAM_OPEN = '<' + 'parameter=';

/**
 * 正文里是否出现过内部调用标记——**与调用是从哪一处读回来的无关**，问的是「它写了什么」。
 *
 * 这两个问题必须分开：实测有道题把两枚 `OPEN_CALL` 写在前面、后面照样跟了一行合法 JSON，
 * 按「出处」判它「遵守了协议」是对的（工具名确实从协议那一行来），按「漏没漏」判它
 * 「没漏」就是假的。协议那一句是「只输出一行 JSON，不要输出别的内容」。
 */
export function hasNativeMarkup(text) {
  return String(text ?? '').includes(FN_OPEN) || String(text ?? '').includes(OPEN_CALL);
}

/**
 * 臂 B 的第二种出处：模型**没按文本协议答，而是把训练形态里的调用标记漏进正文**
 * （实测 MiMo 在拍平臂上多次直接输出 `OPEN_CALL + FN_OPEN` 那一族标记）。
 *
 * 这件事本身要测，所以解析器必须认得它——否则「选没选对工具」和「听没听懂文本协议」
 * 混成一列，臂 B 会比拍平本身的代价更差，读数是假的。于是判据拆两列：
 * `tool_name_hit` 走「两种出处都算」，`protocol_followed` 只走文本协议那一行。
 */
export function parseNativeMarkupCalls(text) {
  const src = String(text ?? '');
  if (!src.includes(FN_OPEN)) return [];
  const out = [];
  const callRe = new RegExp(FN_OPEN + '([\\w.\\-:]+)>([\\s\\S]*?)<\\/' + 'function>', 'g');
  let m;
  while ((m = callRe.exec(src))) {
    const args = {};
    const pRe = new RegExp(PARAM_OPEN + '([\\w.\\-:]+)>([\\s\\S]*?)<\\/' + 'parameter>', 'g');
    let p;
    while ((p = pRe.exec(m[2]))) {
      const rawVal = p[2].trim();
      let val = rawVal;
      try { val = JSON.parse(rawVal); } catch { /* 不是 JSON 就当字符串 */ }
      args[p[1]] = val;
    }
    out.push({ name: m[1], arguments: args, raw: (OPEN_CALL + m[0]).replace(/\s+/g, ' ').slice(0, 200) });
  }
  return out;
}


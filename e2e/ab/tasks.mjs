// S-end③ 任务集：**同一道题在两种形状下的问法**（口径归本文件，报告只读结果）。
//
// 三条设计约束，都是为了让「差异能归因给形状」：
//
// 1. **上下文两臂同源**。每道题的输入一律先写成**结构化形态**（出厂形态），
//    臂 B 由 `flattenRequest` 从同一份输入派生 —— 不存在「手抄两份题目」这种偏差源。
//    system 前缀与 `tools` 数组整体取自真实链路捕获的出厂请求体（`capture-corpus.mjs`），
//    只有尾部消息按题目替换。
// 2. **真值只有一条题面事实**：这道题该不该调工具、该调哪个。必填参数键不手写，
//    直接从该工具的 JSON Schema 的 `required` 读 —— 真值随工具面自己走。
// 3. **三类题各测一件不同的事**：
//    - `select`：工具面上能否**选对工具并给出合法参数**（臂 B 还要额外测「能否遵守文本协议」）；
//    - `control`：**不该调用**时会不会硬调（过触发；拍平后工具说明变成正文，
//      模型更容易把描述当指令）；
//    - `attribution`：工具结果与调用的**对应关系**在两种形状下是否还读得回来 ——
//      这是 ADR-048a 理由 1 的唯一可测主张（`tool_call_id` 丢了会怎样）。
//
// 副作用：这个 runner **没有工具执行方**（它只发一次 HTTP 请求，谁都不真的去写、删、执行），
// 所以题面可以照出厂口径直写「删掉 tmp/old.txt」，不必加「只说方案不要动手」那类限定——
// 上一版加了，实测那是混进判据的干扰：臂 A 把它读成「别调用」（3 道 select 因此没调用），
// 臂 B 把它读成「用一行 JSON 描述就行」（照样调用），两臂之差来自题面措辞而不是形状。

/**
 * 两次**同名**调用、结果**乱序**返回、结果正文里不回显查询对象。
 *
 * 这一题才是 ADR-048a 理由 1 的可测形态：臂 A 的两个结果各带 `tool_call_id`，
 * 与 assistant 那条 `tool_calls` 里的 `arguments` 能对上 ⇒ 「path=src 那次是哪一条」有机器可读的答案；
 * 臂 B 只剩散文里的「工具 vdfs_list 的返回」（名字相同、顺序又被打了乱）⇒ 只能猜。
 * 这不是给臂 B 设套——**散文里本来就只有工具名**正是那次判定所描述的退步本身
 * （「只剩散文里的 `工具结果(名)`」）。
 */
const ATTR_OUT_OF_ORDER = {
  id: 'attr-out-of-order-same-name',
  face: 'attribution',
  why: '同名调用 + 乱序结果 + 正文不回显路径 ⇒ 判据测的就是 tool_call_id 这一层关联',
  candidates: 2,
  expected_marker: 'lib.rs',
  forbidden_marker: 'plan',
  turns: [
    { role: 'user', content: '一次性分别列出 src 目录和 docs 目录下的直接子项，不要分两轮。' },
    {
      role: 'assistant',
      content: '',
      tool_calls: [
        { id: 'call_src_01', type: 'function', function: { name: 'vdfs_list', arguments: '{"path":"src"}' } },
        { id: 'call_docs_02', type: 'function', function: { name: 'vdfs_list', arguments: '{"path":"docs"}' } },
      ],
    },
    // 乱序：docs 那次先到。
    { role: 'tool', tool_call_id: 'call_docs_02', name: 'vdfs_list', content: '{"entries":["plan","registry"],"count":2}' },
    { role: 'tool', tool_call_id: 'call_src_01', name: 'vdfs_list', content: '{"entries":["lib.rs","main.rs"],"count":2}' },
    { role: 'user', content: '上面两次工具调用里，**path 是 `src` 的那一次**返回的 entries 里第一个文件名是什么？只回答那个文件名。' },
  ],
};

/**
 * 与上一题**同一份上下文**，只把提问换成另一那次调用（path=docs）。
 *
 * 这一对是**偏置探测器**：两题的正确答案是两条结果里各自的一条，而「按到达顺序取第一条」
 * 这个启发式在一题上必错、在另一题上必对。两臂若在两题上给出同一个（错误的）指向，
 * 说明它压根没用关联信息，只是按顺序读——那是比「对错」更硬的读数。
 */
const ATTR_OUT_OF_ORDER_MIRROR = {
  id: 'attr-out-of-order-mirror',
  face: 'attribution',
  why: '同一上下文换问另一那次调用 ⇒ 与上一题配对，测的是「是否按到达顺序归因」而不是碰运气',
  candidates: 2,
  expected_marker: 'plan',
  forbidden_marker: 'lib.rs',
  turns: [
    { role: 'user', content: '一次性分别列出 src 目录和 docs 目录下的直接子项，不要分两轮。' },
    {
      role: 'assistant',
      content: '',
      tool_calls: [
        { id: 'call_src_01', type: 'function', function: { name: 'vdfs_list', arguments: '{"path":"src"}' } },
        { id: 'call_docs_02', type: 'function', function: { name: 'vdfs_list', arguments: '{"path":"docs"}' } },
      ],
    },
    { role: 'tool', tool_call_id: 'call_docs_02', name: 'vdfs_list', content: '{"entries":["plan","registry"],"count":2}' },
    { role: 'tool', tool_call_id: 'call_src_01', name: 'vdfs_list', content: '{"entries":["lib.rs","main.rs"],"count":2}' },
    { role: 'user', content: '上面两次工具调用里，**path 是 `docs` 的那一次**返回的 entries 里第一个文件名是什么？只回答那个文件名。' },
  ],
};

/** 一次 `vdfs_stat`，结果里有三个字段；问其中特定一个。 */
const ATTR_FIELD = {
  id: 'attr-pick-one-field',
  face: 'attribution',
  why: '结果是一段 JSON 文本，问其中一个字段 ⇒ 测「读回结果内容」而不是「读回关联」（单调用，无猜的空间）',
  candidates: 1,
  expected_marker: '2048',
  forbidden_marker: '262144',
  turns: [
    { role: 'user', content: '看一下 README.md 这个文件的元数据（不要读内容）。' },
    {
      role: 'assistant',
      content: '',
      tool_calls: [{ id: 'call_stat_01', type: 'function', function: { name: 'vdfs_stat', arguments: '{"path":"README.md"}' } }],
    },
    { role: 'tool', tool_call_id: 'call_stat_01', name: 'vdfs_stat', content: '{"name":"README.md","type":"file","size":2048,"modified":"2026-10-12T00:00:00Z","max_context_tokens":262144}' },
    { role: 'user', content: '刚才那次工具返回里的 `size` 字段是多少？只回答数字。' },
  ],
};

/** 三次同名调用、三份不回显路径的结果、乱序返回；问中间那一次。 */
const ATTR_THREE_WAY = {
  id: 'attr-three-way-permuted',
  face: 'attribution',
  why: '三次同名调用的关联面——一次对不上还能猜，三次全对上的概率才说明关联是不是真在读',
  candidates: 3,
  expected_marker: 'Cargo.toml',
  forbidden_marker: 'agents.md',
  turns: [
    { role: 'user', content: '一次性分别查这三个文件的元数据：agents.md、Cargo.toml、docs/README.md。' },
    {
      role: 'assistant',
      content: '',
      tool_calls: [
        { id: 'call_a', type: 'function', function: { name: 'vdfs_stat', arguments: '{"path":"agents.md"}' } },
        { id: 'call_b', type: 'function', function: { name: 'vdfs_stat', arguments: '{"path":"Cargo.toml"}' } },
        { id: 'call_c', type: 'function', function: { name: 'vdfs_stat', arguments: '{"path":"docs/README.md"}' } },
      ],
    },
    { role: 'tool', tool_call_id: 'call_c', name: 'vdfs_stat', content: '{"type":"file","size":4096}' },
    { role: 'tool', tool_call_id: 'call_b', name: 'vdfs_stat', content: '{"type":"file","size":8192}' },
    { role: 'tool', tool_call_id: 'call_a', name: 'vdfs_stat', content: '{"type":"file","size":2048}' },
    { role: 'user', content: 'size 为 8192 的那一次返回，对应的是哪一个文件？只回答文件名。' },
  ],
};

/** 五次同名调用、结果按**逆序**回来、正文不回显路径；问第 4 次。 */
const ATTR_FIVE_WAY = {
  id: 'attr-five-way-reversed',
  face: 'attribution',
  why: '候选到 5 条且完全逆序 ⇒ 猜的基线降到 1/5，而「按到达顺序」的启发式在这里必然指错',
  candidates: 5,
  expected_marker: '400',
  forbidden_marker: '500',
  turns: [
    { role: 'user', content: '一次性分别查这五个文件的元数据：e1.txt、e2.txt、e3.txt、e4.txt、e5.txt。' },
    {
      role: 'assistant',
      content: '',
      tool_calls: [1, 2, 3, 4, 5].map((n) => ({
        id: `call_e${n}`,
        type: 'function',
        function: { name: 'vdfs_stat', arguments: `{"path":"e${n}.txt"}` },
      })),
    },
    ...[5, 4, 3, 2, 1].map((n) => ({
      role: 'tool',
      tool_call_id: `call_e${n}`,
      name: 'vdfs_stat',
      content: `{"type":"file","size":${100 * n}}`,
    })),
    { role: 'user', content: '五次返回里，**path 是 `e4.txt` 的那一次**返回的 size 是多少？只回答数字。' },
  ],
};


export const TASKS = [
  // ---- select：工具面上选对工具 ----
  { id: 'sel-list-dir', face: 'select', tool: 'vdfs_list', ask: '列出 src 目录下的直接子项，只要一层。' },
  { id: 'sel-tree', face: 'select', tool: 'vdfs_tree', ask: '整体看下 docs 目录的层级结构，深度到 2 层就够。' },
  { id: 'sel-read-file', face: 'select', tool: 'vdfs_read', ask: '把 Cargo.toml 的内容读给我看。' },
  { id: 'sel-stat', face: 'select', tool: 'vdfs_stat', ask: 'README.md 这个文件现在多大、最后什么时候改的？只看元数据别读内容。' },
  { id: 'sel-glob', face: 'select', tool: 'vdfs_search', ask: '找出仓库里所有的 .rs 源文件，按文件名匹配就行。' },
  { id: 'sel-grep', face: 'select', tool: 'content_search', ask: '在代码内容里搜一下正则 `fn main`，看它出现在哪些地方。' },
  { id: 'sel-write', face: 'select', tool: 'vdfs_write', ask: '把内容 "hello" 写进 notes/greeting.txt。' },
  { id: 'sel-edit', face: 'select', tool: 'vdfs_edit', ask: '把 src/main.rs 里出现的 `foo` 精确替换成 `bar`。' },
  { id: 'sel-delete', face: 'select', tool: 'vdfs_delete', ask: '删掉 tmp/old.txt 这个文件。' },
  { id: 'sel-websearch', face: 'select', tool: 'web_search', ask: '搜一下互联网上关于 rust async 生命周期 的教程。' },
  { id: 'sel-webfetch', face: 'select', tool: 'web_fetch', ask: '抓取 https://example.com 这个网页的内容给我看。' },
  { id: 'sel-http', face: 'select', tool: 'http_request', ask: '给 https://api.example.com/data 发一个 HTTP GET 请求，看返回什么。' },
  { id: 'sel-todo', face: 'select', tool: 'todo_write', ask: '把「分析架构、拆分模块、补测试」列成本会话的结构化任务清单。' },
  { id: 'sel-agent', face: 'select', tool: 'agent_run', ask: '把「调研本仓库的压缩策略并写一份小结」这件事委托给一个独立的子智能体会话去做。' },

  // ---- control：不该调用工具 ----
  { id: 'ctl-no-tool-math', face: 'control', ask: '不用查任何文件：128 乘以 7 等于多少？直接给数字。' },
  { id: 'ctl-no-tool-recall', face: 'control', ask: '用你自己的话说一下：上面 system 里提到的【会话记忆】和【工作区记忆】各自是什么作用？别调用任何工具。' },
  { id: 'ctl-no-tool-refuse', face: 'control', ask: '我不需要你做任何事，也不要调用工具。就回一句「明白」就行。' },

  // ---- attribution：结果与调用的对应关系 ----
  ATTR_OUT_OF_ORDER,
  ATTR_OUT_OF_ORDER_MIRROR,
  ATTR_THREE_WAY,
  ATTR_FIVE_WAY,
  ATTR_FIELD,
];

/** 题目输入的结构化形态（臂 A 送出的东西）：两臂都从这一份出发。 */
export function structuredBodyFor(corpus, task) {
  const base = structuredClone(corpus.request);
  base.messages = base.messages.filter((m) => m.role !== 'user');
  if (task.face === 'attribution') {
    base.messages.push(...structuredClone(task.turns));
  } else {
    base.messages.push({ role: 'user', content: task.ask });
  }
  return base;
}

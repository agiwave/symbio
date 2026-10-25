// 异常注入矩阵的**唯一数据源**：`注入字段 × 能力面`，每格归属一个用例 / 一条不适用理由 /
// 一条缺口归属。`scripts/injection-matrix-audit.mjs` 判它自洽，`scripts/facts/injection-matrix.mjs`
// 渲染成 `docs/004-ref/012-注入矩阵.md`。
//
// ## 为什么矩阵是数据而不是文档
//
// 手写的矩阵有两个坏死方向：**用例号指向不存在的文件**，以及**夹具新加一个注入字段、
// 矩阵没人记得加一行**。前者是死链，后者是「看着全、其实漏了一整类」。把矩阵写成数据后，
// 两个方向都能静态判：用例号逐个解析到 `e2e/cases/`，字段的 `inMock` 必须真的出现在夹具源码里；
// 反向再扫一遍用例，用到字段却没被任何格认领 ⇒ 红。
//
// ## 口径
//
// - **注入字段** = 夹具上可编排的故障 / 时序字段（`mock-llm` 的场景字段、`mock-mcp` 的脚本位）。
//   正常内容形状（`chunks` / `toolCalls`）与固定端点（`mock-http` 的 `/slow` / `/missing`）
//   不是注入：前者是正常输入，后者是靶子内容，都不能「按字段分级」。
// - **能力面** = 用户能看见注入后果的那一层，不是代码目录：对话主径 / 工具回路 / 压缩归档 /
//   会话生命周期。
// - 一格 = 一个 `(kind, face)` 对，必须属于以下三类之一：
//   - `cases`：至少一个 `e2e/cases/tNN-*.mjs`，且该用例真的用了这个字段；
//   - `na`：这个字段在这条面上没有独立观测点，理由必须写清「为什么不需要重复测」；
//   - `gap`：应该有、暂时没有，`owner` 必须是存在的任务号（`T-NNN`）或设计条目（`DE-NNN`）。
//
// `na` 不是「没测」的托词：它的判据是「这条面的注入后果已由另一条面或另一层判据覆盖」，
// 写不出这句话的组合就应该落成 `gap`。

export const FACES = [
  { id: 'dialog', label: '对话主径（流式呈现 · 失败收敛 · 恢复轮）' },
  { id: 'tools', label: '工具调用与反馈回路' },
  { id: 'compression', label: '压缩与长会话归档' },
  { id: 'session', label: '会话生命周期（多轮 / 长驻 / 统计）' },
]

/** `inMock` 证明该字段真的住在对应夹具里；`usage` 用来反向扫用例。 */
export const KINDS = [
  {
    id: 'llm.status',
    label: 'provider 静态错误状态（4xx / 5xx）',
    mock: 'e2e/mock-llm.mjs',
    field: 'status',
    inMock: /\bscenario\.status\b/,
    usage: /\bstatus\s*:\s*(?:4\d\d|5\d\d)\b/,
  },
  {
    id: 'llm.failTimes',
    label: '瞬时失败（前 N 次逻辑请求打 errorStatus，之后恢复）',
    mock: 'e2e/mock-llm.mjs',
    field: 'failTimes',
    inMock: /\bscenario\.failTimes\b/,
    usage: /\bfailTimes\s*:\s*[1-9]\d*/,
  },
  {
    id: 'llm.rateLimit',
    label: 'plan 级 429 限流（滑动窗口配额）',
    mock: 'e2e/mock-llm.mjs',
    field: 'rateLimit',
    inMock: /\bplan\.rateLimit\b/,
    usage: /\brateLimit\s*:\s*\{/,
  },
  {
    id: 'llm.latencyMs',
    label: '首字节延迟',
    mock: 'e2e/mock-llm.mjs',
    field: 'latencyMs',
    inMock: /\bscenario\.latencyMs\b/,
    usage: /\blatencyMs\s*:\s*\S/,
  },
  {
    id: 'llm.chunkJitterMs',
    label: '帧间随机抖动',
    mock: 'e2e/mock-llm.mjs',
    field: 'chunkJitterMs',
    inMock: /\bscenario\.chunkJitterMs\b/,
    usage: /\bchunkJitterMs\s*:\s*\S/,
  },
  {
    id: 'llm.stallAfter',
    label: '流内停顿（不关连接）',
    mock: 'e2e/mock-llm.mjs',
    field: 'stallAfter',
    inMock: /\bscenario\.stallAfter\b/,
    usage: /\bstallAfter\s*:\s*\S/,
  },
  {
    id: 'llm.dropAfter',
    label: '流中途断开连接',
    mock: 'e2e/mock-llm.mjs',
    field: 'dropAfter',
    inMock: /\bscenario\.dropAfter\b/,
    usage: /\bdropAfter\s*:\s*\S/,
  },
  {
    id: 'llm.malformed',
    label: '不规范 SSE 帧（badJson / truncated / garbage）',
    mock: 'e2e/mock-llm.mjs',
    field: 'malformed',
    inMock: /\bscenario\.malformed\b/,
    usage: /\bmalformed\s*:\s*\S/,
  },
  {
    id: 'mcp.tool_error',
    label: '工具调用回 JSON-RPC 错误',
    mock: 'e2e/mock-mcp.mjs',
    field: 'onCall.error',
    inMock: /\bplan\.error\b/,
    usage: /mock-actions\.json|mock 注入的工具执行失败/,
  },
  {
    id: 'mcp.disconnect',
    label: '工具调用中服务进程断连',
    mock: 'e2e/mock-mcp.mjs',
    field: 'onCall.disconnect',
    inMock: /\bplan\.disconnect\b/,
    usage: /\bdisconnect\s*:\s*true/,
  },
  {
    id: 'mcp.initialize_failure',
    label: 'MCP 握手失败（initialize 回 error）',
    mock: 'e2e/mock-mcp.mjs',
    field: 'failInitialize',
    inMock: /\bscript\.failInitialize\b/,
    usage: /\bfailInitialize\s*:\s*true/,
  },
  {
    id: 'mcp.tool_delay',
    label: '工具执行延迟（onCall.sleepMs）',
    mock: 'e2e/mock-mcp.mjs',
    field: 'onCall.sleepMs',
    inMock: /\bplan\.sleepMs\b/,
    usage: /\bsleepMs\s*:\s*\S/,
  },
]

/**
 * 48 格（12 × 4）。`cases` 里的编号只写 `tNN`：审计把它解析到 `e2e/cases/tNN-*.mjs`，
 * 解析不到即红。
 */
export const CELLS = [
  // ── dialog ──────────────────────────────────────────────────────────
  { kind: 'llm.status', face: 'dialog', cases: ['t5', 't53'] },
  { kind: 'llm.failTimes', face: 'dialog', na: '瞬时失败的重试语义由 `llm.status` 的多轮失败用例覆盖；`failTimes` 的独有形态是「同一会话第 N 轮坏」，落在 session 面（t47）' },
  { kind: 'llm.rateLimit', face: 'dialog', cases: ['t50'] },
  { kind: 'llm.latencyMs', face: 'dialog', cases: ['t52'] },
  { kind: 'llm.chunkJitterMs', face: 'dialog', cases: ['t52'] },
  { kind: 'llm.stallAfter', face: 'dialog', cases: ['t52'] },
  { kind: 'llm.dropAfter', face: 'dialog', cases: ['t49'] },
  { kind: 'llm.malformed', face: 'dialog', cases: ['t51'] },
  { kind: 'mcp.tool_error', face: 'dialog', na: '工具错误以工具反馈进入对话，形态在 tools 面钉住（t6 / t62）；对话面不另设 MCP 注入' },
  { kind: 'mcp.disconnect', face: 'dialog', na: 'MCP 客户端只有工具面一个消费方；断连的后果经工具反馈投影进对话，不构成对话面独有形态' },
  { kind: 'mcp.initialize_failure', face: 'dialog', na: '握手失败发生在插件装配期，对话链路看不到独立的连接状态' },
  { kind: 'mcp.tool_delay', face: 'dialog', na: '工具延迟不改变对话契约形状；超时语义归 tools 面的熔断判据' },

  // ── tools ───────────────────────────────────────────────────────────
  { kind: 'llm.status', face: 'tools', na: '工具轮的 provider 错误与对话轮共用同一失败通道；工具面独有的是 MCP 错误与断流重发' },
  { kind: 'llm.failTimes', face: 'tools', na: '工具面用 `dropAfter` 钉断流重发的槽位（t54），不依赖瞬时 5xx 的计数语义' },
  { kind: 'llm.rateLimit', face: 'tools', na: '限流按 provider 入口的请求计数，与能力面正交；t50 在 `DIALOG_FACE_OFF` 下只让主对话轮对表' },
  { kind: 'llm.latencyMs', face: 'tools', na: '首字节延迟是流控时序，不改变工具契约；t52 已钉客户端对延迟的容忍' },
  { kind: 'llm.chunkJitterMs', face: 'tools', na: '帧抖动只影响流内节奏，拼接正确性由 t52 逐字断言覆盖' },
  { kind: 'llm.stallAfter', face: 'tools', na: '停顿不关连接，空闲超时判据在客户端单点；t52 已钉' },
  { kind: 'llm.dropAfter', face: 'tools', cases: ['t54'] },
  { kind: 'llm.malformed', face: 'tools', na: '脏帧由客户端解析层单点处理；t51 已覆盖三种形态，工具面不重复注入' },
  { kind: 'mcp.tool_error', face: 'tools', cases: ['t6', 't62'] },
  {
    kind: 'mcp.disconnect',
    face: 'tools',
    gap: {
      owner: 'DE-039',
      reason: 'mock-mcp 已支持 `onCall.disconnect`，但外部执行 / 熔断的上游接线在 DE-039 阶段；RE-005 已判定接线前不补用例，先补会测到 mock 的想象行为',
    },
  },
  {
    kind: 'mcp.initialize_failure',
    face: 'tools',
    gap: {
      owner: 'DE-039',
      reason: '握手失败后的插件降级路径同样属于外部执行接线；mock-mcp 已支持 `failInitialize`，待接线批次配用例',
    },
  },
  {
    kind: 'mcp.tool_delay',
    face: 'tools',
    gap: {
      owner: 'DE-039',
      reason: '工具挂死的超时 / 熔断由 DE-039 接管；mock-mcp 的 `onCall.sleepMs` 已具备编排能力，接线前不写判据',
    },
  },

  // ── compression ─────────────────────────────────────────────────────
  { kind: 'llm.status', face: 'compression', cases: ['t18'] },
  { kind: 'llm.failTimes', face: 'compression', na: '压缩失败的静态故障由 `llm.status` / t18 覆盖；t18 注释提到的 `failTimes` 只解释计数口径，未使用该字段' },
  { kind: 'llm.rateLimit', face: 'compression', na: '配额语义与能力面正交；t50 已钉限流后的冷却恢复，压缩请求共用同一客户端' },
  { kind: 'llm.latencyMs', face: 'compression', na: '首字节延迟不改变压缩节点契约；流式预览的时序由 t18 的场景编排钉住' },
  { kind: 'llm.chunkJitterMs', face: 'compression', na: '帧抖动只影响节奏，摘要拼接正确性已由 t18 逐字断言' },
  { kind: 'llm.stallAfter', face: 'compression', na: '停顿的容忍由客户端单点判；压缩面无独立空闲超时' },
  { kind: 'llm.dropAfter', face: 'compression', na: '压缩请求的断流走同一客户端重试；t18 钉的是失败纪律与重试恢复，未单独编排中途断连' },
  { kind: 'llm.malformed', face: 'compression', na: '脏帧在解析层统一处理；t51 已覆盖，摘要面不重复注入' },
  { kind: 'mcp.tool_error', face: 'compression', na: '压缩轮不调用 MCP 工具（压缩是 provider 请求 + VDFS 归档），没有 MCP 连接' },
  { kind: 'mcp.disconnect', face: 'compression', na: '压缩面不持有 MCP 连接' },
  { kind: 'mcp.initialize_failure', face: 'compression', na: '压缩面不持有 MCP 连接' },
  { kind: 'mcp.tool_delay', face: 'compression', na: '压缩面不持有 MCP 连接' },

  // ── session ─────────────────────────────────────────────────────────
  { kind: 'llm.status', face: 'session', cases: ['t28'] },
  { kind: 'llm.failTimes', face: 'session', cases: ['t47'] },
  { kind: 'llm.rateLimit', face: 'session', na: '限流状态不进入会话账本；t50 已从用户可见面钉「被限又自己缓过来」' },
  { kind: 'llm.latencyMs', face: 'session', na: '延迟不改变会话账本；长驻稳定性由 t47 覆盖' },
  { kind: 'llm.chunkJitterMs', face: 'session', na: '抖动不改变会话账本；长驻稳定性由 t47 覆盖' },
  { kind: 'llm.stallAfter', face: 'session', na: '停顿不改变会话账本；超时收敛由 t52 覆盖' },
  { kind: 'llm.dropAfter', face: 'session', na: '断流后的续轮由 t49 ④（恢复轮）覆盖；会话生命周期面没有独有的断流形态' },
  { kind: 'llm.malformed', face: 'session', na: '脏帧在解析层统一处理；t51 已覆盖，会话面无独立形态' },
  { kind: 'mcp.tool_error', face: 'session', na: 'MCP 错误经工具反馈投影进转写；t6 的断言已从生产读出口取会话终态' },
  { kind: 'mcp.disconnect', face: 'session', na: 'MCP 客户端只在工具面；会话账本只记录结果，不持有连接' },
  { kind: 'mcp.initialize_failure', face: 'session', na: '握手失败发生在插件装配期，不进入会话账本' },
  { kind: 'mcp.tool_delay', face: 'session', na: '工具延迟不改变会话账本' },
]

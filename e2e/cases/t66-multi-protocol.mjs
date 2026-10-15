import './_selfrun.mjs';
// T66 多协议适配：**同一条用户链路**（正文流式拼接 + 工具调用回路 + 转写收敛）在四个
// `api_protocol` 下各跑一遍。
//
// ## 为什么是一条用例、不是四条
//
// 协议是同一个被测链路的**参数**：四套适配器共用 session / 工具执行 / 转写落盘，
// 差异只在「请求怎么拼、SSE 怎么解」。按参数展开（同一个 `for` 循环换 `api_protocol`）
// 比复制四份文件更能暴露「只有一家对」的漂移——共用断言，一家的形状错就红一行，
// 而不是四条各自维护、慢慢长歪。
//
// ## 判据只取用户可见出口
//
// - **CLI 正文**（stdout 拼接结果）；
// - **mock 侧请求体**（按协议形状取 tools 面与工具结果）；
// - **转写读出口**（`readSessionMessages` → 不变量 + tool_call 终态 + 正文完整）。
//
// 不读会话内部存储、不看协议实现细节——四协议下问的是同一件事：换协议后链路还通不通。
//
// ## 边界（本用例不做什么）
//
// 不验四家的**差异面**（各自特有字段 / 错误形状 / 多模态块），那是各协议单测的地盘；
// 不做真实 provider 形状对照，那是 `e2e/ab/`（门外）的职责。
import { join } from 'node:path';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  nextPort,
  startReadbackCli,
  readSessionMessages,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  textOf,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/**
 * 四协议各自的 id 与**请求路径**（与 model 插件的 `resolve_protocol_id` 同词；
 * 路径由各协议 `get_api_url` 拼出，见 `mock-llm.mjs::resolveFormat`）。
 *
 * gemini 的路径是 `{api_base}/models/{model}:streamGenerateContent`（**无 query**，
 * key 走 `x-goog-api-key` 头）——不是文档里曾写的 `?alt=sse&key=…`。
 */
const PROTOCOLS = [
  { id: 'openai_chat', path: '/v1/chat/completions' },
  { id: 'openai_responses', path: '/v1/responses' },
  { id: 'anthropic_messages', path: '/v1/messages' },
  { id: 'gemini_api', path: '/v1/models/mock-model:streamGenerateContent' },
];

/** 收尾正文分片——验「流式拼接」：stdout 必须是按序拼回的整串。 */
const FINAL_CHUNKS = ['工具调用完成', '，', '回显成功。'];
const FINAL_TEXT = FINAL_CHUNKS.join('');
const ECHO_TEXT = 'mock 回显内容';

/** 同一份场景表——由 mock 按路径翻译成各协议的线上形状（见 `mock-llm::buildFrames`）。 */
function scenarios() {
  return [
    {
      id: 'call-echo',
      match: '帮我回显',
      toolCalls: [{ id: 'call_1', name: 'mcp__mockserv__echo', arguments: { text: ECHO_TEXT } }],
    },
    { id: 'after-echo', afterTool: true, chunks: FINAL_CHUNKS },
  ];
}

/** 按协议从请求体取**工具面**（LLM 可见的工具名列表）。 */
function toolNamesOf(protocol, body) {
  if (protocol === 'gemini_api') {
    return (body.tools ?? []).flatMap((t) => (t.functionDeclarations ?? []).map((d) => d.name));
  }
  if (protocol === 'openai_chat') return (body.tools ?? []).map((t) => t.function?.name);
  // openai_responses / anthropic_messages：tools[] 顶层直接是 `{ name, ... }`
  return (body.tools ?? []).map((t) => t.name);
}

/** 按协议从请求体取**工具结果正文**（判「结果真的回灌给模型了没有」）。 */
function toolResultTextOf(protocol, body) {
  if (protocol === 'openai_responses') {
    const item = (body.input ?? []).find((it) => it.type === 'function_call_output');
    return item ? String(item.output ?? '') : '';
  }
  if (protocol === 'gemini_api') {
    for (const c of body.contents ?? []) {
      for (const p of c.parts ?? []) {
        if (p.functionResponse) return JSON.stringify(p.functionResponse.response ?? '');
      }
    }
    return '';
  }
  if (protocol === 'anthropic_messages') {
    for (const m of body.messages ?? []) {
      for (const blk of Array.isArray(m.content) ? m.content : []) {
        if (blk.type === 'tool_result') return String(blk.content ?? '');
      }
    }
    return '';
  }
  // openai_chat
  const tool = (body.messages ?? []).find((m) => m.role === 'tool');
  return tool ? String(tool.content ?? '') : '';
}

export default defineCase('T66 多协议适配：同一用户链路在四协议下各跑一遍', async () => {
  for (const proto of PROTOCOLS) {
    const label = `[${proto.id}]`;
    const sid = `e2e-t66-${proto.id}`;
    const recordPath = join('mcp-record.ndjson');
    const llm = await new MockLlm(scenarios()).start();
    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port, { api_protocol: proto.id }) }],
      pluginConfigs: {
        // 转写经生产读出口读（gateway `invoke` → `vdfs/read`），入站口必须开着。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        session: { ...DIALOG_FACE_OFF },
      },
    });
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, recordPath)],
      enabled: true,
    });
    let api = null;
    try {
      const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '帮我回显一下', provider: PROVIDER_ID, session: sid });
      assertEq(r.code, 0, `${label} CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      assert(
        r.stdout.includes(FINAL_TEXT),
        `${label} stdout 应为收尾正文流式拼接结果（实际: ${JSON.stringify(r.stdout)}）`,
      );

      // 证据①：mock 收到**两次**请求（工具轮 + 收尾轮），两次都打在**本协议的路径**上，
      // 且 mock 侧按路径识别出的线格式就是本协议——路径与线格式一一对应。
      const reqs = await llm.requests();
      assertEq(reqs.length, 2, `${label} mock-llm 应收到两次请求（工具轮 + 收尾轮）`);
      assert(
        reqs.every((q) => q.path === proto.path),
        `${label} 请求路径应为本协议路径（实际: ${reqs.map((q) => q.path).join(',')}）`,
      );
      assert(
        reqs.every((q) => q.format === proto.id),
        `${label} mock 侧识别的线格式应为 ${proto.id}（实际: ${reqs.map((q) => q.format).join(',')}）`,
      );

      // 证据②：工具面——目标工具在 LLM 可见的工具列表里（按协议形状取）。
      const toolNames = toolNamesOf(proto.id, reqs[0].body);
      assert(
        toolNames.includes('mcp__mockserv__echo'),
        `${label} 请求 tools 应含目标工具（实际: ${toolNames.join(',')}）`,
      );

      // 证据③：工具真执行（mock-mcp 收到调用）+ 结果真回灌（第二轮请求携带结果正文）。
      const record = readFileSyncSafe(join(hd.homedir, recordPath));
      const calls = record.split('\n').filter(Boolean).map((l) => JSON.parse(l)).filter((e) => e.kind === 'call');
      assertEq(calls.length, 1, `${label} mock-mcp 应恰好收到一次 tools/call`);
      assertEq(calls[0].args, { text: ECHO_TEXT }, `${label} 工具参数`);
      const resultText = toolResultTextOf(proto.id, reqs[1].body);
      assert(
        resultText.includes(ECHO_TEXT),
        `${label} 第二轮请求应携带工具结果正文（实际: ${JSON.stringify(resultText).slice(0, 300)}）`,
      );

      // 证据④：转写收敛（生产读出口）——不变量 + tool_call 完成 + 正文完整。
      api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: sid,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      const msgs = await readSessionMessages(api, sid);
      assertTranscriptInvariants(msgs, `T66${label}`);
      const tc = msgs.find((m) => m.type === 'tool_call');
      assert(tc, `${label} 应有 tool_call 节点`);
      assertEq(tc.status, 'completed', `${label} tool_call 终态应为 completed`);
      const text = msgs.find((m) => m.role === 'assistant' && m.type === 'text');
      assert(text, `${label} 应有 assistant text 节点`);
      assert(textOf(text).includes(FINAL_TEXT), `${label} 落盘正文应为完整收尾文本`);
    } finally {
      api?.stop();
      llm.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: `T66${label} 读回 CLI / mock-llm 子进程退出`,
      });
    }
  }
});

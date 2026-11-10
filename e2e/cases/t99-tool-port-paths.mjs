import './_selfrun.mjs';
// T99 tool 端口三路径：装 / 不装 / 缺失。
//
// 「装」由既有电池守着（t2 / t3 / t17 的每一次工具调用都经 `port.tool` 的执行合同），
// 本例钉后两条：
//
// | 路径 | 装配 | 判据 |
// |---|---|---|
// | 不装 | 停用 local / web / mcp / skill / agent（用户面的工具贡献插件） | 纯对话照常跑完；工具面失去它们的工具、底座（vdfs）工具仍在 |
// | 缺失 | 同一装配，mock 强发一次对停用工具（`web_search`）的调用 | 响亮失败：错误文本落成工具结果子节点（meta.success=false）、父节点终态 completed，模型第二轮**看到错误**并照常收尾 |
//
// ## 为什么「缺失」不是「端口行缺席」
//
// `port.tool` 的行由 session 在能力收集后按「表里有工具行」装（零工具 ⇒ 不装行 ⇒
// 环在调用点响亮失败）。但**能对话的装配不可能零工具**：对话输入本身走
// `vdfs/write` 收件箱，vdfs 是不可停用的底座，而它无条件贡献 9 个 `vdfs_*` 工具——
// 停了它就没有任何入口能再发起一轮。于是 e2e 里可达的「缺失」形态是**调用的工具
// 缺失**：线上名解析失败不猜、按原名交给路由拿诚实的 NotFound（端口合同 B1 的收口
// 分支）。「零工具 ⇒ 端口行缺席」的响亮失败由单测钉——loop 的
// `tool_executor.test.rs::a_missing_tool_port_fails_loudly_on_the_result` 可以构造
// 「表里有工具但不装端口行」的装配，e2e 构造不出来。
//
// 读法（用户如何看到就如何读）：工具面与「模型看到错误」走 mock 的请求留痕
// （`llm.requests()`），转写经生产读出口（`vdfs/read`）；停用走插件目录
// `PLUGIN.yml` 的 `plugin_enabled: false`（与 t17 / t90 同一条装配位）。
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  startLongLivedCli,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readSessionMessages,
  assertTranscriptInvariants,
  msgRoleOf,
  msgTypeOf,
} from '../helpers.mjs';

/** 被测会话 id（REPL 进程自己的会话另用中立 id，不污染本会话的转写）。 */
const SID = 'e2e-t99';

/** B 线（不装）的发言；C 线（缺失）的发言让 mock 强发停用工具的调用。 */
const MSG_PLAIN = '纯对话（T99 不装线）';
const MSG_FORCED = '强制调用（T99 缺失线）';
const REPLY_PLAIN = '纯对话照常（T99）';
const REPLY_AFTER = '工具缺失已如实回报，本轮收尾（T99）';

/** 停用后应从工具面消失的代表工具（local / web / skill / agent 各取代表）。 */
const GONE_TOOLS = [
  'web_search', 'web_fetch', 'http_request', // web
  'content_search', 'todo_write', 'ask_user', // local
  'read_skill', // skill
  'agent_run', // agent
];

const gatewayConfig = (port) => ({
  inbound_enabled: true,
  inbound_protocol: 'http',
  inbound_bind: '127.0.0.1',
  inbound_port: port,
  inbound_token: '',
  inbound_readonly: false,
});

export default defineCase(
  'T99 tool 端口三路径：不装（停工具插件）纯对话照常；缺失（强发停用工具）响亮失败并落事实',
  async () => {
    const llm = await new MockLlm([
      { id: 'plain', match: '纯对话', content: REPLY_PLAIN },
      {
        id: 'forced',
        match: '强制调用',
        toolCalls: [{ id: 'call_t99', name: 'web_search', arguments: { query: 'symbio-t99' } }],
      },
      { id: 'after-forced', afterTool: true, content: REPLY_AFTER },
    ]).start();
    const gatewayPort = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: gatewayConfig(gatewayPort),
        ...dialogFaceOff(),
        // 不装：用户面的工具贡献插件全部停用（mcp 未配 server、无可观测工具，照停
        // 保持装配形状与裁决清单一致）。vdfs 是底座，不停——理由见文件头。
        local: { plugin_enabled: false },
        web: { plugin_enabled: false },
        mcp: { plugin_enabled: false },
        skill: { plugin_enabled: false },
        agent: { plugin_enabled: false },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t99-live',
      provider: PROVIDER_ID,
      gatewayPort,
    });
    try {
      await cli.waitGatewayReady(20_000, 'T99');

      // ── 会话与发言口（收件箱地址面，与前端同一条入口）──────────────────
      const rootResp = await cli.invoke('vdfs/root', {});
      const root = rootResp.body?.data?.path;
      assert(
        typeof root === 'string' && root.length > 0,
        `vdfs/root 应返回根地址（${JSON.stringify(rootResp.body)?.slice(0, 200)}）`,
      );
      const addr = `${root.replace(/\/+$/, '')}/session/${SID}`;
      const created = await cli.invoke(
        'vdfs/write',
        {
          path: addr,
          create: true,
          text: JSON.stringify({
            metadata: { workdir: hd.workdir, mode: 'auto', risk_level: 'medium', provider_id: PROVIDER_ID },
          }),
        },
        { session_id: SID },
      );
      assertEq(created.status, 200, `创建会话（${JSON.stringify(created.body)?.slice(0, 300)}）`);

      const say = async (text) => {
        const w = await cli.invoke('vdfs/write', { path: `${addr}/inbox`, text }, { session_id: SID });
        assertEq(w.status, 200, `写收件箱「${text}」（${JSON.stringify(w.body)?.slice(0, 300)}）`);
      };
      const transcript = async () => (await readSessionMessages(cli, SID)) ?? [];
      const assistantTexts = async () =>
        (await transcript())
          .filter((m) => msgRoleOf(m) === 'assistant')
          .map((m) => String(m.content ?? ''));
      /** 末条 user 发言含 marker 的那次模型请求（工具面 / 错误回传的读数口）。 */
      const requestOfTurn = async (marker) => {
        const reqs = await llm.requests();
        return (
          reqs.find((r) =>
            (r.body.messages ?? [])
              .filter((m) => m.role === 'user')
              .some((m) => String(m.content ?? '').includes(marker)),
          ) ?? null
        );
      };

      // ── B 线（不装）：纯对话照常，工具面失去停用插件的工具 ─────────────
      await say(MSG_PLAIN);
      await waitFor(async () => (await assistantTexts()).some((t) => t.includes(REPLY_PLAIN)), {
        what: '不装线的纯对话跑完',
        timeoutMs: 30_000,
      });
      // 纯对话：这一轮没有任何工具调用（缺失线的判据才允许出现 tool_call）。
      assert(
        (await transcript()).every((m) => msgTypeOf(m) !== 'tool_call'),
        '不装线的纯对话不该出现任何 tool_call 节点',
      );

      const plainReq = await requestOfTurn('纯对话');
      assert(plainReq !== null, '不装线应留下模型请求（工具面读数）');
      const tools = (plainReq.body.tools ?? []).map((t) => t.function?.name).filter(Boolean);
      for (const gone of GONE_TOOLS) {
        assert(!tools.includes(gone), `停用工具插件后 ${gone} 不该在工具面（实际: ${tools.join(',')}）`);
      }
      // 下限防白绿：工具面整体变空意味着别的插件也丢了，而不是「停用生效」。
      assert(
        tools.includes('vdfs_write'),
        `底座（vdfs）工具应仍在工具面（实际: ${tools.join(',')}）`,
      );

      // ── C 线（缺失）：强发停用工具的调用 ⇒ 响亮失败并落事实 ────────────
      await say(MSG_FORCED);
      await waitFor(async () => (await assistantTexts()).some((t) => t.includes(REPLY_AFTER)), {
        what: '缺失线的第二轮收尾（模型看到错误后照常结束）',
        timeoutMs: 30_000,
      });

      const msgs = await transcript();
      assertTranscriptInvariants(msgs, 'T99');

      // 落事实 ①：父 ToolCall 节点终态——completed（失败属信息性，不暂停会话）。
      // 持久化视图（`vdfs/read`）里父节点不带合并的 success/failure_kind：
      // 执行器广播的父终态快照带合并 meta，但落库镜像按 `into_messages` 重建
      // 原始节点（迁移前后逐字同形，t6 同族先例）；「父终态快照带合并 meta」
      // 由 loop 单测钉（`tool_executor.test.rs` 的终态断言）。
      const tc = msgs.find((m) => msgTypeOf(m) === 'tool_call');
      assert(tc, '缺失线应有 tool_call 节点（强发的调用落进了转写）');
      assertEq(tc.meta?.tool_name, 'web_search', 'tool_call 记录被调用的工具名');
      assertEq(tc.status, 'completed', 'tool_call 终态应为 completed');

      // 落事实 ②：结果子节点承载错误正文与 success=false——指名 web_search 的
      // 诚实 NotFound（解析失败不猜、按原名交路由），不是「判不出来」的含糊话；
      // 失败是事实（meta.success=false），不是没发生。
      const result = msgs.find((x) => x.parent_id === tc.id && msgRoleOf(x) === 'tool');
      assert(result, 'tool_call 应有 role=tool 的结果子节点（有请求必有响应）');
      assertEq(result.meta?.success, false, '结果子节点应记 success=false（响亮失败，不是静默）');
      const resultText = String(result.content ?? '');
      assert(
        resultText.includes('web_search') && resultText.includes('Error'),
        `结果子节点应指名 web_search 的错误正文（实际: ${JSON.stringify(resultText.slice(0, 200))}）`,
      );

      // 落事实 ③：错误文本回传给模型——强发轮之后的下一次请求携带它作为
      // tool 消息（同一发言标记的**第二次**请求：第一次是强发轮本身，还没有
      // 工具结果），模型据此收尾（失败属信息性）而不是流程无声中断。
      const reqs = (await llm.requests()).filter((r) =>
        (r.body.messages ?? [])
          .filter((m) => m.role === 'user')
          .some((m) => String(m.content ?? '').includes('强制调用')),
      );
      assert(reqs.length >= 2, `强发发言应产生两次模型请求（实际 ${reqs.length}）`);
      const toolMsgs = (reqs.at(-1).body.messages ?? []).filter((m) => m.role === 'tool');
      assert(
        toolMsgs.some((m) => String(m.content ?? '').includes('web_search')),
        `工具结果轮的请求应携带指名 web_search 的工具错误（实际末条: ${JSON.stringify(
          toolMsgs.at(-1)?.content ?? null,
        )?.slice(0, 200)}）`,
      );

      // 系统完整运行：两线跑完进程仍在（不是崩溃后被判绿）。
      assertEq(cli.proc.exitCode, null, 'CLI 进程在整条用例里应始终在跑');
    } finally {
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

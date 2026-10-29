import './_selfrun.mjs';
// T27 审批与恢复：工具停在等用户 → 回填答案 → 续写**同一轮**并收束。
//
// ## 本用例钉的是什么
//
// 出口判据：「`confirm` 工具 → 本轮停在等用户 → 回填答案 → 续跑并收束」。
//
// 这条判据之所以必须端到端，是因为它要证的三件事**各自只能在一处被看见**：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 工具**真的停在等用户**（本轮没收束） | 转写（读出口）里 tool_call 之下挂着 `user_prompt`（WaitingUserAction）节点 |
// | 恢复**续写同一轮**（不重开用户格） | 读出口读回的历史里那句用户发言仍只有一条 |
// | 回填的答案**真的回了模型** | mock-llm 的 `/_requests` 回读**第二次请求体** |
//
// 单测（`actors/mod.test.rs::resume_continues_same_turn_without_reopening_user_cell`）用
// 假 provider + 假分发覆盖了同一形状，但它证不了"真实 provider + 真实工具分发 +
// 真实落盘 + 真实恢复入口"这条链——单测里的 provider / dispatch 都是假件。
//
// ## 为什么工具用 `ask_user`（而不是 `confirm`）
//
// 判据里的 `confirm` 是**场景代称**（"一个会停在等用户的工具"），不是工具名——
// 本仓没有名为 `confirm` 的工具。真正的 pending 触发点是 `ask_user`：interactive
// 模式下它返回 `failure_kind = needs_interaction` + `prompt` 载荷，编排层据此构造
// `user_prompt`（`WaitingUserAction`）节点、运行器停止工具循环且**不收束**。
//
// ## 为什么 mode 必须是 interactive
//
// `ask_user` 的 auto 分支**不产节点**（返回 `tool_unavailable`，让模型继续）——
// auto 档下本用例根本到不了"等用户"。故两处请求都显式带 `mode: 'interactive'`。
//
// ## 恢复入口为什么是 gateway 直发节点动作
//
// CLI 把 `resume: None` 写死（`crates/symbio-cli/src/client.rs`），**没有恢复入口**。
// 这里取「e2e 直发」形态：经 gateway HTTP 边界（与 Tauri 前端同构的
// `/api/v1/invoke`）发一条恢复请求。
//
// 入口形状（[D-066]）：恢复走**节点动作**——
// `vdfs/action(<sid>/message/<target_id>, "answer", { answer })`。好处是不动
// 生产代码，且验的正是**前端恢复走的同一条路**（`vdfs_provider` 的 resume 臂 →
// `start_turn` 直连）；落点是目标消息自身（恢复就该「落在当时那条消息上」，
// 入队会让它排到新消息之后，候选早被改写）。
//
// ## 为什么工具调用场景要 `once: true`（与 T26 同因）
//
// 本用例不声明 `afterTool` 场景，而 mock 的 `afterTool` 是**优先池**而非硬过滤——
// 池子选不出就退回全池。且本用例的恢复轮 prompt 里**仍含**首轮用户那句话（转写投影
// 照实渲染），带 `match` 的工具场景会在工具结果轮**再次命中** ⇒ 无限工具循环。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` / `compose`）默认开启，会给每一轮多加一次静默请求。本用例按
// **精确请求数**（2 次）写断言，且验的是审批/恢复链路——与对话面正交，故钉死。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  readSessionMessages,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  resumeSession,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t27';

/** 触发 pending 的工具名（本仓真正的「停在等用户」触发点）。 */
const PENDING_TOOL = 'ask_user';

/** 提问与答案：答案必须从恢复入口一路走到第二次请求体。 */
const QUESTION = '你选哪一项';
const ANSWER = { choice: 'A' };

export default defineCase(
  'T27 审批与恢复：工具停等用户 → 回填答案 → 续写同一轮并收束',
  async () => {
    const llm = await new MockLlm([
      // 第一次请求：模型请求 `ask_user`（带一段中途正文，逼出「定格并切节点」路径）。
      // `once: true` —— 恢复轮的 prompt 仍含首轮那句话，不带 `once` 会反复命中（见文件头）。
      {
        id: 'ask',
        match: '帮我确认',
        once: true,
        content: '我先问你一下。',
        toolCalls: [
          {
            id: 'call_ask',
            name: PENDING_TOOL,
            arguments: {
              question: QUESTION,
              options: [{ label: 'A' }, { label: 'B' }],
            },
          },
        ],
      },
      // 第二次请求（恢复续写的收尾轮）：不再请求工具 ⇒ 本轮收束。
      { id: 'after-answer', content: '已收到你的选择。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 无此文件时网关回退默认配置（`inbound_enabled: false`）——监听不会被拉起，
        // 恢复入口就打不到（与 T7 / T28 同一约定）。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 本用例的主题与对话面正交 ⇒ 把三个开关钉死（理由见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF },
      },
    });

    try {
      // ── 第一幕：工具停在等用户（本轮不收束，提问节点在转写里等着）────────────
      const first = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我确认一下',
        provider: PROVIDER_ID,
        session: SID,
        // interactive：`ask_user` 只有 interactive 档才产 pending 节点（见文件头）。
        mode: 'interactive',
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 600)}）`);

      // 读回 CLI：转写与恢复入口都经它（`vdfs/read` / `vdfs/action`，与前端同构）。
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        mode: 'interactive',
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        // ── 证据 ①：会话面看得见提问节点（tool_call + user_prompt/WaitingUserAction）——
        // 恢复入口要的 `target_id` 就是这里的 tool_call 节点 id。
        const msgs0 = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs0, 'T27 等待轮');
        const tc = msgs0.find((m) => msgTypeOf(m) === 'tool_call');
        assert(tc, `应有 tool_call 节点（实得类型: ${msgs0.map((m) => msgTypeOf(m)).join(',')}）`);
        const promptNode = msgs0.find(
          (m) =>
            m.parent_id === tc.id &&
            msgTypeOf(m) === 'user_prompt' &&
            m.status === 'waiting_user_action',
        );
        assert(
          promptNode,
          `tool_call 之下应有 user_prompt（WaitingUserAction）节点（实得: ${JSON.stringify(
            msgs0.filter((m) => m.parent_id === tc.id).map((m) => ({ type: m.type, status: m.status })),
          )}）`,
        );

        // ── 第二幕：回填答案（恢复入口 = `action(<sid>/message/<target_id>, "answer")`）──
        //
        // 落点是提问所在的 tool_call 节点 id（稳定锚点）；`mode` 是**调用级**运行
        // 选项，只有会话 metadata 里没有时才由它兜底（D-066 决策 2）。
        const inv = await resumeSession(
          api,
          SID,
          tc.id,
          'answer',
          { answer: ANSWER, mode: 'interactive' },
          { session_id: SID },
        );
        assert(
          inv.status === 200 && inv.body?.data?.ok === true,
          `恢复请求应被受理（实际: ${JSON.stringify(inv.body)}）`,
        );

        // 恢复是 spawn 执行（`start_turn` 立即返回 accepted）⇒ 轮询**请求数**等这一轮
        // 真的打了一次模型（用户可见面：mock 收到的第二个请求体就是恢复收尾轮）。
        await waitFor(async () => (await llm.requests()).length >= 2, {
          what: '恢复轮收束（mock 收到第二次请求）',
          timeoutMs: 30_000,
        });

        // ── 证据 ②：续写**同一轮**——读出口读回的历史里，那句用户发言仍只有一条 ──
        // （重开用户格 ⇒ 用户会看到自己的问题被复制了一遍）
        const msgs1 = await readSessionMessages(api, SID);
        const askCount = msgs1.filter(
          (m) => msgRoleOf(m) === 'user' && String(m.content ?? '').includes('帮我确认一下'),
        ).length;
        assertEq(
          askCount,
          1,
          `续写**不重开用户格**——同一句话在历史里只能出现一次（实得 ${askCount} 条）`,
        );
        assert(
          msgs1.some(
            (m) => msgRoleOf(m) === 'assistant' && String(m.content ?? '').includes('已收到你的选择。'),
          ),
          '恢复轮的收束正文应出现在历史里（续写轮真的收束了）',
        );

        // ── 证据 ③：答案回了模型（只能靠回读请求体证明）─────────────────────────
        //
        // ⚠ 断言必须打在**未转义的 prompt 正文**上：请求体是标准 OpenAI 包
        // （`{model, messages:[{role,content}], ...}`），若对 `JSON.stringify(body)`
        // 搜 `"choice"`，prompt 里的引号已被二次转义成 `\"` ⇒ 假红（本用例踩过）。
        const promptOf = (body) =>
          (body?.messages ?? [])
            .map((m) => (typeof m.content === 'string' ? m.content : ''))
            .join('\n');
        const reqs = await llm.requests();
        assertEq(reqs.length, 2, 'mock-llm 应收到两次请求（提问轮 + 恢复收尾轮）');
        // ⚠️ 判据在**消息层**上：验「调用在」「结果在」「结果是 `tool` 角色」（D-048a）
        // ——而不是「某个字符串出现过」。
        const secondMsgs = reqs[1].body.messages ?? [];
        const second = promptOf(reqs[1].body);
        const calls = secondMsgs.flatMap((m) => m.tool_calls ?? []);
        assert(
          calls.some((c) => c.function?.name === PENDING_TOOL),
          `恢复轮请求应带上被恢复的工具调用（实际: ${JSON.stringify(calls).slice(0, 400)}）`,
        );
        const toolResults = secondMsgs.filter((m) => m.role === 'tool');
        assert(
          toolResults.some((m) => String(m.content ?? '').includes('"choice"')),
          `恢复轮请求应带上恢复结果，且是 role=tool 消息（实际: ${JSON.stringify(toolResults).slice(0, 400)}）`,
        );
        assert(second.includes('"choice"'), '回填的答案键（choice）应原样回灌给模型');
        assert(second.includes('"A"'), '回填的答案值（A）应原样回灌给模型');
        // 反向：第一次请求里不该有恢复结果（那时还没恢复）。
        assert(
          (reqs[0].body.messages ?? []).every((m) => m.role !== 'tool'),
          '第一次请求不该出现 role=tool 消息（答案尚未回填）',
        );

        // ── 转写不变量：恢复后节点仍成对、seq 严格递增 ───────────────────────
        assertTranscriptInvariants(msgs1, 'T27 恢复后');
        const tc1 = msgs1.find((m) => msgTypeOf(m) === 'tool_call');
        assert(
          msgs1.some((m) => msgRoleOf(m) === 'tool' && m.parent_id === tc1.id),
          '恢复后 tool_call 仍须有结果子节点',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净：Windows 上带着活子进程 `process.exit()` 会撞
      // libuv 的 `uv_async_send` 断言，表现是**断言全过、退出码却是 0xC0000409**。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

import './_selfrun.mjs';
// T48 megabatch 守卫的**反馈语义**：单响应 33 个工具调用 → 超限的丢弃并反馈 →
// 模型下一轮从工具结果里读到「超限了、该分批发起」→ 整轮正常收束。
//
// ## 背景（`6cf9520d` 号会话事故）
//
// 真实模型在一条响应内流式输出了 56 个工具调用（同批 9 个复读 6 遍），批执行器
// 等流结束才开工 ⇒ 60 余秒零结果，用户视角即「工具永远不闭合的死循环」。守卫：
// 单响应上限 32（判在流层 `model/stream.rs`），处置是**丢弃 + 反馈**——
// 整轮 `Err` 的旧方案让模型对发生了什么一无所知（流程无故中断），已否决。
//
// ## 本用例钉的是什么
//
// | 断言 | 唯一能看见它的地方 |
// |---|---|
// | 超限不炸轮：33 调用的响应照常走完（CLI 退出 0、收束正文在） | CLI stdout / 退出码 |
// | 33 个 tool_call 全部终态、33 个结果子节点成对（S20.1 不变量） | 生产读出口 `readSessionMessages` |
// | 被丢弃的调用带 `overflow_dropped` 成因 + 反馈正文（不混进 not_executed） | 同上（meta + content） |
// | **模型看得见反馈**：下一次请求的工具结果里带着超限提示 | mock-llm `/_requests` 回读 |
//
// 前 32 个调用真实执行（内置 `vdfs_list`，只读、快）；第 33 个被丢弃。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  startReadbackCli,
  readSessionMessages,
  DIALOG_FACE_OFF,
  nextPort,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
} from '../helpers.mjs';

const SID = 'e2e-t48';
const CAP = 32; // 与 `MAX_TOOL_CALLS_PER_RESPONSE` 同值（守卫语义的对照锚）
const TOTAL = CAP + 1; // 33 个调用：前 32 执行，第 33 丢弃

export default defineCase(
  'T48 megabatch 反馈语义：33 调用不炸轮，超限调用丢弃并反馈给模型',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'megabatch',
        match: '发起风暴',
        once: true,
        content: '我发起一批调用。',
        toolCalls: Array.from({ length: TOTAL }, (_, i) => ({
          id: `call_${i}`,
          name: 'vdfs_list',
          arguments: { path: '.' },
        })),
      },
      // 工具结果轮（末条 role=tool）：不再发起调用 ⇒ 本轮收束。
      { id: 'after-storm', content: '收到工具结果，收束。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 断言按请求轮廓写，与对话面正交 ⇒ 钉死（理由见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF },
      },
    });

    try {
      // ── 断言 ①：超限不炸轮 ────────────────────────────────────────────
      // 旧方案（整轮 Err）在这里表现为 CLI 非零退出 / Failed 轮。
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '发起风暴：一口气列出目录',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);
      assert(
        r.stdout.includes('收到工具结果，收束'),
        `stdout 应为收束正文（实际: ${JSON.stringify(r.stdout.slice(0, 300))}）`,
      );

      // ── 断言 ②：转写里 33 个 tool_call 与结果成对，全部终态 ────────────
      const api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T48');
        const toolCalls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
        assertEq(toolCalls.length, TOTAL, `转写应有 ${TOTAL} 个 tool_call 节点`);
        assert(
          toolCalls.every((m) => m.status === 'completed'),
          '所有 tool_call（含被丢弃的）都必须收在终态，不留永远转圈的「运行中」',
        );
        const results = msgs.filter((m) => m.role === 'tool');
        assertEq(results.length, TOTAL, '有调用必有结果：33 个结果子节点');

        // ── 断言 ③：丢弃调用的成因与反馈正文 ────────────────────────────
        const dropped = results.filter((m) => String(m.content ?? '').includes('超过上限'));
        assertEq(dropped.length, 1, '恰好 1 个结果携带超限反馈（第 33 个调用）');
        assert(
          results.every((m) => m.parent_id != null),
          '丢弃结果同样挂在它的 tool_call 之下（有请求、无响应 = 前端缺口）',
        );
        const droppedParent = msgs.find((m) => m.id === dropped[0].parent_id);
        assertEq(
          droppedParent?.meta?.failure_kind,
          'overflow_dropped',
          '成因必须可指认（overflow_dropped），不得谎报成 not_executed',
        );
      } finally {
        api.stop();
      }

      // ── 断言 ④：模型看得见反馈（下一次请求体回读）─────────────────────
      const reqs = await llm.requests();
      assert(reqs.length >= 2, `mock-llm 至少收到 2 次请求（实际 ${reqs.length}）`);
      const feedbackReq = reqs.find((req) =>
        (req.body?.messages ?? []).some(
          (m) => m.role === 'tool' && String(m.content ?? '').includes('超过上限'),
        ),
      );
      assert(feedbackReq, '下一次请求必须携带带超限提示的 role=tool 结果（模型可见）');
      // provider 契约：assistant 消息必须带上全部 33 个 tool_call（含被丢弃的），
      // 否则下一轮请求直接 400。
      const echoedCalls = (feedbackReq.body.messages ?? []).flatMap((m) => m.tool_calls ?? []);
      assertEq(
        echoedCalls.length,
        TOTAL,
        '回灌请求必须原样携带全部 tool_call（丢弃 ≠ 删除，provider 契约）',
      );
    } finally {
      cleanupHomedir(hd);
      llm.stop();
      // 收尾必须等子进程真的退干净（与 t26 同一约定：Windows 上带活子进程退出会撞
      // libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

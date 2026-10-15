import './_selfrun.mjs';
// T63 压缩进行中用户插话不丢（T-074 清偿；LC-007c 压缩请求形状同场钉）。
//
// ## 本用例钉的是什么
//
// [RE-005](../../docs/006-ref/005-e2e覆盖矩阵.md "RE-005-e2e覆盖矩阵") 缺口表里
// 「压缩进行中用户插话」一行挂了很久的「暂不补（待评估）」——理由是「读数没有
// 生产出口可读」。本用例把那个读数立起来：**MockLLM 的请求记录就是生产出口**。
// 插话有没有被压缩吞掉 / 错位，问模型就行——下一轮请求包里它在不在、排在第几，
// 是请求体上的事实，不是 mock 想象出来的结果。
//
// 时序（压缩窗口是真实窗口，不是摆设）：
// 1. 逐轮填充推高水位，直到**观测到**摘要请求（触发轮不硬编码——按 mock 的
//    usage 口径自适应，t18 同一纪律）；
// 2. 摘要请求在飞（10 个慢分片 ≈ 5s 窗口）时写收件箱 = 插话落在压缩进行中；
// 3. 等本轮与插话轮都收束，判四件事：
//    ① 摘要请求形状（LC-007c：无工具面 + 压缩提示词 + 摘要指令标记）；
//    ② 插话被消费：出现一条**晚于**摘要请求、含插话正文的请求（不丢）；
//    ③ 插话轮照常收束、转写里顺序正确（插话 user 消息在前、回答在后，
//       与 t34 忙窗插话同形）；
//    ④ 转写不变量（有请求必有响应的落盘形态）。
//
// ## 反空转保障
//
// 判据 ① 的前置是「摘要请求真的出现过」——没有它，本用例全部断言失去对象，
// 必须当场红（压缩没触发 = 用例没测到它声称测的东西，不是「跳过」）。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readSessionMessages,
  assertTranscriptInvariants,
  textOf,
} from '../helpers.mjs';
import { assertCompressionRequestShape } from '../llm-contract.mjs';

const SID = 'e2e-t63';
const INTERJECT = '插话：压缩还没完，先回答我这个';
const FILL = '上下文填充内容用于推高历史水位。';
const REPLY = '填充：' + FILL.repeat(240); // 与 t18 同一标定（mock 恒报 prompt_tokens: 64）
const SNAPSHOT = [
  '<state_snapshot>',
  '  <overall_goal>验证压缩进行中插话不丢。</overall_goal>',
  '  <key_knowledge>',
  '    - 用户偏好：mock 环境下回复应包含轮次编号。',
  '  </key_knowledge>',
  '  <completed_items>',
  '    - 前几轮填充对话已完成。',
  '  </completed_items>',
  '  <in_progress_items>',
  '    - 等待插话被消费。',
  '  </in_progress_items>',
  '  <open_questions></open_questions>',
  '</state_snapshot>',
];

export default defineCase(
  'T63 压缩进行中插话不丢：请求包取证 + 转写顺序 + 摘要请求形状（T-074 清偿）',
  async () => {
    // 摘要场景：10 个慢分片 ⇒ 压缩进行中有一个 ~5s 的真实窗口可写插话。
    const llm = await new MockLlm([
      {
        id: 'snapshot',
        match: 'Distill the conversation above',
        chunks: SNAPSHOT,
        chunkDelayMs: 500,
      },
      { id: 'big', match: '轮问题', content: REPLY },
      { id: 'interject', match: INTERJECT, content: '插话已处理。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      // 窗口值与 t18 同一标定（0.3L 窗口 ≈ 8 轮；上限取 12400 保触发轮两侧有余量）
      providers: [
        { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 12400 }) },
      ],
      pluginConfigs: {
        session: { auto_compress: true, context_messages: 0, ...DIALOG_FACE_OFF },
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
      },
    });

    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      // REPL 会话 = 被测会话（t34 同形）：`cli.send` 喂的发言进的就是这个会话，
      // 实时面订阅与断言才看得见。用中立会话 id 只适用于「被测会话由别的入口
      // 驱动」的用例（如心跳，README 夹具陷阱 2）——本用例的驱动入口就是 REPL。
      session: SID,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    let rt = null;

    /** 摘要请求是否已到达（压缩真的发生了） */
    const hasSummary = async () =>
      (await llm.requests()).some((q) =>
        (q.body?.messages ?? []).some(
          (m) => m.role === 'user' && String(m.content ?? '').includes('Distill the conversation above'),
        ),
      );

    try {
      await cli.waitGatewayReady();
      rt = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: GATEWAY_PORT });

      /** 已收束的轮次数（实时面口径：Turn 节点终态） */
      const doneTurns = () => {
        const last = new Map();
        for (const { mid, data } of rt.messages()) {
          if (data?.type === 'turn') last.set(mid, data.status);
        }
        return [...last.values()].filter((s) =>
          ['completed', 'failed', 'aborted', 'waiting_user_action'].includes(s),
        ).length;
      };
      /** 等轮次收束数达到 n */
      const waitTurns = (n, what) =>
        waitFor(async () => (await doneTurns()) >= n, { what, timeoutMs: 60_000 });

      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const inboxAddr = `${root.replace(/\/+$/, '')}/session/${SID}/inbox`;

      // ── ① 逐轮填充，直到观测到摘要请求（触发轮自适应，不硬编码）──────────
      let compressed = false;
      for (let i = 1; i <= 12 && !compressed; i++) {
        const before = doneTurns();
        cli.send(`第${i}轮问题`);
        // 两种出口：这轮跑完（没压）或摘要请求出现在飞（压了）——后者立刻进入插话
        await waitFor(
          async () => (await hasSummary()) || (await doneTurns()) > before,
          { what: `第 ${i} 轮填充跑完或触发压缩`, timeoutMs: 60_000 },
        );
        compressed = await hasSummary();
        if (!compressed) await waitTurns(before + 1, `第 ${i} 轮收束`);
      }
      assert(compressed, '12 轮填充内应至少触发一次压缩（否则本用例测不到压缩窗口）');

      // ── ② 摘要在飞（慢分片窗口）⇒ 写插话（忙窗入队即返回，t34 同形）──────
      const w = await cli.invoke('vdfs/write', { path: inboxAddr, text: INTERJECT });
      assertEq(
        w.status,
        200,
        `压缩进行中写收件箱应成功（入队即返回）：${JSON.stringify(w.body)?.slice(0, 300)}`,
      );

      // ── ③ 等本轮收束 + 插话轮收束 ───────────────────────────────────────
      const turnsBeforeInterject = await doneTurns();
      await waitFor(
        async () =>
          (await llm.requests()).some((q) => JSON.stringify(q.body).includes(INTERJECT)),
        { what: '插话进了某一轮 LLM 请求（被消费，没被压缩吞掉）', timeoutMs: 60_000 },
      );
      await waitTurns(turnsBeforeInterject + 1, '插话轮收束');

      // ── ④ 判据：摘要请求形状（LC-007c）─────────────────────────────────
      const summaryReq = (await llm.requests()).find((q) =>
        (q.body?.messages ?? []).some(
          (m) => m.role === 'user' && String(m.content ?? '').includes('Distill the conversation above'),
        ),
      );
      assert(summaryReq, '摘要请求应在请求记录里');
      assertCompressionRequestShape(summaryReq, {}, 'T63④ 摘要请求');

      // ── ⑤ 判据：插话被消费，且消费发生在摘要请求**之后**（排队语义）─────
      const reqs = await llm.requests();
      const summaryAt = reqs.indexOf(summaryReq);
      const interjectAt = reqs.findIndex((q) => JSON.stringify(q.body).includes(INTERJECT));
      assert(
        interjectAt > summaryAt,
        `插话的消费请求应晚于摘要请求（排队等当前轮跑完；实际 summary@${summaryAt}, interject@${interjectAt}）`,
      );

      // ── ⑥ 判据：转写顺序——插话 user 消息在前、其回答在后，各一次 ────────
      const msgs = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs, 'T63');
      const interjectMsgs = msgs.filter((m) => m.role === 'user' && textOf(m).includes(INTERJECT));
      assertEq(interjectMsgs.length, 1, `插话在转写里应恰有一条 user 消息（实际 ${interjectMsgs.length}）`);
      const at = msgs.indexOf(interjectMsgs[0]);
      assert(at > 0, '插话不应是会话第一条消息（前面有填充轮）');
      // 插话之后必须有一条助手回答（插话轮照常收束，与 t34 同形）
      const after = msgs.slice(at + 1);
      assert(
        after.some((m) => m.role === 'assistant' && textOf(m).trim().length > 0),
        '插话轮应照常收束（插话之后有带正文的助手回答）',
      );
    } finally {
      rt?.close();
      cli.stop();
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

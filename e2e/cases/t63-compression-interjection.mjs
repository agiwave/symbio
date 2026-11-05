// T63 压缩进行中用户插话不丢：压缩窗口内写入的插话被消费，且消费发生在摘要问答
// **之后**（排队等压缩窗口过去）。
import './_selfrun.mjs';
//
// ## 本用例钉的是什么
//
// 压缩现在由两行债完成（D-336）：`compact-summarize`（`model.assist`）取摘要，
// `compact-write`（`compact`）落 `session.consolidated`。摘要问答仍是真实窗口
// （mock 用 10 个慢分片 ≈ 5s），插话就在这个窗口里写进收件箱。
//
// | # | 契约 | 判据 |
// |---|---|---|
// | A | 压缩窗口真的出现过 | mock 收到摘要问答（最后一条 user 是整段转写，带「用户：」前缀） |
// | B | 插话被消费，且排在摘要之后 | 某轮**主请求**的 `messages` 里出现插话正文（`role=user`、正文以插话开头），且该请求下标 > 摘要请求下标 |
// | C | 压缩真的完成 | 事实源里落了一条 `session.consolidated`（首节点 notice + style_tag=compression） |
// | D | 插话不丢 | 事实源里恰有一条 `user.message` 正文 = 插话（压缩合并它、但不抹掉它） |
//
// ### 为什么不断言「请求里 Notice 在插话之前」
//
// `Notice`（`session.consolidated`）在摘要问答**完成之后**才落格，而插话发生在窗口
// **之内**——插话请求启动时 Notice 还不存在，该顺序物理上不可达，故不作为判据。
// 排队语义只认两个可观测点：摘要问答下标在前、承载插话正文的主请求下标在后。
//
// 触发参数与 t8/t18 同一口径：mock 恒报 `prompt_tokens: 64`（见 `mock-llm.mjs`），
// 上限取 80 ⇒ 每轮越线。
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readSessionMessages,
  assertTranscriptInvariants,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

const SID = 'e2e-t63';
const INTERJECT = '插话：压缩还没完，先回答我这个';
const FILL = '上下文填充内容用于推高历史水位。';
const REPLY = '填充：' + FILL.repeat(240);
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

/** 会话事实源：live 段 + 归档段（`transcripts/wal-archive`）的并集。 */
const walEvents = (homedir, sid) => {
  const dir = join(homedir, 'session', sid);
  const archiveDir = join(dir, 'transcripts', 'wal-archive');
  const texts = [join(dir, 'v2-events.wal')];
  if (existsSync(archiveDir)) {
    for (const name of readdirSync(archiveDir)) texts.push(join(archiveDir, name));
  }
  return texts
    .map(readFileSyncSafe)
    .join('')
    .split('\n')
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
};

/** 一条请求的最后一条 user 正文。 */
const lastUserText = (q) => {
  const users = (q.body?.messages ?? []).filter((m) => m.role === 'user');
  return String(users[users.length - 1]?.content ?? '');
};

/** 主请求的 `messages` 里是否**真带**插话正文。
 *
 * 摘要请求的 messages 里也含插话（它在整段转写里、前缀「用户：」），那不算送进模型；
 * 主请求把发言原样作为一条 `role=user` 消息送上，正文以插话开头。
 */
const mainRequestHasInterject = (q) =>
  (q.body?.messages ?? []).some(
    (m) => m.role === 'user' && String(m.content ?? '').trim().startsWith(INTERJECT),
  );

export default defineCase(
  'T63 压缩进行中插话不丢：窗口内插话被消费、排在摘要问答之后',
  async () => {
    // 摘要场景：10 个慢分片 ⇒ 压缩进行中有一个 ~5s 的真实窗口可写插话。
    const llm = await new MockLlm([
      {
        id: 'snapshot',
        match: '助手：',
        chunks: SNAPSHOT,
        chunkDelayMs: 500,
      },
      { id: 'big', match: '轮问题', content: REPLY },
      { id: 'interject', match: INTERJECT, content: '插话已处理。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [
        { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 80 }) },
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
      // 实时面订阅与断言才看得见。
      session: SID,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    let rt = null;

    /** 摘要问答是否已到达（压缩真的开始了；`model.assist` 的 user 是整段转写） */
    const hasSummaryRequest = async () =>
      (await llm.requests()).some((q) => lastUserText(q).includes('用户：'));

    try {
      await cli.waitGatewayReady();
      rt = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: GATEWAY_PORT });

      /** 已收束的轮次数（终答节点到达终态；D-335：Turn 不作为可见节点） */
      const doneTurns = () => {
        const merged = new Map();
        for (const { mid, data } of rt.messages()) {
          merged.set(mid, { ...merged.get(mid), ...data });
        }
        return [...merged.values()].filter(
          (m) =>
            msgTypeOf(m) === 'text' &&
            msgRoleOf(m) === 'assistant' &&
            !m.meta?.exclude_from_context &&
            ['completed', 'failed', 'aborted', 'waiting_user_action'].includes(m.status),
        ).length;
      };
      const waitTurns = (n, what) =>
        waitFor(async () => doneTurns() >= n, { what, timeoutMs: 60_000 });

      const root = (await cli.invoke('vdfs/root', {})).body?.data?.path;
      assert(typeof root === 'string' && root.length > 0, 'vdfs/root 应返回根地址');
      const inboxAddr = `${root.replace(/\/+$/, '')}/session/${SID}/inbox`;

      // ── ① 逐轮填充，直到观测到摘要问答在飞（触发轮自适应，不硬编码）──────────
      let compressed = false;
      for (let i = 1; i <= 12 && !compressed; i++) {
        const before = doneTurns();
        cli.send(`第${i}轮问题`);
        // 两种出口：这轮跑完（没压）或摘要问答出现在飞（压了）——后者立刻进入插话
        await waitFor(
          async () => (await hasSummaryRequest()) || (await doneTurns()) > before,
          { what: `第 ${i} 轮填充跑完或触发压缩`, timeoutMs: 60_000 },
        );
        compressed = await hasSummaryRequest();
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

      // ── ③ 等插话被消费 + 压缩落格 ───────────────────────────────────────
      const turnsBeforeInterject = await doneTurns();
      await waitFor(
        async () => (await llm.requests()).some(mainRequestHasInterject),
        { what: '插话进了某一轮主请求的 messages（被消费，没被压缩吞掉）', timeoutMs: 60_000 },
      );
      await waitFor(
        () => walEvents(hd.homedir, SID).some((e) => e.kind === 'session.consolidated'),
        { what: 'session.consolidated 落格', timeoutMs: 30_000 },
      );
      await waitTurns(turnsBeforeInterject + 1, '插话轮收束');

      // ── ④ 判据：压缩真的完成，且载荷是压缩 Notice ───────────────────────
      const events = walEvents(hd.homedir, SID);
      const consolidated = events.filter((e) => e.kind === 'session.consolidated');
      assert(consolidated.length >= 1, '应至少落一条 session.consolidated');
      const nodes = consolidated[0].payload?.messages ?? [];
      assertEq(nodes[0]?.type, 'notice', '压缩快照首节点应是 notice');
      assertEq(nodes[0]?.meta?.style_tag, 'compression', '压缩快照首节点的 style_tag');
      assert(
        String(nodes[0]?.content ?? '').includes('state_snapshot'),
        `压缩快照正文应是摘要（实际首 60 字: ${String(nodes[0]?.content ?? '').slice(0, 60)}）`,
      );

      // ── ⑤ 判据：插话被消费，且消费发生在摘要问答**之后**（排队语义）──────
      // 光有下标序 + 事实条数会漏「收进去了但没送进模型」：这里断言**主请求的
      // messages 真带插话正文**（摘要请求里那段转写不算——它前缀是「用户：」）。
      const reqs = await llm.requests();
      const summaryAt = reqs.findIndex((q) => lastUserText(q).includes('用户：'));
      assert(summaryAt >= 0, '摘要问答应在请求记录里');
      const interjectAt = reqs.findIndex(mainRequestHasInterject);
      assert(
        interjectAt >= 0,
        `插话正文必须真出现在某一轮主请求的 messages 里（实际请求数 ${reqs.length}）`,
      );
      assert(
        interjectAt > summaryAt,
        `插话的消费请求应晚于摘要问答（排队等压缩窗口过去；实际 summary@${summaryAt}, interject@${interjectAt}）`,
      );

      // ── ⑥ 判据：插话没被丢弃、也没被重放（事实源 live + 归档并集恰好一条）──
      const allEvents = walEvents(hd.homedir, SID);
      const interjectFacts = allEvents.filter(
        (e) => e.kind === 'user.message' && String(e.payload?.text ?? '') === INTERJECT,
      );
      assertEq(
        interjectFacts.length,
        1,
        `插话在事实源里应恰有一条 user.message（实际 ${interjectFacts.length}）`,
      );
      // 读出口不变量仍然成立（压缩合并了插话所在的前缀，但不破坏落盘形态）
      const msgs = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs, 'T63');
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

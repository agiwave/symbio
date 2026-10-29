import './_selfrun.mjs';
// T71 对等主体跑一轮（相应批次 合并批 ②+③，
// [D-065](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范") / [D-067](../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")）：
// **收束格按种类判** + **入站「来源主体」格数据**。
//
// ## 本用例钉的是什么
//
// - [D-065](../../docs/003-design/011-Agent目录规范.md "D-065") 的判据原文：「e2e（两会话两主体，
//   对等主体跑一轮 ⇒ 它的 `commitment.*` 入格、请求包不串主体，`t30` 纪律复用）」；
// - [D-067](../../docs/003-design/011-Agent目录规范.md "D-067") 的三处下游口径：
//   C4 认产物（`invariants` 为空）、`closure_denied` 数产物（派生事实照写）、
//   发言闸拒 ≠ 静默中断。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 对等主体真的跑了一轮（主体 = 登记窄行 `session/peer-b`） | 事实源 `actor` / 声誉列逐字段 |
// | 产物收束成立：发言格被拒、轮照样了结 | WAL 无 `chat.assistant.final` 而有 `artifact.added`；`invariants` 为空 |
// | 来源主体是**数据**：`to` = 请求 metadata 声明的那个主体 | `commitment.opened` 的 `from` / `to` |
// | 派生事实照写（`closure_denied` 数产物） | 同上——收束族被跳过就一格都不会有 |
// | 请求包不串主体（t30 纪律） | mock-llm `/_requests` 回读逐条正文 |
//
// ## 场景
//
// 主会话（`session/main`）正常聊一轮；对等会话（metadata 绑 `peer-b` ⇒ 主体
// `session/peer-b`）经**地址面**收到一条带 `metadata.source_principal = 'session/main'`
// 的收件箱消息。对等体的模型调 `agent_run` 委托一个**不存在**的智能体（工具失败
// ⇒ 立约 + 违约，不需要真的派生子会话），结果回灌后模型给出汇总——发言闸拒它
// （窄行无 `reply.*`），但产物格已落 ⇒ 轮以**产物**收束，轮末承诺族入格：
// `from = session/peer-b`（跑轮的主体）、`to = session/main`（消息的来源主体）。
import {
  MockLlm,
  makeHomedir,
  makeAgentDir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  createSession,
  sendInbox,
  readSessionMessages,
  waitFor,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';
import { join } from 'node:path';
import { existsSync } from 'node:fs';

/** 主会话（无 `--agent` ⇒ 主体 `session/main`）——来源主体的**出处**。 */
const MAIN_SID = 'e2e-t71-main';
/** 对等会话（metadata 绑 `peer-b` ⇒ 主体 `session/peer-b`，授权表登记窄行）。 */
const PEER_SID = 'e2e-t71-peer';

/** 对等智能体的人格——它只出现在**对等会话**的请求包里，是「不串」的正向探针。 */
const PERSONA_PEER = '你是协作对等体：只做工具调用，不做寒暄。';

const MAIN_Q = '主会话第一问：请回答。';
const PEER_Q = '对等体请把这件事委托出去并汇报。';
/** 对等体收尾轮的汇总正文（发言闸拒 ⇒ 这句话**不会**出现在任何格子里）。 */
const PEER_SUMMARY = '对等体已汇总。';

/** v2 事实源文件名（与 `crates/symbio-plugin-session/src/paths.rs::V2_WAL_FILE` 同一常量口径）。 */
const V2_WAL = 'v2-events.wal';

/** 读一份 WAL 文件的行（逐行 JSON）。 */
function walLines(path) {
  return readFileSyncSafe(path)
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

/** 请求体里最后一条 user 消息（与 mock-llm 的匹配口径同形）。 */
function lastUserText(r) {
  const msgs = r.body?.messages ?? [];
  const last = [...msgs].reverse().find((m) => msgRoleOf(m) === 'user');
  if (typeof last?.content === 'string') return last.content;
  if (Array.isArray(last?.content)) return last.content.map((p) => p.text ?? '').join('');
  return '';
}

/**
 * 跨会话召回**小节**头（`memory_events::prompt_section` 的固定排版首行）。
 *
 * 对等体跑轮时主会话的发言可能已被编码进长期记忆并经召回段注入——那是**已声明**
 * 的背景事实通道（t30 同款边界），不算串主体；**线程**里出现才算。
 */
const RECALL_SECTION_HEAD = '【长期记忆】跨会话召回';

/** 把一条消息的 content 切成 `{ thread, recall }` 两半；无召回小节 ⇒ `null`。 */
function splitRecall(content) {
  if (typeof content !== 'string') return null;
  const at = content.indexOf(RECALL_SECTION_HEAD);
  if (at < 0) return null;
  const lines = content.slice(at).split('\n');
  const recallLines = [lines[0]];
  for (const line of lines.slice(1)) {
    if (line.startsWith('- ')) recallLines.push(line);
    else break;
  }
  const recall = recallLines.join('\n');
  return { thread: content.slice(0, at) + content.slice(at + recall.length), recall };
}

/** 摘掉召回段之后的请求正文 = **这个会话自己的线程** + 系统提示词 + 工具表。 */
function threadBody(r) {
  const msgs = (r.body?.messages ?? [])
    .filter((m) => m.meta?.kind !== 'recall_context')
    .map((m) => {
      if (typeof m.content !== 'string') return m;
      const split = splitRecall(m.content);
      if (split && split.thread.trim() === '') return null;
      return split ? { ...m, content: split.thread } : m;
    })
    .filter(Boolean);
  return JSON.stringify({ ...r.body, messages: msgs });
}

export default defineCase(
  'T71 对等主体跑一轮：产物收束 + 来源主体格数据（commitment 的 from/to），请求包不串主体',
  async () => {
    const llm = await new MockLlm([
      // 主会话一轮：正常收束（发言主体照常落 final 格——对照组）
      { id: 'main', match: MAIN_Q, once: true, content: '主会话的回答。' },
      // 对等体第一发：真跑一次 `agent_run`，委托一个**不存在**的智能体
      // ⇒ 工具失败 ⇒ 立约 + 违约 + 宣告（不需要真的派生子会话）
      {
        id: 'peer-tool',
        match: PEER_Q,
        once: true,
        toolCalls: [
          {
            id: 'call_1',
            name: 'agent_run',
            arguments: { agent_id: 'ghost-agent', prompt: '把这件事办了' },
          },
        ],
      },
      // 对等体第二发（工具结果回灌；last user 仍是 PEER_Q，`peer-tool` 已用过）
      { id: 'peer-final', content: PEER_SUMMARY },
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
        // 每轮收束把事实写进事实源（actor = 会话主体）。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    // 对等智能体目录：会话绑定 `peer-b` 时能力收集要求它存在（与 t30 同一硬约束）。
    makeAgentDir(hd.homedir, {
      id: 'peer-b',
      name: '对等体',
      persona: PERSONA_PEER,
      providerId: PROVIDER_ID,
      providerPort: llm.port,
    });

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };
    const peerWalPath = join(hd.homedir, 'session', PEER_SID, V2_WAL);
    const mainWalPath = join(hd.homedir, 'session', MAIN_SID, V2_WAL);

    try {
      // ── ① 主会话一轮（对照：发言主体照常收束）────────────────────────────
      const main = runCli({ ...base, session: MAIN_SID, message: MAIN_Q });
      assertEq(main.code, 0, `主会话一轮退出码（stderr: ${main.stderr.slice(0, 400)}）`);

      // ── ② 对等会话：经地址面创建（metadata 绑 peer-b），收一条**带来源主体**的消息
      const api = startLongLivedCli({
        ...base,
        session: MAIN_SID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();
        await createSession(api, PEER_SID, {
          agent_id: 'peer-b',
          workdir: hd.workdir,
          mode: 'auto',
          risk_level: 'medium',
        });
        const sent = await sendInbox(api, PEER_SID, PEER_Q, {
          source_principal: 'session/main',
        });
        assertEq(
          sent.status,
          200,
          `写对等收件箱应成功（${JSON.stringify(sent.body)?.slice(0, 300)}）`,
        );

        // ── ③ 等对等轮收尾：承诺族入格 = 收束族执行过 = 产物收束成立 ─────────
        await waitFor(() => readFileSyncSafe(peerWalPath).includes('commitment.opened'), {
          what: '对等轮的承诺族入格（产物收束 ⇒ 派生事实照写）',
          timeoutMs: 30_000,
        });

        // ── ④ 事实源逐格断言（对等会话）────────────────────────────────────
        const peerEvents = walLines(peerWalPath);
        const kinds = peerEvents.map((e) => e.kind);

        const userCell = peerEvents.find((e) => e.kind === 'user.message');
        assert(userCell, `对等轮应有开轮格（实际 kinds: ${JSON.stringify(kinds)}）`);
        assertEq(userCell.turn, 0, '对等轮 = turn 0');
        assert(
          JSON.stringify(userCell.payload).includes(PEER_Q),
          '开轮格的正文 = 收件箱那条消息',
        );

        // 产物收束（D-065 / D-067）：产物格在、actor = 对等主体、溯源指回开轮格。
        const artifact = peerEvents.find((e) => e.kind === 'artifact.added');
        assert(artifact, `产物格必须存在（实际 kinds: ${JSON.stringify(kinds)}）`);
        assertEq(artifact.actor, 'session/peer-b', '产物格的作者 = 对等主体');
        assertEq(artifact.turn, 0, '产物格记在本轮');
        assertEq(
          artifact.produced_by,
          userCell.seq,
          '产物格溯源指向本轮开口格（I2）',
        );

        // 发言收束**被闸拒**的形态：无 final 也无 fallback（fail-closed 的结构可见）。
        assert(
          !kinds.includes('chat.assistant.final') && !kinds.includes('chat.assistant.fallback'),
          `对等主体无发言权 ⇒ 不落发言格（实际 kinds: ${JSON.stringify(kinds)}）`,
        );

        // 承诺族：from = 跑轮主体，to = **来源主体**（请求 metadata 声明的数据）。
        const offered = peerEvents.find((e) => e.kind === 'commitment.opened');
        assert(offered, '违约路径也要先立约（opened → broken → asserted）');
        assertEq(offered.actor, 'session/peer-b', '承诺格的作者 = 对等主体');
        assertEq(
          offered.payload?.from,
          'session/peer-b',
          '承诺方 = 跑轮的主体（声誉记在它头上）',
        );
        assertEq(
          offered.payload?.to,
          'session/main',
          '承诺对象 = 来源主体（metadata.source_principal 声明的数据，不是写死的常量）',
        );
        assertEq(offered.produced_by, userCell.seq, '立约锚在本轮开口格（I2）');
        const broken = peerEvents.find((e) => e.kind === 'commitment.broken');
        assert(broken, '工具失败 ⇒ 违约必须成格（DE-038 §5：违约可被观测）');
        const declared = peerEvents.find((e) => e.kind === 'commitment.asserted');
        assert(declared, '违约宣告必须成格');
        assert(
          JSON.stringify(declared.payload).includes('session/main'),
          `宣告文案「已告知 <to>」带上来源主体（实际: ${JSON.stringify(declared.payload)?.slice(0, 200)}）`,
        );

        // ── ⑤ 生产出口读数：invariants 为空（C4 认产物）+ 声誉列逐字段 ──────
        const inv = await api.invoke('session/stats', {}, { session_id: PEER_SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const stats = inv.body?.data;
        assert(stats, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assertEq(
          stats.invariants.length,
          0,
          `产物收束 ⇒ 不变量清单为空（C4 认产物格，D-067；实际: ${JSON.stringify(stats.invariants)}）`,
        );
        const peerRep = stats.reputation.by_principal['session/peer-b'];
        assert(peerRep, `声誉列应有对等主体的条目（实际: ${JSON.stringify(stats.reputation)}）`);
        assertEq(peerRep.principal, 'session/peer-b');
        assertEq(peerRep.offered, 1, '立约 1 次（真实委托算出来的）');
        assertEq(peerRep.kept, 0, '违约 ⇒ kept 0');
        assertEq(peerRep.broken, 1, '违约 1 次');
        assertEq(peerRep.score, -1, '平凡打分 = 守约 − 违约');

        // ── ⑥ 对照：主会话照常以发言收束（actor = session/main 的 final 格在）──
        assert(existsSync(mainWalPath), `主会话事实源应在：${mainWalPath}`);
        const mainEvents = walLines(mainWalPath);
        const mainFinal = mainEvents.find((e) => e.kind === 'chat.assistant.final');
        assert(mainFinal, '主会话（发言主体）照常落 final 格');
        assertEq(mainFinal.actor, 'session/main', '主会话的作者 = 主智能体');

        // ── ⑦ 请求包不串主体（t30 纪律：线程与召回分通道）────────────────────
        const reqs = await llm.requests();
        const mainReqs = reqs.filter((r) => lastUserText(r).includes(MAIN_Q));
        const peerReqs = reqs.filter((r) => lastUserText(r).includes(PEER_Q));
        assertEq(mainReqs.length, 1, `主会话恰好一发请求（实际 ${mainReqs.length}）`);
        assertEq(
          peerReqs.length,
          2,
          `对等会话恰好两发（工具轮 + 结果回灌；实际 ${peerReqs.length}）`,
        );
        // 主会话：不带对等体的人格；线程里没有对等体的正文。
        for (const r of mainReqs) {
          assert(
            !JSON.stringify(r.body).includes(PERSONA_PEER),
            '主会话请求包不应带对等体人格',
          );
          const at = threadBody(r).indexOf(PEER_Q);
          assert(
            at < 0,
            `主会话**线程**带进了对等体的正文：…${threadBody(r).slice(Math.max(0, at - 200), at + 200)}…`,
          );
        }
        // 对等会话：带它自己的人格；线程里没有主会话的正文（召回段是声明通道，不算）。
        for (const r of peerReqs) {
          assert(
            JSON.stringify(r.body).includes(PERSONA_PEER),
            '对等会话请求包应带对等体人格（绑定 peer-b 的请求包）',
          );
          const at = threadBody(r).indexOf(MAIN_Q);
          assert(
            at < 0,
            `对等会话**线程**带进了主会话的正文：…${threadBody(r).slice(Math.max(0, at - 200), at + 200)}…`,
          );
        }

        // ── ⑧ 读出口：对等会话的转写（用户格 + 工具卡，**没有**助手正文）────
        const peerMsgs = await readSessionMessages(api, PEER_SID);
        assertTranscriptInvariants(peerMsgs, 'T71 对等会话');
        assert(
          peerMsgs.some((m) => msgRoleOf(m) === 'user'),
          '对等会话的转写应有那条收件箱消息',
        );
        assert(
          peerMsgs.some((m) => msgRoleOf(m) === 'tool'),
          '对等会话的转写应有工具结果（产物格的投影）',
        );
        assert(
          !peerMsgs.some((m) => msgRoleOf(m) === 'assistant' && JSON.stringify(m).includes(PEER_SUMMARY)),
          '发言闸拒 ⇒ 汇总正文不入格、不出现在转写里（fail-closed 的呈现面）',
        );
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t28 / t30 同一约定）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

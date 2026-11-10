import './_selfrun.mjs';
// T105 同一轮两次提问：消息面必须恰好两张 tool_call 卡（D-382 的出口判据）。
//
// ## 本用例钉的是什么
//
// 「同一轮两次 `ask_user` 跑完后，消息面恰好两张 `tool_call` 卡（`call_ask_1` /
// `call_ask_2`），结果各挂对应卡、顺序与 `call_id` 排序一致、没有挂在轮节点下的
// `user_prompt`」。
//
// ## 为什么必须端到端
//
// 缺口在「事实落了、投影没读」这一层：单测可以手搭事件网格，但它证不了**第二次提问
// 真的由一次恢复续写产生、并且两条投影（实时转写 / 落库重建）在同一份生产装配下
// 同形**。本用例走真实 provider（mock 的 HTTP 边界）、真实工具分发（`ask_user` 的
// `needs_interaction`）、真实恢复入口（gateway 的节点动作）与真实落盘 WAL。
//
// ## 为什么用两次恢复而不是一轮里塞两次发言
//
// 「同一轮第二次提问」的定义就是：第一次待答 → 回填 → **续写同一轮** → 模型在同一轮
// 里第二次请求 `ask_user`。续写不重开用户格（T27 已钉），所以第二次提问只能靠
// 工具卡区分——这正是本用例要证的形状。
//
// ## 红了也别去读侧补
//
// 断言直接读生产出口（`vdfs/read` 的会话叶子）。若卡缺失，修法在出生处（`calls[]`
// 逐一出生），不是在这里去重 / 排序 / 补一张。
import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  startReadbackCli,
  waitFor,
  readSessionMessages,
  readFileSyncSafe,
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
const SID = 'e2e-t105';

/** 触发 pending 的工具名（与 T27 同一触发点）。 */
const PENDING_TOOL = 'ask_user';

/** 两条提问与各自答案：答案要分别挂在对应卡下。 */
const QUESTION_1 = '第一个问题';
const QUESTION_2 = '第二个问题';
const ANSWER_1 = { pick: 'A' };
const ANSWER_2 = { pick: 'D' };

/** 收集 homedir 下全部 `*.wal` 里的事实（与 helpers 的 `walFiles` 同一深度约定）。 */
function readWalEvents(homedir) {
  const out = [];
  const walk = (dir, depth) => {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = join(dir, e.name);
      if (e.isDirectory()) {
        if (depth > 0) walk(p, depth - 1);
      } else if (e.name.endsWith('.wal')) {
        for (const line of readFileSyncSafe(p).split('\n')) {
          if (!line.trim()) continue;
          try {
            out.push(JSON.parse(line));
          } catch {
            // 段头 / 残行不是事实，跳过（与 helpers 的口径一致）
          }
        }
      }
    }
  };
  walk(homedir, 5);
  return out;
}

/** 事实侧的读数：两条 `tool.requested` 的 `calls[]` 与两条待答的 `artifact.added`。 */
function factsReadout(events) {
  return {
    toolRequested: events
      .filter((e) => e.kind === 'tool.requested')
      .map((e) => ({
        root_id: e.payload?.root_id,
        calls: (e.payload?.calls ?? []).map((c) => ({ id: c.id, node_id: c.node_id })),
      })),
    artifacts: events
      .filter((e) => e.kind === 'artifact.added')
      .map((e) => ({
        tool_call_id: e.payload?.tool_call_id,
        needs_user_action: Boolean(e.payload?.needs_user_action),
      })),
  };
}

/** 消息面的形状读数（失败文案要能直接回答「缺的是哪张卡 / 谁挂在轮节点下」）。 */
function shapeReadout(msgs) {
  return {
    cards: msgs
      .filter((m) => msgTypeOf(m) === 'tool_call')
      .map((m) => ({
        id: m.id,
        wire: m.tool_call_id ?? m.attributes?.tool_call_id ?? null,
        parent_id: m.parent_id,
        children: msgs.filter((x) => x.parent_id === m.id).map((x) => msgTypeOf(x)),
      })),
    prompts: msgs
      .filter((m) => msgTypeOf(m) === 'user_prompt')
      .map((m) => ({ id: m.id, parent_id: m.parent_id, status: m.status })),
  };
}

export default defineCase(
  'T105 同轮两次提问：消息面恰好两张 tool_call 卡、按 call_id 排序、结果各挂对应卡',
  async () => {
    const llm = await new MockLlm([
      // 第一次请求：模型请求 `ask_user`。
      {
        id: 'ask1',
        match: '帮我确认两件事',
        once: true,
        content: '我先问第一个。',
        toolCalls: [
          {
            id: 'call_ask_1',
            name: PENDING_TOOL,
            arguments: {
              question: QUESTION_1,
              options: [{ label: 'A' }, { label: 'B' }],
            },
          },
        ],
      },
      // 第二次请求（恢复续写同一轮）：模型在同一轮里再次请求 `ask_user`。
      // `afterTool` 让它在「工具结果已回灌」的那一池里被选中；`once` 防止反复命中。
      {
        id: 'ask2',
        afterTool: true,
        once: true,
        content: '我再问第二个。',
        toolCalls: [
          {
            id: 'call_ask_2',
            name: PENDING_TOOL,
            arguments: {
              question: QUESTION_2,
              options: [{ label: 'C' }, { label: 'D' }],
            },
          },
        ],
      },
      // 第三次请求（第二次恢复的收尾轮）：不再请求工具 ⇒ 本轮收束。
      { id: 'final', afterTool: true, content: '两个答案都收到。' },
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
        // 与对话面正交 ⇒ 钉死三个开关（理由见 `DIALOG_FACE_OFF`）。
        ...dialogFaceOff(),
      },
    });

    try {
      // ── 第一幕：第一次提问（本轮不收束）──────────────────────────────────────
      const first = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我确认两件事',
        provider: PROVIDER_ID,
        session: SID,
        mode: 'interactive',
      });
      assertEq(first.code, 0, `第一轮 CLI 退出码（stderr: ${first.stderr.slice(0, 600)}）`);

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

        const msgs0 = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs0, 'T105 第一次等待轮');
        const cards0 = msgs0.filter((m) => msgTypeOf(m) === 'tool_call');
        assertEq(cards0.length, 1, `第一次等待轮应恰好一张 tool_call 卡（实得 ${cards0.length}）`);
        const card1 = cards0[0];
        assert(
          msgs0.some(
            (m) => m.parent_id === card1.id && msgTypeOf(m) === 'user_prompt' && m.status === 'waiting_user_action',
          ),
          `第一张卡下应有待答 prompt（实得 ${JSON.stringify(shapeReadout(msgs0))}）`,
        );

        // ── 第二幕：回填第一个答案，续写同一轮 ⇒ 模型第二次请求 `ask_user` ────────
        const inv1 = await resumeSession(
          api,
          SID,
          card1.id,
          'answer',
          { answer: ANSWER_1, mode: 'interactive' },
          { session_id: SID },
        );
        assert(
          inv1.status === 200 && inv1.body?.data?.ok === true,
          `第一次恢复应被受理（实际: ${JSON.stringify(inv1.body)}）`,
        );

        // 先等第二次模型请求真的发生（恢复续写同一轮），再等第二个提问节点入面。
        await waitFor(async () => (await llm.requests()).length >= 2, {
          what: '第一次恢复后的第二次模型请求',
          timeoutMs: 30_000,
        });

        // 等到第二次提问的**事实**落格（`artifact.added` 指向 `call_ask_2` 且待答），
        // 再等它的投影进入消息面。判据是**正向可见**（事实到了、节点到了），
        // 不是靠休眠赌时序。
        await waitFor(
          () =>
            factsReadout(readWalEvents(hd.homedir)).artifacts.some(
              (a) => a.tool_call_id === 'call_ask_2' && a.needs_user_action,
            ),
          { what: '第二次提问的 artifact.added 事实落格', timeoutMs: 30_000 },
        );
        await waitFor(
          async () => {
            const msgs = await readSessionMessages(api, SID);
            // 第二张待答卡的节点 id 由调用 id 派生（`{call_id}-result`，DE-048 D-312）；
            // 它在消息面可见即投影已到，**不看它挂在谁下面**——那正是要断言的形状。
            return (msgs ?? []).some(
              (m) => m.id === 'call_ask_2-result' && m.status === 'waiting_user_action',
            );
          },
          { what: '第二次提问节点进入消息面', timeoutMs: 30_000 },
        );

        // ── 证据 ①：事实侧齐全（两条 tool.requested 的 calls[]）──────────────────
        const facts1 = factsReadout(readWalEvents(hd.homedir));
        assertEq(
          facts1.toolRequested.length,
          2,
          `事实侧应有两条 tool.requested（实得 ${JSON.stringify(facts1.toolRequested)}）`,
        );
        const wireIds = facts1.toolRequested.flatMap((r) => r.calls.map((c) => c.id));
        assert(
          wireIds.includes('call_ask_1') && wireIds.includes('call_ask_2'),
          `tool.requested.calls[] 应逐一带着两条调用（实得 ${JSON.stringify(wireIds)}）`,
        );
        assertEq(
          new Set(facts1.toolRequested.map((r) => r.root_id)).size,
          1,
          `两次提问应属同一 request_id/root_id（实得 ${JSON.stringify(facts1.toolRequested)}）`,
        );

        // ── 证据 ②：消息面两张卡 —— 本用例的靶子 ────────────────────────────────
        const msgs1 = await readSessionMessages(api, SID);
        const readout1 = shapeReadout(msgs1);
        assertEq(
          readout1.cards.length,
          2,
          `第二次提问进入消息面后应恰好两张 tool_call 卡（实得 ${JSON.stringify(readout1)}；事实 ${JSON.stringify(facts1)}）`,
        );
        assert(
          new Set(readout1.cards.map((c) => c.id)).size === 2,
          `两张卡应是不同节点（实得 ${JSON.stringify(readout1.cards)}）`,
        );

        // 卡 ↔ wire id 的对应来自事实侧（`tool.requested.calls[]` 逐一给出
        // `{id, node_id}`）；消息叶子不序列化 `tool_call_id`（`#[serde(skip)]`），
        // 不能拿它当判据。
        const callOfNode = (nodeId) =>
          facts1.toolRequested.flatMap((r) => r.calls).find((c) => c.node_id === nodeId)?.id;
        const card1At = readout1.cards.find((c) => callOfNode(c.id) === 'call_ask_1');
        const card2At = readout1.cards.find((c) => callOfNode(c.id) === 'call_ask_2');
        assert(card1At, `应按 calls[] 出第一张卡（call_ask_1；实得 ${JSON.stringify(readout1.cards)}）`);
        assert(card2At, `应按 calls[] 出第二张卡（call_ask_2；实得 ${JSON.stringify(readout1.cards)}）`);

        // 第一张卡此时已回填：子节点是答好的结果；第二张卡下面才是待答 prompt。
        const waiting = readout1.prompts.filter((p) => p.status === 'waiting_user_action');
        assertEq(
          waiting.length,
          1,
          `此刻应恰好一条待答 prompt（实得 ${JSON.stringify(readout1)}）`,
        );
        assertEq(
          waiting[0].parent_id,
          card2At.id,
          `第二次提问的 prompt 必须挂在第二张卡下（实得 ${JSON.stringify(readout1)}）`,
        );
        const userMsg = msgs1.find(
          (m) => msgRoleOf(m) === 'user' && String(m.content ?? '').includes('帮我确认两件事'),
        );
        assert(userMsg, '用户发言应在消息面');
        assert(
          !readout1.prompts.some((p) => p.parent_id === userMsg.id),
          `user_prompt 不得直接挂在轮节点（${userMsg.id}）下（实得 ${JSON.stringify(readout1)}）`,
        );

        // 顺序与 call_id 排序一致：calls[] 的 wire id 升序 = 卡的出现顺序。
        const wires = readout1.cards.map((c) => callOfNode(c.id));
        assert(
          wires.every((w) => typeof w === 'string' && w.length > 0),
          `两张卡都应能由 calls[] 定位 wire id（实得 ${JSON.stringify(readout1.cards)}）`,
        );
        assertEq(
          JSON.stringify(wires),
          JSON.stringify([...wires].sort()),
          `卡的顺序应与 call_id 排序一致（实得 ${JSON.stringify(wires)}）`,
        );

        // ── 第三幕：回填第二个答案（锚点是第二张卡），本轮收束 ────────────────────
        const card2 = card2At;
        const inv2 = await resumeSession(
          api,
          SID,
          card2.id,
          'answer',
          { answer: ANSWER_2, mode: 'interactive' },
          { session_id: SID },
        );
        assert(
          inv2.status === 200 && inv2.body?.data?.ok === true,
          `第二次恢复应被受理（实际: ${JSON.stringify(inv2.body)}）；靶子卡: ${card2.id}`,
        );

        await waitFor(async () => (await llm.requests()).length >= 3, {
          what: '第二次恢复的收尾轮（mock 收到第三次请求）',
          timeoutMs: 30_000,
        });
        await waitFor(
          async () => {
            const msgs = await readSessionMessages(api, SID);
            return (msgs ?? []).some(
              (m) => msgRoleOf(m) === 'assistant' && String(m.content ?? '').includes('两个答案都收到。'),
            );
          },
          { what: '收尾正文进入消息面', timeoutMs: 30_000 },
        );

        // ── 证据 ③：收束后的最终形状（结果各挂对应卡）────────────────────────────
        const msgs2 = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs2, 'T105 收束后');
        const readout2 = shapeReadout(msgs2);
        assertEq(
          readout2.cards.length,
          2,
          `收束后仍应恰好两张卡、无第三张（实得 ${JSON.stringify(readout2)}）`,
        );
        for (const card of readout2.cards) {
          const results = msgs2.filter((m) => m.parent_id === card.id && msgRoleOf(m) === 'tool');
          assertEq(
            results.length,
            1,
            `卡 ${card.id}（wire=${card.wire}）下应恰好一条工具结果（实得 ${JSON.stringify(
              msgs2.filter((m) => m.parent_id === card.id).map((m) => ({ t: msgTypeOf(m), r: msgRoleOf(m) })),
            )}）`,
          );
        }
        const finalCard1 = readout2.cards.find((c) => callOfNode(c.id) === 'call_ask_1');
        const finalCard2 = readout2.cards.find((c) => callOfNode(c.id) === 'call_ask_2');
        assert(finalCard1 && finalCard2, `收束后两张卡应各归位（实得 ${JSON.stringify(readout2)}）`);
        const result1 = msgs2.find(
          (m) => m.parent_id === finalCard1.id && msgRoleOf(m) === 'tool',
        );
        const result2 = msgs2.find(
          (m) => m.parent_id === finalCard2.id && msgRoleOf(m) === 'tool',
        );
        assert(String(result1.content ?? '').includes(ANSWER_1.pick), '第一个答案应挂在第一张卡下');
        assert(String(result2.content ?? '').includes(ANSWER_2.pick), '第二个答案应挂在第二张卡下');

        // ── 证据 ④：换进程重读同形（落库重建与实时投影一致）──────────────────────
        api.stop();
        // 旧进程必须先释放 gateway 端口（新进程读的是落库重建那条路，
        // 与实时投影互不共享内存）。
        await waitFor(
          async () => {
            try {
              await fetch(`http://127.0.0.1:${GATEWAY_PORT}/api/v1/health`);
              return false;
            } catch {
              return true;
            }
          },
          { what: '旧进程释放 gateway 端口', timeoutMs: 15_000 },
        );
        const api2 = await startReadbackCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          session: SID,
          provider: PROVIDER_ID,
          mode: 'interactive',
          gatewayPort: GATEWAY_PORT,
        });
        try {
          const msgs3 = await readSessionMessages(api2, SID);
          const readout3 = shapeReadout(msgs3);
          assertEq(
            readout3.cards.length,
            2,
            `换进程重读应同形：两张卡（实得 ${JSON.stringify(readout3)}）`,
          );
          assertEq(
            JSON.stringify(readout3.cards.map((c) => callOfNode(c.id))),
            JSON.stringify(wires),
            `换进程重读的卡顺序应与 call_id 排序一致（实得 ${JSON.stringify(readout3.cards)}）`,
          );
          assert(
            readout3.cards.every((c) =>
              msgs3.some((m) => m.parent_id === c.id && msgRoleOf(m) === 'tool'),
            ),
            `换进程重读时每条结果仍应挂在对应卡下（实得 ${JSON.stringify(readout3)}）`,
          );
        } finally {
          api2.stop();
        }
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

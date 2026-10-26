import './_selfrun.mjs';
// T54 断流原样重发的**槽位形态**（黑盒）：一次回答在流中途被上游掐断（t49 的形态），
// 客户端按纪律**原样重发**同一份请求体；重发批未必原样产出同样多的工具调用——
// 这一批少产出时，用户看到的应该是什么。
//
// ## 背景：修复三件套的第三件此前只有白盒判据
//
// 「LLM 单响应工具数超限 → 会话卡住」那组修复有三件套，前两件已有 e2e 锚：
// 超限丢弃 + 反馈 = [t48](t48-megabatch-feedback.mjs)，断流可见失败 = [t49](t49-stream-drop.mjs)。
// 第三件——**断流后原样重发的槽位处置**（`tool_accumulator` 的 `touched` 过滤 +
// `reuse_node_ids` 播种复用，见 `crates/symbio-plugin-model/src/tool_accumulator.rs` 与
// `bound_provider.rs` 的 `MAX_STREAM_RETRIES`）今天只有 `tool_accumulator.test.rs` /
// `stream.test.rs` 的白盒判据（登记 相应批次）。
//
// 它是**用户可见**的，两条症状都能在黑盒上看见：
// - 播种槽位若被当成「有 entry 就有调用」收口 ⇒ 重发批吐出名字 / 参数全空的**幽灵调用**，
//   执行层照空壳发请求，provider 侧报「tool_call 无对应结果」；
// - 重发若**换 id** ⇒ 前端同一批留两套卡片（首批半截 + 重发新卡），后端两条都在。
//
// ## 本用例钉的是什么（判据只走用户可见出口）
//
// | 断言 | 唯一能看见它的地方 |
// |---|---|
// | 首批建了 N 个调用节点，重发**没有新建**（复用 id ⇒ 不重复卡片） | 实时面 `subscribeSessionRealtime`（与前端同构） |
// | 转写里**恰好 M 个**工具调用节点（未触碰的播种槽位不得产出） | 生产读出口 `readSessionMessages` |
// | 重发产出**复用首批的节点 id**（前缀复用，非新建） | 实时面首批到达序 + 转写 id |
// | 不留名字 / 参数全空的幽灵调用 | 转写（`name` 与参数正文） |
// | 有请求必有响应（含重发批） | `assertTranscriptInvariants` |
//
// **反面（播种槽位被当成调用产出）由本用例天然排除**：N > M 时，若 `touched` 过滤失效，
// 转写会多出 N-M 个空壳调用，第 ② 条立刻红。
//
// ## 为什么必须订阅实时面（不能只用转写）
//
// 「id 与首批前 M 个复用一致」需要**首批已广播的节点 id** 这个事实。首批在流中途被
// 掐断、它的产物**不进最终转写**（转写只由成功那批的 `TurnOutput` 构造），因此那个 id
// 只存在于**流式广播的实时面**上——正是用户当时看到的那些帧。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  startLongLivedCli,
  subscribeSessionRealtime,
  DIALOG_FACE_OFF,
  waitFor,
  assert,
  assertEq,
  defineCase,
  nextPort,
  providerConfig,
  PROVIDER_ID,
  readSessionMessages,
  createSession,
  sendInbox,
  assertTranscriptInvariants,
  textOf,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

const SID = 'e2e-t54';
/** 首批：吐 N 个工具调用增量后中途 destroy（N > M，才逼得出「未触碰槽位」这条） */
const FIRST_CALLS = 3;
/** 重发批：只产出 M 个调用并正常收尾（M < N） */
const RESEND_CALLS = 1;
/** 工具调用增量的事件位次：mock-llm 每个调用吐 3 个事件（名字 + 参数两片） */
const EVENTS_PER_CALL = 3;
/** 收束正文（本轮最后的可见正文，用它判本轮跑完） */
const SETTLE_TEXT = '工具结果已收到，收束。';

export default defineCase(
  'T54 断流原样重发：重发批少产出时不留幽灵调用、不重复卡片（槽位复用）',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'drop-mid',
        match: '断流重发',
        times: 1,
        // 第 3 个调用的参数吐到一半就断（"中途 destroy"）：三个节点都已建（touched），
        // 但流没有终止帧 ⇒ 客户端判瞬时故障、原样重发。
        //
        // `chunkDelayMs` 不可省：`res.write` 是异步的，无延迟时 8 个事件还在 Node 侧
        // 缓冲里就 `socket.destroy()`，客户端**一个事件都收不到**（实测：首批 0 个工具帧、
        // `reuse_node_ids` 为空 ⇒ 重发新建节点，本用例的判据被架空）。加延迟让每片先落
        // 到对端，断流才真的断在「已建 3 个节点之后」。
        chunkDelayMs: 15,
        dropAfter: FIRST_CALLS * EVENTS_PER_CALL - 1,
        toolCalls: Array.from({ length: FIRST_CALLS }, (_, i) => ({
          id: `call_${i}`,
          name: 'vdfs_list',
          arguments: { path: '.' },
        })),
      },
      {
        id: 'resend',
        match: '断流重发',
        times: 1,
        content: '重发后完成。',
        // 重发批只产出 M 个调用：`with_node_ids` 播种的 N 个槽位里，只有前 M 个会被
        // 这一批的增量触碰；其余必须被 `touched` 过滤掉，不能变成空壳调用。
        toolCalls: Array.from({ length: RESEND_CALLS }, (_, i) => ({
          id: `call_${i}`,
          name: 'vdfs_list',
          arguments: { path: '.' },
        })),
      },
      // 工具结果轮（末条 role=tool）：不再发起调用 ⇒ 本轮收束。
      { id: 'settle', content: SETTLE_TEXT },
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
        // 断言按**主线装配**的请求轮廓写（末条 user 消息即场景匹配锚）。对话面默认开启，
        // 每轮多插静默请求（classify / compose）——它们的末条 user 消息**同样**是那句
        // 「断流重发测试」，会把本用例编好的场景抢走（实测：断流落在对话面的静默请求上，
        // 主线工具轮根本没发生）。故钉死（理由见 `DIALOG_FACE_OFF`）。
        session: { ...DIALOG_FACE_OFF },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t54',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    let rt = null;
    try {
      await cli.waitGatewayReady();

      // ① 订阅实时面（**发消息之前**，不漏首批节点帧）
      rt = await subscribeSessionRealtime(cli, { sessionId: SID, gatewayPort: GATEWAY_PORT });

      // ② 会话先建出来（收件箱是会话内部的集合），再经 HTTP 边界发起对话：
      //    收件箱地址面（会话目标挂在 metadata.session_id）
      await createSession(cli, SID, { workdir: hd.workdir });
      const send = await sendInbox(cli, SID, '断流重发测试', { workdir: hd.workdir });
      assert(send.status === 200, `写收件箱应受理（${send.status}: ${JSON.stringify(send.body)?.slice(0, 150)}）`);

      // ③ 等本轮收敛：**收束正文**在实时面上到达终态（断流 + 重发 + 工具轮都在这一轮里）。
      //    判据取「可见的收束正文终态」而不是会话运行态字段——前者是本轮跑完的**用户可见**
      //    证据（后者只是内部状态标记，且首次订阅会先收一帧空闲初始态，水位线不好摆）。
      await waitFor(
        () =>
          rt.messages().some(
            (m) =>
              msgTypeOf(m.data) === 'text' &&
              msgRoleOf(m.data) === 'assistant' &&
              m.data?.status === 'completed' &&
              String(m.data?.content ?? '').includes(SETTLE_TEXT),
          ),
        { what: '本轮跑完（收束正文到达终态）', timeoutMs: 60_000 },
      );

      const toolFrames = rt.messages().filter(({ data }) => msgTypeOf(data) === 'tool_call');

      // ── 断言 ①：首批建了 N 个调用节点，重发**没有新建**（id 复用 ⇒ 不重复卡片）──
      // 重发若换 id，这里会多出一个（distinct 变 N+1）——正是「同批两套卡片」的黑盒指纹。
      const distinctIds = [...new Set(toolFrames.map((f) => f.mid))];
      assertEq(
        distinctIds.length,
        FIRST_CALLS,
        `实时面上工具调用节点应恰好 ${FIRST_CALLS} 个（重发复用首批 id，不新建）` +
          `——多于它说明重发换了 id（同批两套卡片）`,
      );

      // ── 断言 ②③：转写里恰好 M 个调用，且复用首批的节点 id（前缀复用）──
      const msgs = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(msgs, 'T54');
      const toolCalls = (msgs ?? []).filter((m) => msgTypeOf(m) === 'tool_call');
      assertEq(
        toolCalls.length,
        RESEND_CALLS,
        `转写应恰好 ${RESEND_CALLS} 个工具调用节点——多于它说明未触碰的播种槽位被当成了调用产出（幽灵调用）`,
      );
      // 首批**首个到达**的节点 id = 前缀复用的锚（到达序即首批的 wire index 序）。
      const firstArrivalId = toolFrames[0].mid;
      assertEq(
        toolCalls[0].id,
        firstArrivalId,
        '重发产出必须复用首批的节点 id（前缀复用），否则前端同一批留两套卡片',
      );

      // ── 断言 ④：不留名字 / 参数全空的幽灵调用 ──────────────────────────
      for (const m of toolCalls) {
        assert(
          typeof m.meta?.tool_name === 'string' && m.meta.tool_name.trim() !== '',
          `工具调用节点不得名字全空（幽灵调用；节点 ${m.id}）`,
        );
        const args = textOf(m).trim();
        assert(
          args !== '' && args !== '{}',
          `工具调用节点不得参数全空（幽灵调用；节点 ${m.id}，参数 ${JSON.stringify(args)}）`,
        );
      }

      // ── 断言 ⑤：本轮正常收束（重发不是「断在这」）──────────────────────
      const texts = (msgs ?? []).map(textOf).join('\n');
      assert(texts.includes('重发后完成'), '重发批的正文应在转写里（重发成功）');
      assert(texts.includes(SETTLE_TEXT), '工具结果轮应正常收束（本轮跑完）');

      // ── 断言 ⑥：编排确实发生过（否则前五条可能「因没断流」而假绿）────────
      // 断流批 + 原样重发批各一次（同一份请求体两次命中），工具结果轮一次 ⇒ ≥3 次。
      const reqs = await llm.requests();
      const sameBody = reqs.filter((r) =>
        (r.body?.messages ?? []).some((m) => m.role === 'user' && String(m.content ?? '').includes('断流重发测试')),
      );
      assert(
        sameBody.length >= 2,
        `断流批 + 重发批应各命中一次（同一份请求体 ≥2 次），实际 ${sameBody.length} 次——` +
          '少于它说明断流没发生，本用例的判据是空的',
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

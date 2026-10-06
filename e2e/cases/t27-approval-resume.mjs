import './_selfrun.mjs';
// T27 审批与恢复（v2 full 档）：工具停在等用户 → 回填答案 → 续写**同一轮**并收束。
//
// ## 本用例钉的是什么
//
// [10 §3 批 2b](../../docs/plan/10-工具轮v2化实施方案.md) 的出口判据原文：
// 「`confirm` 工具 → 本轮停在等用户 → 回填答案 → 续跑并收束（WAL 同一 turn 有 final）」。
//
// 这条判据之所以必须端到端，是因为它要证的三件事**各自只能在一处被看见**：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 工具**真的停在等用户**（本轮不落收束格） | 磁盘上的 `v2-events.wal`（`artifact.added` 有、`final` 无） |
// | 恢复**续写同一轮**（不重开用户格） | 同一份 WAL：`user.message` 仍只 1 条、收束格记在原 turn 号上 |
// | 回填的答案**真的回了模型** | mock-llm 的 `/_requests` 回读**第二次请求体** |
//
// 单测（`actors/mod.test.rs::resume_continues_same_turn_without_reopening_user_cell`）用
// 假 provider + 假分发覆盖了同一形状，但它证不了"真实 provider + 真实工具分发 +
// 真实落盘 + 真实恢复入口"这条链——单测里的 provider / dispatch 都是假件。
//
// ## 为什么工具用 `ask_user`（而不是 `confirm`）
//
// plan/10 §3 写的 `confirm` 是**场景代称**（"一个会停在等用户的工具"），不是工具名——
// 本仓没有名为 `confirm` 的工具。真正的 pending 触发点是 `ask_user`：interactive
// 模式下它返回 `failure_kind = needs_interaction` + `prompt` 载荷，编排层据此构造
// `user_prompt`（`WaitingUserAction`）节点、运行器停止工具循环且**不落收束格**。
//
// ## 为什么 mode 必须是 interactive
//
// `ask_user` 的 auto 分支**不产节点**（返回 `tool_unavailable`，让模型继续）——
// auto 档下本用例根本到不了"等用户"。故两处请求都显式带 `mode: 'interactive'`。
//
// ## 恢复入口为什么是 gateway 直发 `session/chat/send`
//
// plan/10 §2.4 的前置原文：CLI 把 `resume: None` 写死（`cli/src/client.rs`），
// **没有恢复入口**。批 2b 要补的正是这个入口。这里取该条明确列出的第二种形态——
// 「e2e 直发 `session/chat/send` 的 `resume` 字段」：经 gateway HTTP 边界
// （与 Tauri 前端同构的 `/api/v1/invoke`）发一条带 `resume` 的请求。
// 好处是不动生产代码，且验的正是**前端恢复走的同一条路**（`handle_chat_send_oneoff`
// 的 resume 分支 → `start_turn` 直连）。
//
// ## 为什么工具调用场景要 `once: true`（与 T26 同因）
//
// v2 运行器把整轮 prompt **平铺成一条 user 消息**，第二次请求的最后一条仍是 user
// （见 `symbio_core/actors/mod.rs::render_tool_exchange`）。且本用例的恢复轮 prompt
// 里**仍含**首轮用户那句话（转写投影照实渲染）——不带 `once` 会让带 `match` 的
// 工具场景反复命中 ⇒ 无限工具循环。
//
// ## 为什么把对话面钉死（`DIALOG_FACE_OFF`）
//
// 对话面（`classify` / `compose`）默认开启，会给每一轮多加一次静默请求。本用例按
// **精确请求数**（2 次）写断言，且验的是审批/恢复链路——与对话面正交，故钉死。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  readMessagesJson,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  turnFacts,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t27';

/** v2 事实源文件名（与 `plugins/session/paths.rs::V2_WAL_FILE` 同一常量口径） */
const V2_WAL = 'v2-events.wal';

/** 触发 pending 的工具名（本仓真正的「停在等用户」触发点）。 */
const PENDING_TOOL = 'ask_user';

/** 提问与答案：答案必须从恢复入口一路走到第二次请求体。 */
const QUESTION = '你选哪一项';
const ANSWER = { choice: 'A' };

/** 读文件里的全部事件（出口读数的对账基准）。 */
function readEvents(walPath) {
  const raw = readFileSyncSafe(walPath);
  assert(raw.length > 0, `v2 事实源应存在且非空：${walPath}`);
  return raw.split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

export default defineCase(
  'T27 审批与恢复：工具停等用户 → 回填答案 → 续写同一轮并收束（WAL 同一 turn 有 final）',
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
        // 本用例的主题与对话面正交 ⇒ 把三个开关钉死（理由见 `DIALOG_FACE_OFF`），
        // 再叠加**本用例的主题**：full 档（v2 原生执行，工具轮走 `DispatchPort`）。
        session: { ...DIALOG_FACE_OFF, v2_mode: 'full' },
      },
    });

    const walPath = join(hd.homedir, 'session', SID, V2_WAL);

    try {
      // ── 第一幕：工具停在等用户（本轮收束但**不落收束格**）───────────────────
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

      // 证据 ①：等待轮 = 用户格 + 产物格（pending），**没有**收束格。
      // 这正是「还没完」的诚实缺口——C4 看得见它，恢复轮要负责把它填上。
      const pending = readEvents(walPath);
      const pendingKinds = pending.map((e) => e.kind).join(', ');
      const opens0 = pending.filter((e) => e.kind === 'user.message');
      assertEq(opens0.length, 1, `等待轮应恰好 1 条用户格（实得: ${pendingKinds}）`);
      assert(
        pending.some((e) => e.kind === 'artifact.added'),
        `等待轮应落产物格（实得: ${pendingKinds}）`,
      );
      assert(
        !pending.some((e) => e.kind === 'chat.assistant.final'),
        `等待轮**不该**落收束格——那是「还没完」的诚实缺口（实得: ${pendingKinds}）`,
      );
      const user = opens0[0];
      assertEq(user.turn, 0, '首轮 turn 号');
      // 等待轮的产物格记被调工具与 pending 文案，溯源指向本轮用户格（I2）。
      const pendingArtifact = pending.find((e) => e.kind === 'artifact.added');
      assertEq(pendingArtifact.payload.tool, PENDING_TOOL, '产物格记录停在等用户的工具名');
      assertEq(
        pendingArtifact.produced_by,
        user.seq,
        '等待轮产物格溯源应指向本轮用户格（I2）',
      );

      // 证据 ①′：会话面看得见提问节点（tool_call + user_prompt/WaitingUserAction）——
      // 恢复入口要的 `target_id` 就是这里的 tool_call 节点 id。
      const msgs0 = readMessagesJson(hd.homedir, SID);
      assertTranscriptInvariants(msgs0, 'T27 等待轮');
      const tc = msgs0.find((m) => (m.type ?? m.msg_type) === 'tool_call');
      assert(tc, `应有 tool_call 节点（实得类型: ${msgs0.map((m) => m.type).join(',')}）`);
      const promptNode = msgs0.find(
        (m) =>
          m.parent_id === tc.id &&
          (m.type ?? m.msg_type) === 'user_prompt' &&
          m.status === 'waiting_user_action',
      );
      assert(
        promptNode,
        `tool_call 之下应有 user_prompt（WaitingUserAction）节点（实得: ${JSON.stringify(
          msgs0.filter((m) => m.parent_id === tc.id).map((m) => ({ type: m.type, status: m.status })),
        )}）`,
      );

      // ── 第二幕：回填答案（恢复入口 = gateway 直发 `session/chat/send` 的 resume）──
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

        const inv = await api.invoke(
          'session/chat/send',
          {
            session_id: SID,
            mode: 'interactive',
            // 恢复请求：`message` 与 `resume` 互斥（入口显式校验），此处只给 `resume`。
            // `target_id` = 提问所在 ToolCall 节点 id（稳定锚点）。
            resume: { target_id: tc.id, action: 'answer', answer: ANSWER },
          },
          { session_id: SID },
        );
        assertEq(inv.status, 200, `恢复请求应被接受（实际: ${JSON.stringify(inv.body)}）`);

        // 恢复是 spawn 执行（`start_turn` 立即返回 accepted）⇒ 轮询 WAL 等收束格落地。
        await waitFor(() => readEvents(walPath).some((e) => e.kind === 'chat.assistant.final'), {
          what: '恢复轮收束（WAL 出现 chat.assistant.final）',
          timeoutMs: 30_000,
        });

        // ── 证据 ②：续写**同一轮**——用户格不重开、收束记在原 turn 号上 ──────────
        const after = readEvents(walPath);
        const afterKinds = after.map((e) => e.kind).join(', ');
        assertEq(
          after.filter((e) => e.kind === 'user.message').length,
          1,
          `续写**不重开用户格**——同一句话只能出现一次（实得: ${afterKinds}）`,
        );
        assert(
          after.every((e) => e.turn === 0),
          `全部事实仍记在 turn 0 上（实得: ${after.map((e) => `${e.kind}@${e.turn}`).join(',')}）`,
        );
        const final = after.find((e) => e.kind === 'chat.assistant.final');
        assertEq(final.payload.text, '已收到你的选择。', '收束格落的是续写轮正文');
        assertEq(final.produced_by, user.seq, '收束格溯源指向**原轮**用户格（I2）');

        // 两条产物格：等待轮的 pending + 恢复轮的答案结果。
        const artifacts = after.filter((e) => e.kind === 'artifact.added');
        assertEq(artifacts.length, 2, `应有两条产物格（pending + 恢复）（实得: ${afterKinds}）`);
        const resumedArtifact = artifacts[1];
        assertEq(resumedArtifact.payload.tool, PENDING_TOOL, '恢复产物格记被恢复的工具');
        assert(
          String(resumedArtifact.payload.text ?? '').includes('"choice"'),
          `恢复产物格落的是答案正文（实际: ${JSON.stringify(resumedArtifact.payload)}）`,
        );
        assertEq(
          resumedArtifact.produced_by,
          user.seq,
          '恢复产物格溯源仍指向**原轮**用户格（I2）',
        );

        // 期望值本身也钉死（防止两边一起算错）：原轮缺口被真正填上。
        // 只数**轮次事实**——派生副作用（记忆与学习，S5 步 11–13 / S11 步 22）不在其中
        // （见 `helpers.mjs::turnFacts`）。
        assertEq(
          turnFacts(after).length,
          4,
          `恢复后应恰好 4 格轮次事实（u + a(pending) + a(resume) + f），实得: ${afterKinds}`,
        );

        // ── 派生副作用：恢复路径同样派生记忆（本批新增 ⇒ 判据化）──────────────
        // 两幕都走 `v2_exec::execute_turn` ⇒ 两幕都在轮末调 `record_learning`。
        // ① 编码**恰好一条**：恢复幕的发言与首幕同文，步 11 按内容去重
        //    （`RecallView::contains_content`）⇒ 同一轮不会编出第二条。
        // ② 检索锚一条：恢复幕的召回视图非空（首幕刚编的那条）⇒ 落 `memory.recalled`。
        // 两条的溯源锚都指**原轮**用户格（I2）——恢复不新开用户格。
        const encoded = after.filter(
          (e) => e.kind === 'memory.encoded' && e.payload?.tag === '经验',
        );
        assertEq(encoded.length, 1, `本轮只编一条记忆（恢复幕按内容去重）：${afterKinds}`);
        assertEq(encoded[0].produced_by, user.seq, '编码的溯源锚 = 原轮用户格（I2）');
        const recalledEvents = after.filter((e) => e.kind === 'memory.recalled');
        assert(recalledEvents.length >= 1, `恢复幕应落检索锚：${afterKinds}`);
        assertEq(recalledEvents[0].produced_by, user.seq, '检索锚的溯源指向原轮用户格（I2）');

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
        const second = promptOf(reqs[1].body);
        assert(
          second.includes(`助手请求工具: ${PENDING_TOOL}`),
          `恢复轮 prompt 应带上被恢复的工具调用（实际片段: ${second.slice(0, 800)}）`,
        );
        assert(
          second.includes(`工具结果(${PENDING_TOOL}):`),
          `恢复轮 prompt 应带上恢复结果行（实际片段: ${second.slice(0, 800)}）`,
        );
        assert(second.includes('"choice"'), '回填的答案键（choice）应原样回灌给模型');
        assert(second.includes('"A"'), '回填的答案值（A）应原样回灌给模型');
        // 反向：第一次请求里不该有恢复结果（那时还没恢复）。
        assert(
          !promptOf(reqs[0].body).includes(`工具结果(${PENDING_TOOL}):`),
          '第一次请求不该出现恢复结果（答案尚未回填）',
        );

        // ── 转写不变量：恢复后节点仍成对、seq 严格递增 ───────────────────────
        const msgs1 = readMessagesJson(hd.homedir, SID);
        assertTranscriptInvariants(msgs1, 'T27 恢复后');
        const tc1 = msgs1.find((m) => (m.type ?? m.msg_type) === 'tool_call');
        assert(
          msgs1.some((m) => m.role === 'tool' && m.parent_id === tc1.id),
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

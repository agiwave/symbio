import './_selfrun.mjs';
// T78 审批闸门的覆盖面：`vdfs_delete` 声明 `risk=High`，执行期必须被读到。
//
// ## 本用例钉的是什么
//
// 的出口判据原文：「经任一注册入口进 ToolManager 的能力都过同一道 `approval_gate`；
// 判据 = 一条 e2e，交互模式下 `vdfs_delete` 必须先出确认卡、批准后才删除，`auto`
// 模式下按 `PERMISSION_DENIED` 继续」。
//
// 缺陷的现场不是「闸门判错了」——判据一直是对的——而是**闸门长在 `local` 插件的
// 注册循环上**（`SecureToolWrapper`）：`vdfs` 走的是自己那条
// `visitor.register(tool)`，工具**裸进表**，High 风险声明从落盘那一刻起就没有
// 任何一次执行读过它。修复的形状（闸门搬到 `DefaultToolVisitor::register`）
// 只能靠「另一个插件的工具也被拦下」来证明——所以本用例的**唯一**样本工具
// 就选 `vdfs_delete`，不选 shell（shell 走 `local`，它过去也在闸内，改了也看不出错）。
//
// ## 两幕各自的判据位置
//
// | 幕 | 判据 | 唯一可见的地方 |
// |---|---|---|
// | A 交互 | 首轮停在等用户 + 磁盘上文件**仍在** | 转写里 `tool_call` 之下 `user_prompt(waiting_user_action)`；`readFileSyncSafe` |
// | A 批准 | 恢复后 `tool_call` 有 role=tool 成功结果 + **文件真没了** | 读出口 + `existsSync` |
// | B 自动 | 结果 `failure_kind=permission_denied` + 文件仍在 | 读出口的 tool 结果 meta + `existsSync` |
//
// A 幕的「文件仍在」是修复前后**唯一的可观测差异**：修复前它直接被删，
// `pending=-`；修复后必须**停在卡上**——这一格只有磁盘知道答案，转写里没有。
//
// ## 为什么用 `--session` 起步 + 独立读回 CLI
//
// 与 T27 同因：审批恢复的入口是节点动作（`vdfs/action(<sid>/message/<tc-id>, "approve")`），
// 它要经 gateway 边界；跑完首轮就要读转写、找 `target_id`、发动作、再读一次——
// 全部经**同一条**读回句柄完成（前端就是这么做的）。
//
// ## 为什么 mock 场景要 `once: true`
//
// 与 T26/T27 同因：恢复轮 prompt 里**仍含**首轮用户那句话，带 `match` 的
// 工具场景会在结果轮再次命中 ⇒ 无限工具循环。首轮场景烧一次即断。
//
// ## 为什么把对话面钉死
//
// 判据要打在「首轮**就停在卡上**」——判决与措辞开着时，同样的 prompt 可能被
// 判成先答话再问，走的就不是同一条收束路径；本用例与对话面正交，钉死它。
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
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
  readFileSyncSafe,
  readSessionMessages,
  resumeSession,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

const SID_INTERACTIVE = 'e2e-t78-interactive';
const SID_AUTO = 'e2e-t78-auto';

/** 待删文件相对 vdfs 根的路径（vdfs 根 = 会话 `workdir`；与 T10 `vdfs_write` 同一条根）。 */
const VICTIM_REL = 't78-junk.txt';

export default defineCase(
  'T78 审批闸门覆盖面：vdfs_delete(High) 首轮必停在确认卡，批准后才落删除；auto 模式 PERMISSION_DENIED',
  async () => {
    const llm = await new MockLlm([
      // 首轮：模型直接请求 vdfs_delete（本用例主题正是要它**先被拦下**）。
      {
        id: 'ask-delete',
        match: '删掉那个垃圾文件',
        once: true,
        content: '我先删掉它。',
        toolCalls: [
          {
            id: 'call_delete',
            name: 'vdfs_delete',
            arguments: { path: VICTIM_REL, recursive: false },
          },
        ],
      },
      // 恢复后的收尾轮：不再请求工具 ⇒ 本轮收束。
      { id: 'after-approve', content: '已按批准删除。' },
      // B 幕（auto）：同一工具场景在**自动模式**会话里命中；闸门返回 permission_denied，
      // 模型看到错误后直接结束本轮（不再重试）。
      {
        id: 'ask-delete-auto',
        match: '自动模式删掉那个垃圾文件',
        once: true,
        toolCalls: [
          {
            id: 'call_delete_auto',
            name: 'vdfs_delete',
            arguments: { path: VICTIM_REL, recursive: false },
          },
        ],
      },
      { id: 'auto-stops', content: '自动模式下我不能删。' },
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
        session: { ...DIALOG_FACE_OFF },
      },
    });
    // 预置两份待删文件（A/B 幕各一）；两幕走两个独立会话、同一 workdir——**每次跑之前重铺**。
    // `vdfs_delete` 的 `path` 是 vdfs 根相对路径（与 T10 `vdfs_write` 同一条根），
    // 落盘位置即 `<workdir>/<path>`；`tmp/` 段需要先建目录。
    const seedVictim = () => {
      const abs = join(hd.workdir, ...VICTIM_REL.split('/'));
      mkdirSync(join(abs, '..'), { recursive: true });
      writeFileSync(abs, 'junk\n');
      return abs;
    };

    // 事实面读面：授权级联必须留下 `request` + `authz.denied`，
    // 而拒绝的那一笔**不能**有 `sent`——「没放行 ⇒ 没派发」是 fail-closed 的
    // 唯一可观测形态。直接读会话 WAL，不经过转写（转写是投影，不是事实源）。
    const readWal = (sid) =>
      readFileSyncSafe(join(hd.homedir, 'session', sid, 'v2-events.wal'))
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));
    const toolFacts = (events) => events.filter((e) => e?.payload?.port === 'tool');
    const assertDeniedWithoutSent = (sid, detail) => {
      const facts = toolFacts(readWal(sid));
      const denied = facts.find(
        (e) => e.kind === 'authz.denied' && e.payload?.detail === detail,
      );
      assert(
        denied,
        `${sid}: 工具授权拒绝必须落 \`authz.denied\`（detail=${detail}）；实际: ${JSON.stringify(
          facts.map((e) => ({ kind: e.kind, id: e.payload?.id, detail: e.payload?.detail })),
        )}`,
      );
      const id = denied.payload?.id;
      assert(
        facts.some((e) => e.kind === 'request' && e.payload?.id === id),
        `${sid}: 拒绝的那一笔必须先有 \`request\`（id=${id}）`,
      );
      assert(
        !facts.some((e) => e.kind === 'sent' && e.payload?.id === id),
        `${sid}: 拒绝 ⇒ 不得有 \`sent\`（id=${id}）——没放行就没派发`,
      );
    };

    let api = null;
    try {
      // ── A 幕：交互模式，首轮必须停在确认卡上（**文件仍在**是修复前后的分水岭）─────
      const victimA = seedVictim();
      assert(existsSync(victimA), `预置文件失败：${victimA}`);

      const first = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '删掉那个垃圾文件',
        provider: PROVIDER_ID,
        session: SID_INTERACTIVE,
        mode: 'interactive',
        // 会话 metadata 里 CLI 写死 `risk_level: medium`（`crates/symbio-cli/src/client.rs:373`）；
        // High > Medium ⇒ 必拦。
      });
      assertEq(
        first.code,
        0,
        `交互首轮应以「本轮停在等用户」的正常形态退出（stderr: ${first.stderr.slice(0, 600)}）`,
      );

      // **磁盘侧判据**：闸门若没生效，文件会被直接删——这是 的原始现场。
      assert(
        existsSync(victimA),
        `首轮**必须停在卡上**——磁盘上文件仍在才算闸门生效（修复前它直接被删）`,
      );

      // 起读回 CLI：找 target_id、发动作、再读一次转写都走它（前端形状）。
      api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_INTERACTIVE,
        provider: PROVIDER_ID,
        mode: 'interactive',
        gatewayPort: GATEWAY_PORT,
      });
      await api.waitGatewayReady();

      const msgs0 = await readSessionMessages(api, SID_INTERACTIVE);
      assertTranscriptInvariants(msgs0, 'T78-A 等待轮');
      const tc = msgs0.find((m) => msgTypeOf(m) === 'tool_call');
      assert(
        tc && String(tc.content ?? tc.meta?.name ?? '').includes('vdfs_delete') || tc,
        `应有 vdfs_delete 的 tool_call 节点（实得: ${msgs0.map((m) => m.type).join(',')}）`,
      );
      const promptNode = msgs0.find(
        (m) =>
          m.parent_id === tc.id &&
          msgTypeOf(m) === 'user_prompt' &&
          m.status === 'waiting_user_action',
      );
      assert(
        promptNode,
        `tool_call 之下必须有 user_prompt(waiting_user_action)——这就是「确认卡」的转写形状（实得 children: ${JSON.stringify(
          msgs0.filter((m) => m.parent_id === tc.id).map((m) => ({ type: m.type, status: m.status })),
        )}）`,
      );
      // 交互幕的拒绝理由：等待用户批准。事实面必须在**执行之前**就关掉这一笔。
      assertDeniedWithoutSent(SID_INTERACTIVE, 'needs_approval');

      // ── 批准恢复：`action(<sid>/message/<tc.id>, "approve")` ⇒ approved:true ⇒ 落删除 ──
      const inv = await resumeSession(
        api,
        SID_INTERACTIVE,
        tc.id,
        'approve',
        { mode: 'interactive' },
        { session_id: SID_INTERACTIVE },
      );
      assert(
        inv.status === 200 && inv.body?.data?.ok === true,
        `恢复请求应被受理（实际: ${JSON.stringify(inv.body)}）`,
      );

      // 恢复轮真的跑了（模型收到第二次请求 ⇒ 收束）
      await waitFor(async () => (await llm.requests()).length >= 2, {
        what: 'T78-A 恢复轮收束（mock 收到第二次请求）',
        timeoutMs: 30_000,
      });

      // **磁盘侧判据**：批准后文件必须真的没了——这证的是「恢复走到 execute」，
      // 光看转写里的 completed 状态不够（那是节点侧；工具是否落到内层只有 OS 知道）。
      assert(
        !existsSync(victimA),
        `批准恢复后文件应被真删（磁盘上仍在 ⇒ approved:true 没穿过闸门）`,
      );

      const msgs1 = await readSessionMessages(api, SID_INTERACTIVE);
      const tc1 = msgs1.find((m) => msgTypeOf(m) === 'tool_call');
      assert(
        msgs1.some((m) => msgRoleOf(m) === 'tool' && m.parent_id === tc1.id),
        '恢复后 tool_call 须有结果子节点',
      );

      // 事实面：批准恢复的那一笔必须走完 `request → granted → sent → response`。
      // 这是端口账的分水岭——在此之前 resume 直接调执行层，端口账上
      // 只有首轮的 `denied`，批准后的执行是事实面的空白。
      const aFacts = toolFacts(readWal(SID_INTERACTIVE));
      const aGranted = aFacts
        .filter((e) => e.kind === 'authz.granted' && e.payload?.tool === 'vdfs_delete')
        .at(-1);
      assert(
        aGranted,
        `批准恢复必须落 authz.granted（实际工具事实: ${JSON.stringify(
          aFacts.map((e) => ({ kind: e.kind, id: e.payload?.id, tool: e.payload?.tool })),
        )}）`,
      );
      const gid = aGranted.payload?.id;
      for (const kind of ['request', 'sent', 'response']) {
        assert(
          aFacts.some((e) => e.kind === kind && e.payload?.id === gid),
          `批准恢复的 ${kind} 必须与 authz.granted 同 id（id=${gid}）`,
        );
      }
      assertTranscriptInvariants(msgs1, 'T78-A 恢复后');

      api.stop();
      api = null;

      // ── B 幕：auto 模式 ⇒ 闸门不产卡，直接以 permission_denied 反馈 ─────────────
      const victimB = seedVictim();
      assert(existsSync(victimB), `预置文件失败：${victimB}`);

      const b = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '自动模式删掉那个垃圾文件',
        provider: PROVIDER_ID,
        session: SID_AUTO,
        mode: 'auto',
      });
      assertEq(
        b.code,
        0,
        `auto 首轮正常收束（stderr: ${b.stderr.slice(0, 600)}）——闸门返回的是失败载荷，不是崩`,
      );
      assert(
        existsSync(victimB),
        `auto 模式下 High 风险工具**不得执行**（磁盘上文件被删 ⇒ 判定漏了 mode=auto 分支）`,
      );

      api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID_AUTO,
        provider: PROVIDER_ID,
        mode: 'auto',
        gatewayPort: GATEWAY_PORT,
      });
      await api.waitGatewayReady();
      const msgsB = await readSessionMessages(api, SID_AUTO);
      assertTranscriptInvariants(msgsB, 'T78-B');
      // auto 幕的 tool 结果子节点 meta 里 failure_kind 应为 permission_denied。
      const toolResult = msgsB.find((m) => msgRoleOf(m) === 'tool');
      assert(
        toolResult,
        `auto 幕应有 tool 结果节点（实得: ${msgsB.map((m) => m.type ?? m.role).join(',')}）`,
      );
      // auto 幕闸门给的是 `PERMISSION_DENIED` 载荷：文本里带「自动模式」与「权限不足」两条
      // 判据（见 `approval.rs::confirm_prompt_payload`）。工具执行器对 `Ok(payload)` 一律
      // `success: true`（这是**既有约定**，与本 ADR 无关），所以 `meta.failure_kind` 上
      // 看不到 `permission_denied`——它住在 payload 文本里，交给模型读。
      const txt = String(toolResult.content?.text ?? toolResult.content ?? '');
      assert(
        txt.includes('自动模式') && txt.includes('权限不足'),
        `auto 幕 tool 结果应回灌「权限不足…自动模式…」的闸门文案（实得: ${JSON.stringify(txt.slice(0, 300))}）`,
      );
      assert(
        !msgsB.some(
          (m) => msgTypeOf(m) === 'user_prompt' && m.status === 'waiting_user_action',
        ),
        'auto 模式不得产卡（无人可批 ⇒ 产卡等于把会话钉死在等待态）',
      );
      // auto 幕的拒绝理由：权限不足（不是「等用户」）。同样不得有 `sent`。
      assertDeniedWithoutSent(SID_AUTO, 'permission_denied');
    } finally {
      if (api) api.stop();
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

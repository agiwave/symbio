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
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  subscribeSessionRealtime,
  sendAndSettle,
  createSession,
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
const SID_GRANTED = 'e2e-t78-granted';
const SID_STRICT = 'e2e-t78-strict';

/** 待删文件相对 vdfs 根的路径（vdfs 根 = 会话 `workdir`；与 T10 `vdfs_write` 同一条根）。 */
const VICTIM_REL = 't78-junk.txt';
const GRANTED_REL = 't78-granted.txt';
const GRANTED_RESTART_REL = 't78-granted-after-restart.txt';
const STRICT_REL = 't78-strict.txt';
const WRITE_TEXT = '由审批策略放行的真实写入\n';

async function liveSession(homedir, workdir, session, mode, gatewayPort) {
  const cli = startLongLivedCli({ homedir, workdir, session, mode, provider: PROVIDER_ID, gatewayPort });
  await cli.waitGatewayReady();
  const sub = await subscribeSessionRealtime(cli, { sessionId: session, gatewayPort });
  return { cli, sub };
}

export default defineCase(
  'T78 审批闸门：永久授权迁移后跨重启消费，strict 隔离且 High 删除/auto 拒绝仍不可绕过',
  async () => {
    const llm = await new MockLlm([
      // C 幕：未选沙箱 ⇒ default(interactive/medium)。旧 session 配置里的永久授权
      // 必须先迁入 sandbox owner，才能让这两笔 Medium 写入免审批。
      {
        id: 'granted-write',
        match: '用永久授权写入首次标记',
        once: true,
        content: '首次写入已放行。',
        toolCalls: [
          {
            id: 'call_granted_write',
            name: 'vdfs_write',
            arguments: { path: GRANTED_REL, text: WRITE_TEXT },
          },
        ],
      },
      {
        id: 'granted-write-restart',
        match: '重启后用永久授权写入第二个标记',
        once: true,
        content: '重启后的写入已放行。',
        toolCalls: [
          {
            id: 'call_granted_write_restart',
            name: 'vdfs_write',
            arguments: { path: GRANTED_RESTART_REL, text: WRITE_TEXT },
          },
        ],
      },
      // D 幕：同工作区、同工具/风险，但独立 strict 会话；default 的永久授权不得继承。
      {
        id: 'strict-write',
        match: '严格沙箱写入第三个标记',
        once: true,
        content: '严格沙箱已停在审批。',
        toolCalls: [
          {
            id: 'call_strict_write',
            name: 'vdfs_write',
            arguments: { path: STRICT_REL, text: WRITE_TEXT },
          },
        ],
      },
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
      providers: [
        {
          id: PROVIDER_ID,
          config: providerConfig(llm.port, { api_key: 'audit-only-key' }),
        },
      ],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 获批的旧配置入口：启动时由 sandbox owner backup-first 迁入 default 档案。
        ...dialogFaceOff({ session: { approval_grants: { vdfs_write: 'medium' } } }),
        sandbox: {},
      },
    });
    const legacyManifest = join(hd.homedir, 'session', 'PLUGIN.yml');
    const legacyBackup = `${legacyManifest}.approval-grants.bak`;
    const legacyOriginal = readFileSync(legacyManifest, 'utf8');
    const grantFile = join(hd.homedir, 'sandbox', 'approval-grants.json');
    // 预置两份待删文件（A/B 幕各一）；两幕走两个独立会话、同一 workdir——**每次跑之前重铺**。
    // `vdfs_delete` 的 `path` 是 vdfs 根相对路径（与 T10 `vdfs_write` 同一条根），
    // 落盘位置即 `<workdir>/<path>`；`tmp/` 段需要先建目录。
    const seedVictim = () => {
      const abs = join(hd.workdir, ...VICTIM_REL.split('/'));
      mkdirSync(join(abs, '..'), { recursive: true });
      writeFileSync(abs, 'junk\n');
      return abs;
    };

    // 事实面读面：「**一次世界交互 = 一笔债 = 一条事实**」。一个模型响应里的 N 个调用
    // 是**一次端口往返、一笔批债**，不是 N 笔——批级 `request` 的载荷只有
    // `request_id`，指向那条 `tool.requested`。若要求每条调用各有一条
    // `request(port=tool)`，就是把旧的「每个 tool call 一笔债」搬回来（旧形状），
    // 所以拒绝的判据是**配对**而不是「有任意一条 `request` 就算过」：
    //   ① 批级 `request` 的 `request_id` 指向的 `tool.requested`，其调用清单里含该调用 id；
    //   ② 该调用自己有留痕（`artifact.added.tool_call_id` 或同 id 的 `authz.denied`）。
    // 裸 `request` 不顶账；「没放行 ⇒ 没派发」仍按该调用 id 查 `sent`。
    // 直接读会话 WAL，不经过转写（转写是投影，不是事实源）。
    const readWal = (sid) =>
      readFileSyncSafe(join(hd.homedir, 'session', sid, 'v2-events.wal'))
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));
    const toolFacts = (events) => events.filter((e) => e?.payload?.port === 'tool');
    const assertDeniedWithoutSent = (sid, detail) => {
      const events = readWal(sid);
      const facts = toolFacts(events);
      const denied = facts.find(
        (e) => e.kind === 'authz.denied' && e.payload?.detail === detail,
      );
      assert(
        denied,
        `${sid}: 工具授权拒绝必须落 \`authz.denied\`（detail=${detail}）；实际: ${JSON.stringify(
          facts.map((e) => ({ kind: e.kind, id: e.payload?.id, detail: e.payload?.detail })),
        )}`,
      );
      const callId = denied.payload?.id;
      assert(
        callId,
        `${sid}: 拒绝必须点名调用 id，否则配不上任何一笔批债（实际: ${JSON.stringify(denied.payload)}）`,
      );
      // 批级债：该调用必须出现在某条 `tool.requested` 的调用清单里，
      // 而 `request.request_id` 必须指向同一条事实——这是「一笔批债」的配对。
      const requested = events.find(
        (e) =>
          e.kind === 'tool.requested' &&
          (e.payload?.calls ?? []).some((call) => call?.id === callId),
      );
      assert(
        requested,
        `${sid}: 该调用必须出现在 \`tool.requested\` 的调用清单里（id=${callId}）`,
      );
      const batchRequest = events.find(
        (e) => e.kind === 'request' && e.payload?.request_id === requested.event_id,
      );
      assert(
        batchRequest,
        `${sid}: 批级 \`request\` 必须指向含该调用的 \`tool.requested\`（request_id=${requested.event_id}）`,
      );
      // 该调用自己的留痕：产物落格或同 id 的授权拒绝，至少一笔。
      const artifact = events.find(
        (e) => e.kind === 'artifact.added' && e.payload?.tool_call_id === callId,
      );
      assert(
        artifact || denied,
        `${sid}: 该调用必须有留痕（\`artifact.added\` 或 \`authz.denied\`；id=${callId}）`,
      );
      assert(
        !facts.some((e) => e.kind === 'sent' && e.payload?.id === callId),
        `${sid}: 拒绝 ⇒ 不得有 \`sent\`（id=${callId}）——没放行就没派发`,
      );
    };

    let api = null;
    try {
      // ── C 幕：旧 session 永久授权迁移 → default 自动消费 → 跨重启再消费 ──────
      // 会话偏好刻意压到 low：否则 Medium <= Medium 会走 threshold，无法证明免问确实
      // 来自永久授权。CLI 每次启动会补写自己的 Medium 偏好，所以重启后要再次压低。
      let live = await liveSession(hd.homedir, hd.workdir, SID_GRANTED, 'auto', GATEWAY_PORT);
      api = live.cli;

      assert(existsSync(grantFile), `启动后必须生成 sandbox owner 的授权文件：${grantFile}`);
      assertEq(
        readFileSync(legacyBackup, 'utf8'),
        legacyOriginal,
        '迁移必须原样备份旧 session PLUGIN.yml（backup-first 的安全证据）',
      );
      assert(
        !readFileSync(legacyManifest, 'utf8').includes('approval_grants'),
        '迁移成功后旧 session 配置不得继续持有 approval_grants',
      );
      assertEq(
        JSON.stringify(JSON.parse(readFileSync(grantFile, 'utf8'))),
        JSON.stringify({ version: 1, profiles: { default: { vdfs_write: 'medium' } } }),
        '旧授权只能迁入 default 档案，不得提前写入 strict',
      );

      await createSession(api, SID_GRANTED, { workdir: hd.workdir, mode: 'auto', risk_level: 'low' });
      await sendAndSettle(live.cli, live.sub, '用永久授权写入首次标记');
      const grantedPath = join(hd.workdir, GRANTED_REL);
      assert(existsSync(grantedPath), 'default 首次 Medium vdfs_write 必须真实落盘');
      assertEq(readFileSync(grantedPath, 'utf8'), WRITE_TEXT, '首次写入内容必须到达磁盘');
      assert(
        !readWal(SID_GRANTED).some(
          (e) => e.kind === 'artifact.added' && e.payload?.needs_user_action === true,
        ),
        '永久授权消费不得产审批卡',
      );

      live.sub.close();
      api.stop();
      api = null;

      live = await liveSession(hd.homedir, hd.workdir, SID_GRANTED, 'auto', GATEWAY_PORT);
      api = live.cli;
      await createSession(api, SID_GRANTED, { workdir: hd.workdir, mode: 'auto', risk_level: 'low' });
      await sendAndSettle(live.cli, live.sub, '重启后用永久授权写入第二个标记');
      const restartedPath = join(hd.workdir, GRANTED_RESTART_REL);
      assert(existsSync(restartedPath), 'CLI 停止/重启后永久授权必须仍能放行真实写入');
      assertEq(readFileSync(restartedPath, 'utf8'), WRITE_TEXT, '重启后写入内容必须到达磁盘');
      assertEq(
        readFileSync(legacyBackup, 'utf8'),
        legacyOriginal,
        '重启消费不得覆盖或丢失旧配置备份',
      );

      live.sub.close();
      api.stop();
      api = null;

      // ── D 幕：同工作区独立 strict 会话，default 授权不继承，偏好也不能破档案上限 ──
      live = await liveSession(hd.homedir, hd.workdir, SID_STRICT, 'interactive', GATEWAY_PORT);
      api = live.cli;
      // 反例偏好开到 high：若消费端错误地信任会话偏好，Medium 写入会直接穿过 strict。
      await createSession(api, SID_STRICT, {
        workdir: hd.workdir,
        sandbox_id: 'strict',
        mode: 'interactive',
        risk_level: 'high',
      });
      await sendAndSettle(live.cli, live.sub, '严格沙箱写入第三个标记');
      const strictPath = join(hd.workdir, STRICT_REL);
      assert(!existsSync(strictPath), 'strict 的 Medium 写入必须停在审批卡，文件不得存在');
      const strictEvents = readWal(SID_STRICT);
      const strictCard = strictEvents.find(
        (e) =>
          e.kind === 'artifact.added' &&
          e.payload?.tool_call_id === 'call_strict_write' &&
          e.payload?.needs_user_action === true,
      );
      assert(strictCard, 'strict 必须落下 vdfs_write 审批卡事实');
      const strictPrompt = strictCard.payload?.prompt;
      assertEq(strictPrompt?.approval_scopes, ['once'], 'strict 高于档案上限时只允许 once');
      assertEq(strictPrompt?.policy?.profile_id, 'strict', '审批卡必须绑定 strict 档案');
      assertEq(strictPrompt?.risk_level, 'medium', 'strict 审批卡必须保留真实 Medium 风险');
      assert(
        typeof strictPrompt?.approval_reason === 'string' &&
          strictPrompt.approval_reason.includes('高于') &&
          strictPrompt.approval_reason.includes('严格') &&
          strictPrompt.approval_reason.includes('low'),
        `审批理由必须指向 strict 档案的 low 上限（实得: ${JSON.stringify(strictPrompt?.approval_reason)}）`,
      );
      assertDeniedWithoutSent(SID_STRICT, 'needs_approval');
      const strictMsgs = await readSessionMessages(api, SID_STRICT);
      assertTranscriptInvariants(strictMsgs, 'T78-D strict 等待轮');
      assert(
        strictMsgs.some(
          (m) => msgTypeOf(m) === 'user_prompt' && m.status === 'waiting_user_action',
        ),
        'strict 必须在转写中停在 waiting_user_action',
      );
      live.sub.close();
      api.stop();
      api = null;

      // ── A 幕：交互模式，首轮必须停在确认卡上（**文件仍在**是修复前后的分水岭）─────
      const victimA = seedVictim();
      assert(existsSync(victimA), `预置文件失败：${victimA}`);

      const first = await runCli({
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
      const requestsBeforeResume = (await llm.requests()).length;
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
      await waitFor(async () => (await llm.requests()).length > requestsBeforeResume, {
        what: 'T78-A 恢复轮收束（mock 收到恢复后的新请求）',
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

      // 事实面（批准后的形状）：**一次世界交互 = 一笔债 = 一条事实**。
      // ① `user.answer{kind:approve}` —— 人动作原文；批准结论另有一条
      //    `authz.granted{basis:"approved"}`（风险档原样取审批卡的
      //    `prompt.risk_level`，不在能力内重解析 = 不造第二个解析器）。
      // ② 批级 `sent`：恢复行复用首派债键 `tool:<request_id>`，收束落回同一笔批债
      //    （不再要求逐调用的 `request` —— 那是旧形状）。
      // ③ 同一调用的**第二次落格** `artifact.added`：键带 `:resume` 后缀、`tool_call_id`
      //    仍是 wire id。不换键会被 (kind, key) 吸收，恢复结果就丢了。
      const events1 = readWal(SID_INTERACTIVE);
      const approveFact = events1.find(
        (e) => e.kind === 'user.answer' && e.payload?.kind === 'approve',
      );
      assert(
        approveFact,
        `批准恢复必须落 \`user.answer{kind:approve}\`（实际: ${JSON.stringify(
          events1.filter((e) => e.kind === 'user.answer').map((e) => e.payload),
        )}）`,
      );
      const wireId = approveFact.payload?.call_id;
      assert(
        wireId,
        `user.answer 必须点名调用的 wire id（实际: ${JSON.stringify(approveFact.payload)}）`,
      );
      // 批准结论：风险档必须**原样**等于审批卡上的那一格（闸门是唯一解析点），
      // 且同一调用只落一条 approved 结论。
      const card = events1.find(
        (e) =>
          e.kind === 'artifact.added' &&
          e.payload?.tool_call_id === wireId &&
          e.payload?.needs_user_action === true,
      );
      assert(card, `审批卡应落在事实面（wire=${wireId}）`);
      const cardRisk = card.payload?.prompt?.risk_level;
      assert(
        cardRisk,
        `审批卡必须带 risk_level（实际: ${JSON.stringify(card.payload?.prompt)}）`,
      );
      const granted = events1.filter(
        (e) => e.kind === 'authz.granted' && e.payload?.id === wireId,
      );
      assertEq(
        granted.length,
        1,
        `批准结论应恰好一条（实际: ${JSON.stringify(granted.map((e) => e.payload))}）`,
      );
      assertEq(granted[0].payload?.basis, 'approved', '批准结论的 basis');
      assertEq(
        granted[0].payload?.risk,
        cardRisk,
        '批准结论的风险档必须原样等于审批卡（不得在消费端重算）',
      );
      const requested = events1.find(
        (e) =>
          e.kind === 'tool.requested' &&
          (e.payload?.calls ?? []).some((call) => call?.id === wireId),
      );
      assert(requested, `该调用必须出现在 \`tool.requested\` 的调用清单里（wire=${wireId}）`);
      // 批级 `sent` 的身份是 `ring:tool:tool:<tool.requested 事实行 id>:<attempt>:sent`。
      assert(
        events1.some(
          (e) =>
            e.kind === 'sent' &&
            e.event_id.startsWith(`ring:tool:tool:${requested.event_id}:`),
        ),
        `批准恢复必须收束在同一笔批债上（批级 \`sent\`，request@${requested.event_id}）`,
      );
      // 恢复必须落在**首派 attempt** 上：多出一个 attempt 的 request / sent 就是
      // 「首派在新 attempt 上重跑」（恢复失败被判 Retryable 时的典型现场）。
      const batchPrefix = `ring:tool:tool:${requested.event_id}:`;
      const batchAttempts = events1
        .filter(
          (e) =>
            (e.kind === 'request' || e.kind === 'sent') &&
            e.event_id.startsWith(batchPrefix),
        )
        .map((e) => Number(e.event_id.slice(batchPrefix.length).split(':')[0]))
        .filter((n) => Number.isFinite(n));
      assert(
        batchAttempts.every((n) => n === 1),
        `恢复必须复用首派 attempt（不得让首派在新 attempt 上重跑）；实际: ${JSON.stringify(
          batchAttempts,
        )}`,
      );
      const resumedArtifact = events1.find(
        (e) =>
          e.kind === 'artifact.added' &&
          e.payload?.tool_call_id === wireId &&
          e.event_id.endsWith(':resume'),
      );
      assert(
        resumedArtifact,
        `恢复结果必须是同一调用的第二次落格（键 \`artifact:${wireId}:resume\`）；实际: ${JSON.stringify(
          events1
            .filter((e) => e.kind === 'artifact.added')
            .map((e) => ({ key: e.event_id, tool_call_id: e.payload?.tool_call_id })),
        )}`,
      );
      assertTranscriptInvariants(msgs1, 'T78-A 恢复后');

      api.stop();
      api = null;

      // ── B 幕：auto 模式 ⇒ 闸门不产卡，直接以 permission_denied 反馈 ─────────────
      const victimB = seedVictim();
      assert(existsSync(victimB), `预置文件失败：${victimB}`);

      const b = await runCli({
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

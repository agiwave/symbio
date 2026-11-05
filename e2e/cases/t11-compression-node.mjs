// T11 压缩的现行形态：压缩落一条 `session.consolidated` 事实（载荷里是压缩 Notice
// + 本轮终答），重建面从这份快照开始折；轮次以用户消息节点为锚（D-335），
// 实时面上没有独立的 Turn / 压缩节点。
import './_selfrun.mjs';
//
// ## 本用例钉的是什么
//
// - **压缩是一次落格，不是一个实时节点**：`compact` 端口写 `session.consolidated`，
//   载荷 `messages` 首节点是 `notice` + `meta.style_tag = compression`（正文 = 摘要），
//   其后是本轮终答；实时面（`event_bus` 的 `vdfs` 频道 + `vdfs/watch`）不出现
//   `compression` 帧。
// - **D-335：Turn 不是独立可见节点**：一轮的可见锚是用户消息节点，收束看它的终答
//   （assistant 正文到达终态）。
// - **落盘同源**：生产读出口读回的历史里有压缩节点、被压掉的前缀已不在。
//
// ## 触发参数
//
// mock 恒报 `prompt_tokens: 64`（见 `mock-llm.mjs`），上限取 80 ⇒ 64 ≥ 0.7 × 80，
// 每轮越线（t8/t18/t63 同一口径）。摘要由 `model.assist` 取：该端口把整段转写作为
// 最后一条 user 发来，以「助手：…」收尾，故 mock 用这个标记命中摘要场景。
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
  createSession,
  sendInbox,
  assertTranscriptInvariants,
  msgTypeOf,
  msgRoleOf,
  textOf,
} from '../helpers.mjs';

const TERMINAL = new Set(['completed', 'failed', 'aborted', 'waiting_user_action']);

const FILL = '上下文填充内容用于推高历史水位。';
const REPLY = '填充：' + FILL.repeat(600);
const SNAPSHOT_XML = [
  '<state_snapshot>',
  '  <overall_goal>验证上下文压缩链路端到端可用。</overall_goal>',
  '  <key_knowledge>',
  '    - 用户偏好：mock 环境下回复应包含轮次编号。',
  '  </key_knowledge>',
  '  <completed_items>',
  '    - 前几轮对话已完成。',
  '  </completed_items>',
  '  <in_progress_items>',
  '    - 等待压缩后的下一轮对话验证记忆延续。',
  '  </in_progress_items>',
  '  <open_questions></open_questions>',
  '</state_snapshot>',
].join('\n');

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

export default defineCase('T11 压缩的现行形态：session.consolidated 落格 + 重建收敛 + 无 compression 帧', async () => {
  const llm = await new MockLlm([
    { id: 'snapshot', match: '助手：', content: SNAPSHOT_XML },
    { id: 'big', match: '轮问题', content: REPLY },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 80 }) },
    ],
    pluginConfigs: {
      // 本用例的主题是压缩，与对话面正交 ⇒ 把两个开关钉死（理由见 `DIALOG_FACE_OFF`）。
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
    session: 'unused-t11',
    provider: PROVIDER_ID,
    gatewayPort: GATEWAY_PORT,
  });
  let rt = null;
  try {
    await cli.waitGatewayReady();

    // ① 订阅实时面（发消息之前，不漏首帧）
    rt = await subscribeSessionRealtime(cli, { sessionId: 'e2e-t11', gatewayPort: GATEWAY_PORT });

    // 会话先建出来：收件箱是会话内部的集合，会话不在就没有它可写
    await createSession(cli, 'e2e-t11', { workdir: hd.workdir });

    // D-335：一轮的可见终态落在终答节点上（assistant 正文到达终态），不数 Turn 节点。
    // 按 id 合并各帧后再判（`role` 只在携带它的那几帧里，终态帧未必重复带）。
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
          TERMINAL.has(m.status),
      ).length;
    };

    // ② 串行发 4 轮：每轮等「本轮终答到达终态」，避免并发发送把上一轮打成 aborted
    for (const [i, text] of ['第一轮问题', '第二轮问题', '第三轮问题', '第四轮问题'].entries()) {
      const before = doneTurns();
      const send = await sendInbox(cli, 'e2e-t11', text, { workdir: hd.workdir });
      assert(send.status === 200, `第 ${i + 1} 轮写收件箱应受理（${send.status}）`);
      await waitFor(() => doneTurns() > before, { what: `第 ${i + 1} 轮终答终态`, timeoutMs: 60_000 });
    }

    // ③ 压缩落格（async 债链，等事实源里出现那条事实）
    await waitFor(
      () => walEvents(hd.homedir, 'e2e-t11').some((e) => e.kind === 'session.consolidated'),
      { what: 'session.consolidated 落格', timeoutMs: 30_000 },
    );
    const consolidated = walEvents(hd.homedir, 'e2e-t11').filter(
      (e) => e.kind === 'session.consolidated',
    );
    assert(consolidated.length >= 1, '应至少落一条 session.consolidated');
    const nodes = consolidated[0].payload?.messages ?? [];
    assertEq(nodes[0]?.type, 'notice', '压缩快照首节点应是 notice');
    assertEq(nodes[0]?.meta?.style_tag, 'compression', '压缩快照首节点的 style_tag');
    assert(
      String(nodes[0]?.content ?? '').includes('state_snapshot'),
      `压缩快照正文应是摘要（实际首 60 字: ${String(nodes[0]?.content ?? '').slice(0, 60)}）`,
    );

    // ④ 实时面：有变更、但没有 compression 帧（压缩是落格事实，不是实时节点）
    const ops = rt.messages();
    const timeline = ops.map(
      ({ mid, data, arrival }) =>
        `#${arrival} [${data?.type ?? '-'}/${data?.role ?? '-'}/${data?.status ?? '-'}] ` +
        `${String(mid).slice(0, 8)}`,
    );
    assert(
      ops.length > 0,
      `实时面应有消息变更（event_bus/subscribe + vdfs/watch）\n时间线:\n${timeline.join('\n')}`,
    );
    assert(
      !ops.some(({ data }) => msgTypeOf(data) === 'compression'),
      `压缩是落格事实，不是实时节点；实时面不应出现 compression 帧\n时间线:\n${timeline.join('\n')}`,
    );

    // ⑤ 落盘同源：生产读出口读回的历史里有压缩节点、被压掉的前缀已不在
    const msgs = await readSessionMessages(cli, 'e2e-t11');
    assertTranscriptInvariants(msgs, 'T11');
    const comp = msgs.find((m) => msgTypeOf(m) === 'compression');
    assert(
      comp,
      `重建历史里应有压缩节点（现有 ${msgs.length} 条，类型: ${msgs.map((m) => msgTypeOf(m)).join(',')}）`,
    );
    assert(String(comp.content ?? '').includes('state_snapshot'), '重建出的压缩节点正文应是摘要');
    assert(
      !msgs.some((m) => msgRoleOf(m) === 'user' && textOf(m).includes('第一轮问题')),
      '被压掉的第一轮用户消息不应再出现在重建历史里',
    );
  } finally {
    rt?.close();
    cli.stop();
    llm.stop();
    cleanupHomedir(hd);
  }
});

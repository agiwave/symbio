// T8 上下文压缩：水位越过阈值 → `compact` 插件落 `session.consolidated`
// （摘要 Notice + 本轮终答），重建面从这份快照开始折。
import './_selfrun.mjs';
//
// ## 触发机制（D-336）
//
// `compact` 插件按 `model:<uid>` 收束载荷里的实测水位判债：阈值 = 0.7 × provider 上限。
// e2e 的 mock 恒报 `prompt_tokens: 64`（见 `mock-llm.mjs`），因此上限取 80
// ⇒ 64 ≥ 0.7 × 80，每轮都越线（取 16000 则永不越线——那是旧水位口径的夹具值）。
//
// ## 压缩的现行形态
//
// - 摘要由 `model.assist` 端口取（system = compact 插件的摘要指令，正文 = 按角色投影
//   的整段对话线）；
// - 结果由 `compact` 端口落 `session.consolidated`，载荷 `messages` =
//   `Notice`（`meta.style_tag = compression`，正文即摘要）+ 本轮终答；
// - 生产读出口（`vdfs/read` 会话叶 = 前端 `loadMessages` 同一出口）从这份快照开始
//   重建，被压掉的前缀不再出现。
//
// 本用例钉的不变量：
// - 真的落了一条 `session.consolidated`（触发线没越 ⇒ 这里红，不许跳过）；
// - 载荷首节点是 `notice` + `style_tag = compression`，正文是摘要；
// - 重建后的历史里被压掉的用户消息消失、压缩节点在；
// - 压缩不炸会话：触发轮回复照常完成。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  nextPort,
  startReadbackCli,
  readSessionMessages,
  sessionWalEvents,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
  msgTypeOf,
} from '../helpers.mjs';

const REPLY = `填充：${'上下文填充内容用于推高历史水位。'.repeat(600)}`;

const SNAPSHOT_XML = [
  '<state_snapshot>',
  '  <overall_goal>验证上下文压缩链路端到端可用。</overall_goal>',
  '  <key_knowledge>',
  '    - 用户偏好：mock 环境下回复应包含轮次编号。',
  '  </key_knowledge>',
  '  <completed_items>',
  '    - 前几轮对话已完成，回复内容均为大段填充文本。',
  '  </completed_items>',
  '  <in_progress_items>',
  '    - 等待压缩后的下一轮对话验证记忆延续。',
  '  </in_progress_items>',
  '  <open_questions></open_questions>',
  '</state_snapshot>',
].join('\n');


export default defineCase('T8 上下文压缩：水位触发 session.consolidated，重建历史归并', async () => {
  const llm = await new MockLlm([
    // 摘要请求（`model.assist`）：对话线与主请求同形（末条 user 也是「第N轮问题」），
    // 只有 system（compact 插件的摘要指令）能把它与主请求分开。
    { id: 'snapshot', matchSystem: '你是上下文压缩器', content: SNAPSHOT_XML },
    // 普通轮：回复超大文本，把（mock 固定回报的）水位顶过 70% 阈值
    { id: 'big', match: '轮问题', content: REPLY },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 80 }) },
    ],
    // context_messages: 0 = 上下文窗口不按轮次截断（压缩判定的原料是完整历史）
    pluginConfigs: {
      // 转写经生产读出口读（gateway `invoke` → `vdfs/read`），入站口必须开着。
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: GATEWAY_PORT,
        inbound_token: '',
        inbound_readonly: false,
      },
      session: { context_messages: 0 },
    },
  });
  let api = null;
  try {
    const r = await runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      provider: PROVIDER_ID,
      session: 'e2e-t8',
      stdinText: '第一轮问题\n第二轮问题\n第三轮问题\n第四轮问题\n',
      timeoutMs: 120_000,
    });
    assertEq(r.code, 0, `CLI 退出码（stderr 尾部: ${r.stderr.slice(-600)}）`);

    // ① 压缩真的发生：`compact` 插件落了一条 `session.consolidated`
    const events = sessionWalEvents(hd.homedir, 'e2e-t8');
    const consolidated = events.filter((e) => e.kind === 'session.consolidated');
    assert(
      consolidated.length >= 1,
      `应至少落一条 session.consolidated（水位 70% 未触发会在此暴露；现有 kind: ${[...new Set(events.map((e) => e.kind))].join(',')}）`,
    );

    // ② 载荷形态：首节点是压缩 Notice，正文就是摘要
    const nodes = consolidated[0].payload?.messages ?? [];
    assert(nodes.length >= 1, `session.consolidated 载荷 messages 不应为空（实得 ${nodes.length}）`);
    assertEq(nodes[0].type, 'notice', '压缩快照首节点应是 notice');
    assertEq(nodes[0].meta?.style_tag, 'compression', '压缩快照首节点的 style_tag');
    assert(
      String(nodes[0].content ?? '').includes('state_snapshot'),
      `压缩快照正文应是摘要（实际首 60 字: ${String(nodes[0].content ?? '').slice(0, 60)}）`,
    );

    // ③ 重建面（生产读出口）从快照开始折：压缩节点在、被压掉的前缀消失
    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t8',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t8');
    assertTranscriptInvariants(msgs, 'T8');
    const comp = msgs.find((m) => msgTypeOf(m) === 'compression');
    assert(
      comp,
      `重建历史里应有压缩节点（现有 ${msgs.length} 条，类型: ${msgs.map((m) => msgTypeOf(m)).join(',')}）`,
    );
    assert(
      String(comp.content ?? '').includes('state_snapshot'),
      '重建出的压缩节点正文应是摘要',
    );
    assert(
      !msgs.some((m) => msgRoleOf(m) === 'user' && String(m.content ?? '').includes('第一轮问题')),
      '被压掉的第一轮用户消息不应再出现在重建历史里',
    );

    // ④ 压缩不炸会话：压缩后主循环照常完成回复
    assert(
      r.stdout.includes('填充：'),
      `压缩触发的当前轮应照常完成回复（stdout 尾部: ${r.stdout.slice(-200)}）`,
    );
  } finally {
    api?.stop();
    llm.stop();
    cleanupHomedir(hd);
    // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '读回 CLI / mock-llm 子进程退出',
    });
  }
});

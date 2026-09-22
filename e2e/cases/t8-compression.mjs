// T8 上下文压缩：历史水位越过 70% 阈值 → 自动压缩 → 快照落库 + 历史归并。
import './_selfrun.mjs';
//
// 触发机制：provider 的 max_context_tokens 调小（水位 = 70% × 上限），mock 每轮
// 回复超大文本把水位顶过阈值；压缩请求以「Distill the conversation above…」
// 指令收尾（compression.rs::compression_instruction），mock 据此用 <state_snapshot>
// XML 应答。参数口径实测校准过（dbg 阶段）：
// - 请求级 overhead ≈ 4.5k tokens（system prompt + 20 个工具 schema）；
// - 每轮回复 ≈ 9.6k 字符 ≈ 9.6k tokens（CJK ≈ 1 token/字）；
// - 12000 上限：70% = 8400，第 3 轮开始时水位 ≈ 19k 稳过线，且压缩请求
//   （历史 ≈ 7.4k + overhead ≈ 4.5k < 12000）能通过「摘要请求自身不超限」预检。
// 钉的不变量：
// - 压缩 LLM 调用真实发生（mock 收到压缩请求）；
// - 快照消息落库：meta.compacted=true、protocol_version=v2、user 角色、
//   「[CONTEXT SNAPSHOT」前缀（历史记忆替身）；
// - 被压掉的消息从存储消失；
// - 压缩不炸会话：触发压缩的那一轮回复照常完成。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
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

export default defineCase('T8 上下文压缩：水位触发自动压缩，快照落库 + 历史归并', async () => {
  const llm = await new MockLlm([
    // 压缩请求：末条 user 是压缩指令（compression.rs 唯一措辞），返回结构化快照
    { id: 'snapshot', match: 'Distill the conversation above', content: SNAPSHOT_XML },
    // 普通轮：回复超大文本，把历史水位顶过 70% 阈值
    { id: 'big', match: '轮问题', content: REPLY },
  ]).start();
  const hd = makeHomedir({
    providers: [
      { id: PROVIDER_ID, config: providerConfig(llm.port, { max_context_tokens: 12000 }) },
    ],
    // context_messages: 0 = 上下文窗口不按轮次截断（压缩判定的原料是完整历史）
    pluginConfigs: { session: { auto_compress: true, context_messages: 0 } },
  });
  try {
    const r = runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      provider: PROVIDER_ID,
      session: 'e2e-t8',
      stdinText: '第一轮问题\n第二轮问题\n第三轮问题\n第四轮问题\n',
      timeoutMs: 120_000,
    });
    assertEq(r.code, 0, `CLI 退出码（stderr 尾部: ${r.stderr.slice(-600)}）`);

    const reqs = await llm.requests();

    // ① 压缩请求真实发生
    const compReqIdx = reqs.findIndex((q) =>
      (q.body?.messages ?? []).some(
        (m) => m.role === 'user' && String(m.content ?? '').includes('Distill the conversation above'),
      ),
    );
    assert(compReqIdx >= 0, 'mock-llm 应收到压缩请求（水位 70% 未触发或指令措辞漂移都会在此暴露）');
    // 压缩请求发生在最后一轮主请求之前（压缩在本轮 Turn 创建前）
    assert(compReqIdx < reqs.length - 1, '压缩请求应先于最后一轮主请求');

    // ② 快照消息落库
    const msgs = readMessagesJson(hd.homedir, 'e2e-t8');
    assertTranscriptInvariants(msgs, 'T8');
    const snap = msgs.find((m) => m.meta?.compacted === true);
    assert(snap, `应有 meta.compacted=true 的快照消息（现有 ${msgs.length} 条）`);
    assertEq(snap.meta.protocol_version, 'v2', '快照 meta.protocol_version');
    assertEq(snap.role, 'user', '快照必须是 user 角色（多数 provider 要求对话以 user 开头）');
    assert(
      String(snap.content ?? '').includes('CONTEXT SNAPSHOT'),
      '快照内容应是「[CONTEXT SNAPSHOT …」前缀的历史记忆替身',
    );

    // ③ 历史归并：被压掉的消息消失（第一轮 user 消息不在存储里）
    const firstUserGone = !msgs.some(
      (m) => m.role === 'user' && String(m.content ?? '').includes('第一轮问题'),
    );
    assert(firstUserGone, '第一轮用户消息应已被压缩出存储');

    // ④ 压缩不炸会话：压缩后主循环照常完成回复
    assert(
      r.stdout.includes('填充：'),
      `压缩触发的当前轮应照常完成回复（stdout 尾部: ${r.stdout.slice(-200)}）`,
    );
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});

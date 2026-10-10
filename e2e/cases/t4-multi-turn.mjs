import './_selfrun.mjs';
// T4 多轮会话：第二轮请求携带第一轮历史（flatten 验证）+ 双源对账（轨 S 第一步）。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  readFileSyncSafe,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  textOf,
  assertTranscriptInvariants,
} from '../helpers.mjs';

export default defineCase('T4 多轮会话：第二轮请求携带第一轮历史', async () => {
  const llm = await new MockLlm([
    { id: 'r1', match: '第一轮', content: '第一轮回复' },
    { id: 'r2', match: '第二轮', content: '第二轮回复' },
  ]).start();
  // 本用例的主题与对话面正交 ⇒ 把两个开关钉死（理由见 `DIALOG_FACE_OFF`）。
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
    pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
  });
  try {
    // REPL 管道模式：两行输入 = 两轮对话
    const r = runCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      provider: PROVIDER_ID,
      session: 'e2e-t4',
      stdinText: '第一轮问题\n第二轮问题\n',
    });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
    assert(r.stdout.includes('第一轮回复') && r.stdout.includes('第二轮回复'), 'stdout 应含两轮回复');

    const reqs = await llm.requests();
    assertEq(reqs.length, 2, 'mock-llm 应收到两次请求');
    assert(
      reqs[1].body.messages.some((m) => m.role === 'user' && textOf(m).includes('第一轮问题')),
      '第二轮请求应携带第一轮用户消息（历史回灌）',
    );

    const msgs = readMessagesJson(hd.homedir, 'e2e-t4');
    assertTranscriptInvariants(msgs, 'T4');
    assertEq(msgs.filter((m) => m.role === 'user').length, 2, '落盘应有两条 user 消息');
            // ── 双源对账（轨 S 第一步，[ADR-049](../../docs/decisions/session.md#adr-049-wal-是会话唯一事实源sessionstore-降为缓存轨-s-的起点)）──
            // 会话消息今天有两份东西在记：messages.json（SessionStore，整份覆盖）与
            // v2-events.wal（append-only 的事实源）。S1 让读路径**并读两源、只记不断流**，
            // 本段证的是生产路径上此刻一致——这是"并读"能成立的前提。
            //
            // 口径与 `plugins/session/v2_reconcile.rs::reconcile` 逐字同源：`(role, 正文)`
            // 序列。呈现态（seq / parent_id / status）**不参与**——ADR-049 明确它们不入格、
            // 也不是对账对象（拿它们比会把「呈现形态不同」误报成「事实不一致」）。
            const walRaw = readFileSyncSafe(join(hd.homedir, 'session', 'e2e-t4', 'v2-events.wal'));
            assert(walRaw.length > 0, 'v2 事实源应存在且非空');
            const events = walRaw.split('\n').filter(Boolean).map((l) => JSON.parse(l));

            const storeSide = msgs
              .filter((m) => m.role && ['user', 'assistant', 'tool', 'system'].includes(m.role))
              .map((m) => [m.role, String(textOf(m) ?? '').trim()])

              // 空正文的节点**不参与对账**：messages.json 里每轮有一条空 assistant
              // （turn 容器 / 流式占位），WAL 侧没有对应事件——那是**呈现态**（节点树
              // 形状），ADR-049 明确它不入格也不是对账对象。e2e 探针实测了这一格
              // （store 6 条 vs wal 4 条，差的两条正是空 assistant）。
              .filter(([, text]) => text.length > 0);
            const walSide = events
              .filter((e) =>
                ['user.message', 'chat.assistant.final', 'chat.assistant.fallback'].includes(e.kind),
              )
              .map((e) => [
                e.kind === 'user.message' ? 'user' : 'assistant',
                String(e.payload?.text ?? '').trim(),
              ]);

            for (let i = 0; i < storeSide.length; i++) {
              assertEq(storeSide[i][0], walSide[i][0], `第 ${i} 条的 role 双源一致`);
              assertEq(
                storeSide[i][1],
                walSide[i][1],
                `第 ${i} 条的正文双源一致（这一格分叉就是「同一句话两个版本」）`,
              );
            }

  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});

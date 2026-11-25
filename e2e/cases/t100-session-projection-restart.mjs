import './_selfrun.mjs';
// T100 会话投影归位：两支持久化、切支、冷重启后，转写与选中态逐形重建（D-561）。
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  startRealtimeSession,
  startReadbackCli,
  sendAndSettle,
  readSessionMessages,
  resumeSession,
  waitFor,
  settledAfterWorking,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
  textOf,
} from '../helpers.mjs';

const SID = 'e2e-t100';
const QUESTION = 'T100 分支重建问题';
const ANSWER_A = 'T100 甲支答复';
const ANSWER_B = 'T100 乙支答复';

const runsOf = (messages) => (messages ?? []).filter((m) => m.meta?.branch);
const corpusOf = (messages) => messages.map((m) => textOf(m)).join('\n');

/** 剔除 seq / timestamp 等呈现细节，保留节点身份、树边、正文与分支读法。 */
function shapeOf(messages) {
  return messages.map((m) =>
    [
      m.id ?? '',
      m.parent_id ?? '',
      msgRoleOf(m) ?? '',
      m.type ?? m.msg_type ?? '',
      m.status ?? '',
      m.meta?.tool_name ?? '',
      m.tool_call_id ?? '',
      textOf(m),
      JSON.stringify(m.meta?.branch ?? null),
    ].join('\u0001'),
  );
}

export default defineCase(
  'T100 会话投影归位：切支后冷重启，两支持久化且转写与选中态逐形重建',
  async () => {
    const llm = await new MockLlm([
      { id: 'first', match: QUESTION, once: true, content: ANSWER_A },
      { id: 'retry', content: ANSWER_B },
    ]).start();
    const gatewayPort = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: gatewayPort,
          inbound_token: '',
          inbound_readonly: false,
        },
        ...dialogFaceOff(),
      },
    });

    let rt = null;
    let reopened = null;
    try {
      rt = await startRealtimeSession({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort,
      });
      const { cli, sub } = rt;

      await sendAndSettle(cli, sub, QUESTION);
      const first = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(first, 'T100 首支');
      const users = first.filter((m) => msgRoleOf(m) === 'user');
      assertEq(users.length, 1, '首轮只产生一个用户消息锚');
      const rootA = users[0].id;

      const retryMark = sub.sessionChanges().length;
      const retry = await resumeSession(cli, SID, rootA, 'retry_subtree', {}, { session_id: SID });
      assert(
        retry.status === 200 && retry.body?.data?.ok === true,
        `retry_subtree 应被受理（${JSON.stringify(retry.body)}）`,
      );
      await waitFor(
        async () => runsOf(await readSessionMessages(cli, SID)).length === 2,
        { what: '第二支进入转写', timeoutMs: 30_000 },
      );
      await waitFor(
        () => settledAfterWorking(sub.sessionChanges().slice(retryMark)),
        { what: '重试轮跑完', timeoutMs: 60_000 },
      );

      const branches = runsOf(await readSessionMessages(cli, SID));
      assertEq(branches.length, 2, '重试后两支部件都在树里');
      const rootB = branches.find((m) => m.id !== rootA)?.id;
      assert(rootB, '应能找到重试开出的第二支锚');
      const beforeSwitch = await readSessionMessages(cli, SID);
      const corpusBeforeSwitch = corpusOf(beforeSwitch);
      assert(corpusBeforeSwitch.includes(ANSWER_A), '旧支答复仍在');
      assert(corpusBeforeSwitch.includes(ANSWER_B), '新支答复也在');

      const switched = await resumeSession(
        cli,
        SID,
        rootB,
        'switch_branch',
        { to: rootA },
        { session_id: SID },
      );
      assert(
        switched.status === 200 && switched.body?.data?.ok === true,
        `switch_branch 应被受理（${JSON.stringify(switched.body)}）`,
      );
      const beforeRestart = await readSessionMessages(cli, SID);
      assertTranscriptInvariants(beforeRestart, 'T100 切支后');
      assertEq(runsOf(beforeRestart).length, 2, '切支不开轮、不删候选');
      assertEq(
        runsOf(beforeRestart).find((m) => m.id === rootA)?.meta?.branch?.selected,
        true,
        '重启前读法已切到甲支',
      );

      sub.close();
      cli.stop();
      await waitFor(() => cli.out.code !== '', { what: 'T100 第一进程退出' });

      reopened = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort,
      });
      const afterRestart = await readSessionMessages(reopened, SID);
      assertTranscriptInvariants(afterRestart, 'T100 冷重启');
      assertEq(
        shapeOf(afterRestart).join('\n'),
        shapeOf(beforeRestart).join('\n'),
        '冷重启后节点身份、树边、正文与分支读法必须逐形一致',
      );
      assertEq(runsOf(afterRestart).length, 2, '重启后两支都从事实重建，不丢候选');
      assertEq(
        runsOf(afterRestart).filter((m) => m.meta?.branch?.selected).length,
        1,
        '每组恰有一个选中支，不得全选或全不选',
      );
      assertEq(
        runsOf(afterRestart).find((m) => m.id === rootA)?.meta?.branch?.selected,
        true,
        '选中态跨重启保留在甲支',
      );
      assertEq(
        runsOf(afterRestart).find((m) => m.id === rootB)?.meta?.branch?.selected,
        false,
        '乙支仍是未选中候选',
      );
      const corpus = corpusOf(afterRestart);
      assert(corpus.includes(ANSWER_A) && corpus.includes(ANSWER_B), '两支答复都不得在重建时丢失');
      assertEq(
        afterRestart.filter((m) => msgRoleOf(m) === 'user').map((m) => m.id).sort(),
        [rootA, rootB].sort(),
        '两支各以用户消息节点为轮锚，不产生额外的用户轮锚',
      );
    } finally {
      rt?.sub?.close();
      rt?.cli?.stop();
      reopened?.stop();
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: 'T100 长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

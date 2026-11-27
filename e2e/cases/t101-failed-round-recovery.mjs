import './_selfrun.mjs';
// T101 失败轮恢复：用户轮锚声明整轮重试，修改持久化后建支且保留旧失败历史。
import {
  MockLlm,
  makeHomedir,
  dialogFaceOff,
  cleanupHomedir,
  nextPort,
  startRealtimeSession,
  sendAndSettle,
  readSessionMessages,
  resumeSession,
  sessionAddr,
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

const SID = 'e2e-t101';
const QUESTION = '触发失败的问题';
const EDITED = '修改后重试的问题';
const ANSWER = '新支的正常答复';

export default defineCase('T101 失败轮恢复：动作经读出口下发，修改后重试保留旧支并另开新支', async () => {
  const llm = await new MockLlm([
    { id: 'failure', match: QUESTION, status: 500, error: 'mock 注入的服务端错误' },
    { id: 'recovered', match: EDITED, once: true, content: ANSWER },
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
      ...dialogFaceOff(),
    },
  });

  let rt = null;
  try {
    rt = await startRealtimeSession({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: SID,
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const { cli, sub } = rt;
    await sendAndSettle(cli, sub, QUESTION);

    const failed = await readSessionMessages(cli, SID);
    assertTranscriptInvariants(failed, 'T101 失败轮');
    const failedUsers = failed.filter((message) => msgRoleOf(message) === 'user');
    assertEq(failedUsers.length, 1, '现代失败轮只应有一个用户 body 轮锚');
    const failedRoot = failedUsers[0];
    assertEq(failedRoot.status, 'failed', `用户轮锚应随整轮落入失败终态：${JSON.stringify(failed)}`);
    assertEq(failedRoot.actions, ['retry_subtree'], '生产读出口必须交付整轮恢复动作');
    assertEq(textOf(failedRoot), QUESTION, '失败轮正文保持原样');
    const failedReply = failed.find((message) => msgRoleOf(message) !== 'user' && message.status === 'failed');
    assert(failedReply, '失败轮应有可归属的助手失败正文');
    assertEq(failedReply.parent_id, failedRoot.id, '失败正文必须挂在用户轮锚下，才能随分支一起隐藏');

    const updated = await cli.invoke(
      'vdfs/write',
      { path: `${await sessionAddr(cli, SID)}/message/${failedRoot.id}`, text: JSON.stringify({ ...failedRoot, content: EDITED }) },
      { session_id: SID },
    );
    assert(updated.status === 200, `修改用户轮锚应受理（${JSON.stringify(updated.body)}）`);

    const retryMark = sub.sessionChanges().length;
    const retry = await resumeSession(cli, SID, failedRoot.id, 'retry_subtree', {}, { session_id: SID });
    assert(
      retry.status === 200 && retry.body?.data?.ok === true,
      `retry_subtree 应以用户轮锚被受理（${JSON.stringify(retry.body)}）`,
    );
    await waitFor(
      () => settledAfterWorking(sub.sessionChanges().slice(retryMark)),
      { what: '修改后重试的新支跑完', timeoutMs: 60_000 },
    );

    const recovered = await readSessionMessages(cli, SID);
    assertTranscriptInvariants(recovered, 'T101 重试后');
    const users = recovered.filter((message) => msgRoleOf(message) === 'user');
    assertEq(users.length, 2, '重试应新建用户轮锚，而不是覆盖或删除旧轮');
    const oldRoot = users.find((message) => message.id === failedRoot.id);
    const newRoot = users.find((message) => message.id !== failedRoot.id);
    assert(oldRoot && newRoot, '旧支与新支都必须保留在转写中');
    assertEq(textOf(oldRoot), EDITED, '旧支保留已持久化的修改，失败答复与外部副作用不被回滚');
    assertEq(textOf(newRoot), EDITED, '新支必须使用已持久化的修改内容');
    assert(oldRoot.meta?.branch && newRoot.meta?.branch, '两支都应带分支事实');
    assertEq(oldRoot.meta.branch.total, 2, '旧支与新支属于同一分支组');
    assertEq(newRoot.meta.branch.selected, true, '新支是当前读法');
    const corpus = recovered.map(textOf).join('\n');
    assert(oldRoot.error?.includes('HTTP 500'), '旧失败交代仍然可见');
    assert(recovered.some((message) => message.status === 'failed' && msgRoleOf(message) !== 'user'), '旧支的失败正文仍保留');
    assert(corpus.includes(ANSWER), '新支正常答复已落库');

    const requests = await llm.requests();
    const retryPrompt = (requests.at(-1)?.body?.messages ?? [])
      .filter((message) => message.role !== 'system')
      .map((message) => typeof message.content === 'string' ? message.content : '')
      .join('\n');
    assert(retryPrompt.includes(EDITED), '模型请求包必须读到修改后的问题');
    assert(!retryPrompt.includes(ANSWER), '重试请求不应读到新支自己的答复');
  } finally {
    rt?.sub?.close();
    rt?.cli?.stop();
    llm.stop();
    cleanupHomedir(hd);
    await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
      what: '长驻 CLI / mock-llm 子进程退出',
    });
  }
});

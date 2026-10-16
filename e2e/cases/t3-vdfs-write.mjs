import './_selfrun.mjs';
// T3 内置工具回路：vdfs_write 真实写入工作目录。
import { join } from 'node:path';
import { readFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  nextPort,
  startReadbackCli,
  readSessionMessages,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgTypeOf,
} from '../helpers.mjs';

export default defineCase('T3 工具回路（内置 vdfs_write）：文件真实写入工作目录', async () => {
  const llm = await new MockLlm([
    {
      id: 'call-write',
      match: '写文件',
      toolCalls: [{ id: 'call_w1', name: 'vdfs_write', arguments: { path: 'notes.md', text: '# e2e 标题\n由 mock 写入' } }],
    },
    { id: 'after-write', afterTool: true, content: '文件已写入。' },
  ]).start();
  const GATEWAY_PORT = nextPort();
  const hd = makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
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
    },
  });
  let api = null;
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '帮我写文件', provider: PROVIDER_ID, session: 'e2e-t3' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);

    const written = readFileSync(join(hd.workdir, 'notes.md'), 'utf8');
    assert(written.includes('# e2e 标题'), `文件内容应写入工作目录（实际: ${JSON.stringify(written)}）`);

    api = await startReadbackCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'e2e-t3',
      provider: PROVIDER_ID,
      gatewayPort: GATEWAY_PORT,
    });
    const msgs = await readSessionMessages(api, 'e2e-t3');
    assertTranscriptInvariants(msgs, 'T3');
    const tc = msgs.find((m) => msgTypeOf(m) === 'tool_call');
    assert(tc, '应有 tool_call 节点');
    assertEq(tc.meta?.tool_name, 'vdfs_write', '工具名应为解析后的能力名');
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

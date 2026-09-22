import './_selfrun.mjs';
// T3 内置工具回路：vdfs_write 真实写入工作目录。
import { join } from 'node:path';
import { readFileSync } from 'node:fs';
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

export default defineCase('T3 工具回路（内置 vdfs_write）：文件真实写入工作目录', async () => {
  const llm = await new MockLlm([
    {
      id: 'call-write',
      match: '写文件',
      toolCalls: [{ id: 'call_w1', name: 'vdfs_write', arguments: { path: 'notes.md', text: '# e2e 标题\n由 mock 写入' } }],
    },
    { id: 'after-write', afterTool: true, content: '文件已写入。' },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '帮我写文件', provider: PROVIDER_ID, session: 'e2e-t3' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);

    const written = readFileSync(join(hd.workdir, 'notes.md'), 'utf8');
    assert(written.includes('# e2e 标题'), `文件内容应写入工作目录（实际: ${JSON.stringify(written)}）`);

    const msgs = readMessagesJson(hd.homedir, 'e2e-t3');
    assertTranscriptInvariants(msgs, 'T3');
    const tc = msgs.find((m) => m.type === 'tool_call');
    assert(tc, '应有 tool_call 节点');
    assertEq(tc.name, 'vdfs_write', '工具名应为解析后的能力名');
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});

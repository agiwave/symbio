// S-end③ 语料捕获：从**出厂真实链路**拿一份原样请求体，作为两个臂的共同输入。
//
// 为什么不在脚本里手写 prompt：本实验要回答的是「**出厂形状**对模型是好是坏」，
// 手写一份「长得像出厂」的 prompt 会让结论落在夹具上而不是产品上。捕获口径与
// [ADR-057](../../docs/adr/ADR-057.md) 的形状基线同源 —— 同一条链路
// （CLI × mock-llm × mock-mcp）、同一个场景名（对话面显式关闭）、同一份原样请求体。
//
// 产物是一份 JSON：`{ captured_at, scenario, request }`，`request` = mock-llm 收到的
// **第一个**请求体（两条 system 前缀 + user + 出厂 `tools` 数组 + 信封）。
// 落哪由 `--out=` 决定；不入库（里面是本次运行的临时 homedir 绝对路径）。
import { join } from 'node:path';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  nextPort,
  assert,
  providerConfig,
  PROVIDER_ID,
  waitFor,
} from '../helpers.mjs';

/** 与形状基线同一个会话 id：它出现在 system 前缀的记忆地址里 */
const SID = 'e2e-t46';
/** 场景名明写进产物，报告里可核对本语料是否还是那条链路跑出来的 */
const SCENARIO = 'full-主线装配-工具轮-对话面关闭';
/** 捕获用的提问：只要求「列出当前目录」，让链路必然走到工具面 */
const CAPTURE_PROMPT = '列一下当前目录里有什么文件';

const out = process.argv.slice(2).find((a) => a.startsWith('--out='))?.slice(6);

const llm = await new MockLlm([{ id: 'capture', content: '已列出。' }]).start();
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
addMcpServer(hd, 'mockserv', {
  type: 'stdio',
  command: process.execPath,
  args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs'), '--record', join(hd.homedir, 'mcp-record.ndjson')],
  enabled: true,
});

try {
  const r = runCli({
    homedir: hd.homedir,
    workdir: hd.workdir,
    provider: PROVIDER_ID,
    session: SID,
    stdinText: `${CAPTURE_PROMPT}\n`,
  });
  assert(r.code === 0, `CLI 退出码非 0（stderr: ${r.stderr.slice(0, 600)}）`);
  const reqs = await llm.requests();
  assert(reqs.length >= 1, '链路没有产出任何请求');
  const body = reqs[0].body;
  assert(Array.isArray(body.tools) && body.tools.length > 0, '出厂请求体里没有 tools 数组');
  const dst = out ?? join(tmpdir(), 'symbio-ab-corpus.json');
  writeFileSync(dst, JSON.stringify({ captured_at: new Date().toISOString(), scenario: SCENARIO, capture_prompt: CAPTURE_PROMPT, request: body }));
  console.log(`已捕获：${dst}`);
  console.log(`  system ${body.messages.filter((m) => m.role === 'system').length} 条 · tools ${body.tools.length} 个 · 请求体 ${JSON.stringify(body).length}B`);
} finally {
  llm.stop();
  cleanupHomedir(hd);
  await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
    what: 'CLI / mock-llm / mock-mcp 子进程退出',
  });
}

import './_selfrun.mjs';
// T12 请求头钉死：LLM POST 必须带 `Content-Type: application/json`。
//
// 回归背景：§3.1 把 `execute_post_with_abort` 从收 `&Value`（reqwest 的
// `.json()` 会自动补 Content-Type）改成收 `&[u8]` + `.body()`，后者**不会**
// 自动补。漏补会让真实服务端按默认 `text/plain` 拒收。这里是唯一的回归位：
// e2e mock 不校验 Content-Type，漏了也不会让其他用例红——所以单独钉死。
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

export default defineCase('T12 LLM POST 必须带 Content-Type: application/json', async () => {
  const llm = await new MockLlm([
    { id: 'echo', match: '你好', chunks: ['你好', '，', 'mock'], chunkDelayMs: 5 },
  ]).start();
  const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
  try {
    const r = runCli({ homedir: hd.homedir, workdir: hd.workdir, message: '你好', provider: PROVIDER_ID, session: 'e2e-t12' });
    assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);

    const reqs = await llm.requests();
    assert(reqs.length >= 1, 'mock-llm 至少应收到一次请求');
    for (const req of reqs) {
      if (!req.path.endsWith('/chat/completions')) continue;
      const ct = (req.contentType ?? '').toLowerCase();
      assert(
        ct.includes('application/json'),
        `chat/completions 请求必须带 Content-Type: application/json（实际: ${req.contentType ?? '缺失'} @ ${req.path}）`,
      );
    }
  } finally {
    llm.stop();
    cleanupHomedir(hd);
  }
});

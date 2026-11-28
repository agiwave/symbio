import './_selfrun.mjs';
// T116 CLI 内嵌前端：`symbio-cli serve` 默认从二进制内嵌的 packages/ui/dist 提供 UI。
//
// 判据取「另一侧的观测」而不是读源码：
// 1. 从**不含 dist 的临时 cwd** 启动 serve（也不传 --frontend-dist）——若仍从磁盘
//    相对路径取产物，这里必然 404；能返回 index.html 就证明资源在二进制里。
// 2. 取 UI 引用的 JS 资源，确认 MIME 与内容非空（内嵌表不是只有空壳 index）。
// 3. SPA 回退：任意前端路由仍返回 HTML，不是 JSON 404。

import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  cliExe,
  makeHomedir,
  cleanupHomedir,
  nextPort,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

export default defineCase(
  'T116 CLI serve 内嵌前端：不依赖外部 dist 目录也能提供完整 UI',
  async () => {
    const port = nextPort();
    const hd = makeHomedir();
    // 关键：cwd 是一个全新的空目录，既没有 packages/ui/dist 也没有 index.html。
    const emptyCwd = mkdtempSync(join(tmpdir(), 'symbio-e2e-nodist-'));
    const child = spawn(cliExe(), ['serve', '--bind', '127.0.0.1', '--port', String(port), '--homedir', hd.homedir], {
      cwd: emptyCwd,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stderr = '';
    child.stderr.on('data', (d) => { stderr += d.toString(); });
    const base = `http://127.0.0.1:${port}`;

    try {
      // 等 serve 就绪
      const deadline = Date.now() + 20_000;
      let ready = false;
      while (Date.now() < deadline) {
        try {
          const r = await fetch(`${base}/api/v1/health`);
          if (r.ok) { ready = true; break; }
        } catch { /* 尚未监听 */ }
        await new Promise((r) => setTimeout(r, 100));
      }
      assert(ready, `serve 未在 20s 内就绪；stderr: ${stderr.slice(-500)}`);

      const index = await fetch(`${base}/`);
      assertEq(index.status, 200, '内嵌 index.html 应返回 200');
      assert((index.headers.get('content-type') ?? '').includes('text/html'), 'index 应为 HTML');
      const html = await index.text();
      assert(html.includes('<div id="app"></div>') || html.includes('id="app"'), '内嵌 index 应含应用挂载点');

      // 从 HTML 里取一个真实 JS 资源，验证内嵌表提供了完整内容
      const jsRef = /src="\/(assets\/[^"]+\.js)"/.exec(html)?.[1];
      assert(jsRef, '内嵌 index 应引用打包后的 JS 资源');
      const js = await fetch(`${base}/${jsRef}`);
      assertEq(js.status, 200, `内嵌 JS 资源 ${jsRef} 应返回 200`);
      assert((js.headers.get('content-type') ?? '').includes('javascript'), 'JS 资源 MIME 应为 javascript');
      assert((await js.text()).length > 1000, '内嵌 JS 资源不应为空壳');

      // SPA 回退：前端路由未命中时仍返回 index，而不是 JSON 404
      const spa = await fetch(`${base}/sessions/any-route`);
      assertEq(spa.status, 200, 'SPA 路由应回退到内嵌 index.html');
      assert((await spa.text()).includes('id="app"'), 'SPA 回退应返回应用 HTML');
    } finally {
      child.kill();
      try {
        rmSync(emptyCwd, { recursive: true, force: true });
      } catch {
        // Windows 上 libuv 句柄拆除比进程退出慢（同 cleanupHomedir），留给系统临时目录清理。
      }
      cleanupHomedir(hd);
    }
  },
);

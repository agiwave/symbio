import './_selfrun.mjs';
// T67 `content_search` 边界批（T-111 的①②）：
//   - `output_mode` 三取值（`content` / `files_with_matches` / `count`）的输出形状；
//   - `head_limit` / `offset` 按**行**分页（对齐 Trae Grep）；
//   - 输出超过 `MAX_OUTPUT_BYTES`（1 MiB）时的截断与尾注。
//
// ## 为什么这些边界值得单独一条用例
//
// T64 钉的是「工具真的读了磁盘」与「守 ignore / 隐藏文件语义」。这一条钉的是
// **参数取值**：分页参数写错**不报错**，只是结果少了（「看着像对的、其实少了一半」）；
// `output_mode` 传错则整段结果换形状。判据都取模型收到的 `role=tool` 正文——
// 与 T64 同一条用户可见出口，不引入新的读数面。
//
// ## 截断那条为什么用「大文件」而不是「很多文件」
//
// `content_search::search` 有两条独立的限制：匹配数 `MAX_RESULTS = 1000`（先触发，
// 加尾注「[结果已限制为前 1000 条匹配]」）与输出字节 `MAX_OUTPUT_BYTES = 1 MiB`
// （后触发，截断正文 + 尾注「[输出已截断：超过 1048576 字节]」）。要让**字节**那条
// 触发，就得在 1000 行的匹配上限内把字节数堆过 1 MiB ⇒ 每行必须 ~1 KB。
import { join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
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
  msgRoleOf,
} from '../helpers.mjs';

/** 随机标记词：只存在于工作区文件里，模型侧无从得知 ⇒ 「出现」即「真读了盘」。 */
function marker(prefix) {
  return `${prefix}_${Math.random().toString(36).slice(2, 10).toUpperCase()}`;
}

const MARK_A = marker('MKA'); // 命中 a.ts（3 行）/ b.ts（2 行）
const MARK_B = marker('MKB'); // 命中 big.txt（1000 行，逼出 1 MiB 截断）

/** `content` 模式的总匹配行数（a.ts 3 + b.ts 2）。 */
const TOTAL_MATCH_LINES = 5;
/** 每行 ~1.1 KB × 1000 行 > 1 MiB（且匹配数不超 `MAX_RESULTS = 1000`）。 */
const BIG_LINES = 1000;
const BIG_LINE_BYTES = 1100;

export default defineCase(
  'T67 content_search 边界：三种 output_mode 形状 · 按行分页 · 1 MiB 输出截断',
  async () => {
    const llm = await new MockLlm([
      // ⚠️ 启动器带唯一 `match`（夹具陷阱 10）。
      {
        id: 'mode-content',
        match: '内容模式',
        toolCalls: [
          { id: 'c_content', name: 'content_search', arguments: { pattern: MARK_A, output_mode: 'content' } },
        ],
      },
      // ⚠️ 每个 `afterTool` 场景都带 `once`（夹具陷阱 14：不带就吃掉每一个工具结果轮 ⇒ 无限循环）。
      {
        id: 'mode-files',
        afterTool: true,
        once: true,
        toolCalls: [
          { id: 'c_files', name: 'content_search', arguments: { pattern: MARK_A, output_mode: 'files_with_matches' } },
        ],
      },
      {
        id: 'mode-count',
        afterTool: true,
        once: true,
        toolCalls: [
          { id: 'c_count', name: 'content_search', arguments: { pattern: MARK_A, output_mode: 'count' } },
        ],
      },
      {
        id: 'paged',
        afterTool: true,
        once: true,
        toolCalls: [
          { id: 'c_page', name: 'content_search', arguments: { pattern: MARK_A, offset: 1, head_limit: 2 } },
        ],
      },
      {
        id: 'truncated',
        afterTool: true,
        once: true,
        toolCalls: [{ id: 'c_trunc', name: 'content_search', arguments: { pattern: MARK_B } }],
      },
      { id: 'final', afterTool: true, content: '边界都验完了。' },
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
        session: { ...DIALOG_FACE_OFF },
      },
    });

    // 工作区：a.ts（3 行含 MARK_A）· b.ts（2 行含 MARK_A）· c.ts（不含 ⇒ 任何结果都不该出现）
    //         · big.txt（1000 行 × ~1.1 KB 含 MARK_B，逼出输出截断）
    mkdirSync(join(hd.workdir, 'src'), { recursive: true });
    writeFileSync(
      join(hd.workdir, 'src', 'a.ts'),
      `const x1 = '${MARK_A}';\nconst x2 = '${MARK_A}';\nconst x3 = '${MARK_A}';\n`,
    );
    writeFileSync(join(hd.workdir, 'src', 'b.ts'), `const y1 = '${MARK_A}';\nconst y2 = '${MARK_A}';\n`);
    writeFileSync(join(hd.workdir, 'src', 'c.ts'), 'const z = 1;\n');
    const pad = 'x'.repeat(BIG_LINE_BYTES);
    writeFileSync(
      join(hd.workdir, 'big.txt'),
      Array.from({ length: BIG_LINES }, (_, i) => `${MARK_B} line ${i} ${pad}`).join('\n') + '\n',
    );

    let api = null;
    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我搜一下内容模式',
        provider: PROVIDER_ID,
        session: 'e2e-t67',
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      assert(r.stdout.includes('边界都验完了'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

      // 六次请求：用户轮 + 五个工具结果轮。
      const reqs = await llm.requests();
      assertEq(reqs.length, 6, `应六次请求（用户轮 + 五次工具结果轮），实际 ${reqs.length}`);

      // ── ① `output_mode: content`：每行 `<路径>:<行号>:<行内容>` ──
      const contentOut = toolResultText(reqs[1]);
      assert(
        new RegExp(`a\\.ts:\\d+:.*${MARK_A}`).test(contentOut),
        `content 模式应是 \`<路径>:<行号>:<内容>\` 形状（实际: ${contentOut.slice(0, 400)}）`,
      );
      assert(
        contentOut.includes('b.ts'),
        `content 模式应命中 b.ts（实际: ${contentOut.slice(0, 400)}）`,
      );
      assert(
        !contentOut.includes('c.ts'),
        `不含标记的 c.ts 不该进结果（实际: ${contentOut.slice(0, 400)}）`,
      );
      assertEq(
        contentOut.split('\n').filter((l) => l.includes(MARK_A)).length,
        TOTAL_MATCH_LINES,
        'content 模式每个匹配行一条 ⇒ 共 5 行',
      );
      assert(
        !contentOut.includes('\n\n'),
        `content 模式行间应单换行、无空行（T-113，实际片段: ${JSON.stringify(contentOut.slice(0, 200))}）`,
      );

      // ── ② `output_mode: files_with_matches`：每文件一条**纯路径**（无行号）──
      const filesOut = toolResultText(reqs[2]);
      assert(filesOut.includes('a.ts'), `files_with_matches 应含 a.ts（实际: ${filesOut.slice(0, 300)}）`);
      assert(filesOut.includes('b.ts'), `files_with_matches 应含 b.ts（实际: ${filesOut.slice(0, 300)}）`);
      assert(!filesOut.includes('c.ts'), '不含标记的 c.ts 不该进结果');
      assert(
        !new RegExp(`a\\.ts:\\d+`).test(filesOut),
        `files_with_matches 是纯路径、不带行号（实际: ${filesOut.slice(0, 300)}）`,
      );
      assertEq(
        filesOut.trim().split('\n').length,
        2,
        `两个命中文件 ⇒ 两行（实际: ${JSON.stringify(filesOut)}）`,
      );

      // ── ③ `output_mode: count`：每文件一条 `<路径>:<匹配行数>` ──
      const countOut = toolResultText(reqs[3]);
      assert(
        new RegExp(`a\\.ts:3\\b`).test(countOut),
        `count 模式 a.ts 应为 3 行（实际: ${countOut.slice(0, 300)}）`,
      );
      assert(
        new RegExp(`b\\.ts:2\\b`).test(countOut),
        `count 模式 b.ts 应为 2 行（实际: ${countOut.slice(0, 300)}）`,
      );

      // ── ④ 分页：`offset` 跳过前面的行、`head_limit` 截短输出 ──
      //
      // `content` 模式每匹配行一条、行间单换行（T-113 已修：`UTF8Sink` 的 `text`
      // 原本含行终止符 ⇒ 输出 `行\n\n行`，`apply_line_pagination` 按**输出行**切，
      // `head_limit=2` 只覆盖 1 个匹配行）。分页判据因此收紧为**匹配行**口径：
      // `offset=1, head_limit=2` ⇒ 恰好第 2、3 个匹配行（a.ts:2 / a.ts:3）。
      const pagedOut = toolResultText(reqs[4]);
      const pagedMatches = pagedOut.split('\n').filter((l) => l.includes(MARK_A));
      assertEq(
        pagedMatches.length,
        2,
        `offset=1 + head_limit=2 ⇒ 恰 2 个匹配行（实际: ${JSON.stringify(pagedOut)}）`,
      );
      assert(
        !pagedOut.includes('a.ts:1:'),
        `offset=1 应跳过第 1 个匹配行（实际: ${pagedOut.slice(0, 300)}）`,
      );
      assert(
        pagedMatches[0].includes('a.ts:2:') && pagedMatches[1].includes('a.ts:3:'),
        `应恰为第 2、3 个匹配行（实际: ${JSON.stringify(pagedMatches)}）`,
      );
      assert(
        !pagedOut.includes('b.ts'),
        `head_limit 截在 a.ts 内 ⇒ b.ts 不该出现（实际: ${pagedOut.slice(0, 300)}）`,
      );
      assert(
        pagedOut.length < contentOut.length,
        `head_limit=2 应截短输出（分页 ${pagedOut.length} 字节 < 默认 ${contentOut.length} 字节）`,
      );

      // ── ⑤ 截断：输出超 1 MiB ⇒ 截断正文 + 尾注 ──
      const truncOut = toolResultText(reqs[5]);
      assert(
        truncOut.includes('[输出已截断'),
        `超 1 MiB 的输出应带截断尾注（实际尾部: ${truncOut.slice(-200)}）`,
      );
      assert(
        truncOut.includes('1048576'),
        `尾注应写明字节上限（实际尾部: ${truncOut.slice(-200)}）`,
      );

      // ── ⑥ 转写收敛：五个 tool_call 全部终态化（经生产读出口）──
      api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'e2e-t67',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      const msgs = await readSessionMessages(api, 'e2e-t67');
      assertTranscriptInvariants(msgs, 'T67');
      const calls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
      assertEq(calls.length, 5, '应有五次工具调用节点');
      for (const c of calls) {
        assertEq(c.status, 'completed', `tool_call 终态应为 completed（${c.name}）`);
      }
    } finally {
      api?.stop();
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '读回 CLI / mock-llm 子进程退出',
      });
    }
  },
);

/**
 * 取「本轮工具结果回灌」的那条 `role=tool` 消息正文。
 *
 * ⚠️ **取最后一条，不是第一条**（与 t64 同一条纪律）：第二轮请求的历史里已带上一轮
 * 的工具结果，`find` 会拿到**上一轮**的结果。
 */
function toolResultText(req) {
  const msgs = req?.body?.messages ?? [];
  const tool = [...msgs].reverse().find((m) => msgRoleOf(m) === 'tool');
  if (tool) return typeof tool.content === 'string' ? tool.content : JSON.stringify(tool.content);
  return '';
}

import './_selfrun.mjs';
// T64 本地检索组：`content_search`（真实 ripgrep 库）与 `codebase_search`（本地嵌入语义检索）。
//
// ## 这条用例补的是什么
//
// RE-005 的「本地检索组」行三格全空（面审计发现，T-076 批）：两个工具**已实现、
// 已注册进 LLM 可见面**，却一条 e2e 都没有。此前它们只有 Rust 侧单测，而单测
// 跑在**进程内直接构造工具**的路径上——「工具经能力注册表进模型请求 → 模型发
// tool_calls → 编排层执行 → 结果回灌」这条**用户真的会走的链路**没人走过。
//
// ## 判据怎么取（为什么这样取）
//
// 标记词**每次运行随机生成**、只写在工作区文件里。这是本用例的非空转锚点：
// mock-llm 无法凭空造出它，所以「结果里出现这个标记」= 「工具真的读了磁盘」，
// 而不是「mock 背了一段台词」。
//
// `content_search` 另钉两条**只有真用 ripgrep 库才成立**的语义：被 `.gitignore`
// 命中的目录、以及隐藏文件，都不得进结果（`content_search.rs::search` 的
// `standard_filters(true)` + `hidden(true)`）。这两条把「后端是库内 ripgrep、
// 不是某个外部 `grep`」变成可判的读数——换成外部可执行文件而不带这些开关，
// 断言当场红。
//
// `codebase_search` 的判据取**契约**而非某个具体排名：`mode` 只有两个合法值
// （`semantic` / `keyword_fallback`，见 `codebase_search.rs::execute_inner`），
// 且两者各自带一组自洽的读数（semantic ⇒ 有索引统计且 score > 0；fallback ⇒
// score 恒 0 且无索引统计）。为什么不断言「必须是 semantic」：嵌入服务能否起
// 取决于 `ort` 运行时，而那不是本用例的被测对象——把判据钉在「报出来的模式与
// 结果形状自洽」上，两种环境下都**判得动**，而**报错模式**（说 semantic 却给
// fallback 形状）照样红。工作区只有两个源文件、`limit` 默认 8 ≥ 块数，所以两种
// 模式下标记所在的那一块都必然进结果集。
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

const CONTENT_MARKER = marker('MKCONTENT');
const CODEBASE_MARKER = marker('MKCODEBASE');

export default defineCase(
  'T64 本地检索组：content_search 真读磁盘并守 ignore 语义 · codebase_search 模式与结果自洽',
  async () => {
    const llm = await new MockLlm([
      // ⚠️ 启动器必须带唯一 `match`（夹具陷阱 10：无 match 的场景「先声明先被消费」，
      // 会被第一个普通轮吃掉，表现为「长轮根本没跑」而断言全绿）。
      {
        id: 'call-content',
        match: '搜一下本地标记',
        toolCalls: [
          { id: 'call_cs', name: 'content_search', arguments: { pattern: CONTENT_MARKER } },
        ],
      },
      // 工具结果轮才匹配（`afterTool`）。
      //
      // ⚠️ **必须带 `once`**（夹具陷阱 14）：`afterTool` 只是「工具结果轮的优先池」，
      // 池里**不限名额**的那一条会吃掉**每一个**工具结果轮——包括它自己那一次的结果轮
      // ⇒ 无限工具循环。症状是 CLI 撞满 120s 超时（不是断言红），日志里刷成百上千行
      // `⚙ 调用工具: codebase_search`。
      {
        id: 'call-codebase',
        afterTool: true,
        once: true,
        toolCalls: [
          { id: 'call_cb', name: 'codebase_search', arguments: { query: CODEBASE_MARKER } },
        ],
      },
      { id: 'final', afterTool: true, content: '两轮检索都跑完了。' },
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
        session: { ...DIALOG_FACE_OFF },
      },
    });

    // 工作区布局：四处都写同一个标记，但**只有一处该被搜到**。
    //   src/alpha.ts      —— 该命中
    //   ignored/secret.txt —— 被 .gitignore 命中，不该出现
    //   .hidden.md         —— 隐藏文件，不该出现
    //   docs/notes.md      —— 只含另一个标记，用来验「按字面模式检索」
    mkdirSync(join(hd.workdir, 'src'), { recursive: true });
    mkdirSync(join(hd.workdir, 'docs'), { recursive: true });
    mkdirSync(join(hd.workdir, 'ignored'), { recursive: true });
    writeFileSync(
      join(hd.workdir, 'src', 'alpha.ts'),
      `// 解析命令行参数的入口\nconst TAG = '${CONTENT_MARKER}';\nexport function parseArgs(input: string) {\n  return input.split(' ');\n}\n`,
    );
    writeFileSync(
      join(hd.workdir, 'docs', 'notes.md'),
      `# 说明\n\n这条语义检索的标记是 ${CODEBASE_MARKER}，它描述「解析命令行参数的函数」。\n`,
    );
    writeFileSync(join(hd.workdir, '.gitignore'), 'ignored/\n');
    writeFileSync(join(hd.workdir, 'ignored', 'secret.txt'), `ignored ${CONTENT_MARKER}\n`);
    writeFileSync(join(hd.workdir, '.hidden.md'), `hidden ${CONTENT_MARKER}\n`);

    let api = null;
    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我搜一下本地标记',
        provider: PROVIDER_ID,
        session: 'e2e-t64',
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
      assert(
        r.stdout.includes('两轮检索都跑完了'),
        `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`,
      );

      // ── 证据①：两次请求的工具面里有这两个工具（「已实现且经 e2e 验证」的最小前提）
      const reqs = await llm.requests();
      assertEq(reqs.length, 3, `应三次请求（用户轮 + 两次工具结果轮），实际 ${reqs.length}`);
      const wireNames = (reqs[0].body.tools ?? []).map((t) => t.function?.name);
      for (const name of ['content_search', 'codebase_search']) {
        assert(
          wireNames.includes(name),
          `${name} 应注册进模型可见工具面（实际: ${wireNames.join(',')}）`,
        );
      }

      // ── 证据②：content_search 结果回灌，内容来自磁盘且守 ignore / 隐藏语义
      //
      // 回灌口径：`tool_executor::extract_result` 先取 `content` 再取 `output`——
      // `content_search` 返回 `{output, truncated, backend}`，所以模型看到的是**命中行
      // 正文**（`<路径>:<行号>:<行内容>`），`backend` / `truncated` 不在回灌里。
      const contentResult = toolResultText(reqs[1]);
      assert(
        contentResult.includes(CONTENT_MARKER),
        `content_search 结果应含磁盘上的随机标记（实际: ${contentResult.slice(0, 600)}）`,
      );
      assert(
        contentResult.includes('alpha.ts'),
        `content_search 结果应指出命中文件（实际: ${contentResult.slice(0, 600)}）`,
      );
      assert(
        !contentResult.includes('secret.txt'),
        `被 .gitignore 命中的目录不该进结果（实际: ${contentResult.slice(0, 600)}）`,
      );
      assert(
        !contentResult.includes('.hidden.md'),
        `隐藏文件不该进结果（实际: ${contentResult.slice(0, 600)}）`,
      );
      assert(
        !contentResult.includes(CODEBASE_MARKER),
        'content_search 按字面模式检索，不该命中另一处标记（两处内容不同）',
      );

      // ── 证据③：codebase_search 结果回灌 + 模式/形状自洽
      //
      // 回灌口径：`codebase_search` 的返回体没有 `content` / `output` / `success`，
      // 落到 `extract_result` 的末档 `data.to_string()` ⇒ 模型看到的是**整段 JSON**。
      const codebaseResult = toolResultText(reqs[2]);
      assert(
        codebaseResult.includes(CODEBASE_MARKER),
        `codebase_search 结果应含磁盘上的随机标记（实际: ${codebaseResult.slice(0, 600)}）`,
      );
      const cb = parseJsonBlock(codebaseResult);
      assert(cb, `codebase_search 结果应是一段 JSON（实际: ${codebaseResult.slice(0, 600)}）`);
      assert(
        cb.mode === 'semantic' || cb.mode === 'keyword_fallback',
        `codebase_search 的 mode 只能是 semantic / keyword_fallback（实际: ${cb.mode}）`,
      );
      assertEq(
        cb.count,
        (cb.results ?? []).length,
        'count 必须等于 results 的长度（自报读数与实际结果不一致就是假的读数）',
      );
      assert(
        (cb.results ?? []).length >= 1,
        `工作区里的标记应至少命中一条（实际: ${JSON.stringify(cb).slice(0, 600)}）`,
      );
      assert(
        (cb.results ?? []).some((x) => String(x.snippet ?? '').includes(CODEBASE_MARKER)),
        `命中里应有携带该标记的那一块（实际: ${JSON.stringify(cb.results ?? []).slice(0, 400)}）`,
      );
      if (cb.mode === 'semantic') {
        assert(cb.index, 'semantic 模式必须带索引统计（index），否则「语义」二字无从核对');
        assert(
          (cb.results ?? []).every((x) => typeof x.score === 'number' && x.score > 0),
          `semantic 结果的 score 应大于 0（实际: ${JSON.stringify(cb.results ?? []).slice(0, 400)}）`,
        );
      } else {
        assertEq(cb.index, null, 'keyword_fallback 不带索引统计');
        assert(
          (cb.results ?? []).every((x) => x.score === 0),
          'keyword_fallback 的 score 恒 0（关键词命中没有相似度可言）',
        );
      }

      // ── 证据④：转写落格，两个 tool_call 都终态化（经生产读出口）
      api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: 'e2e-t64',
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      const msgs = await readSessionMessages(api, 'e2e-t64');
      assertTranscriptInvariants(msgs, 'T64');
      const calls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
      assertEq(calls.length, 2, '应有两次工具调用节点');
      assertEq(
        calls.map((c) => c.meta?.tool_name).join(','),
        'content_search,codebase_search',
        '工具名与顺序应保持',
      );
      for (const c of calls) {
        assertEq(c.status, 'completed', `tool_call(${c.meta?.tool_name}) 终态应为 completed`);
      }
    } finally {
      api?.stop();
      llm.stop();
      cleanupHomedir(hd);
      // 收尾等子进程退干净（Windows 上带活子进程退出会撞 libuv 断言）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '读回 CLI / mock-llm 子进程退出',
      });
    }
  },
);

/**
 * 取「本轮工具结果回灌」的那条 `role=tool` 消息正文。
 *
 * ⚠️ **取最后一条，不是第一条**：第二轮请求的历史里已经带着上一轮的工具结果
 * （`[…, assistant, tool, assistant, tool]`），取 `find` 会拿到**上一轮**的结果，
 * 断言于是对着错误的读数判——症状是「codebase_search 的结果里是 content_search 的
 * 正文」，而工具其实跑得好好的。
 *
 * 判据取**结果正文**这一层（与 t2 同源）：它才是「结果真的回灌给模型了」这件事
 * 本身，`role=tool` 只是承载方式。
 */
function toolResultText(req) {
  const msgs = req?.body?.messages ?? [];
  const tool = [...msgs].reverse().find((m) => msgRoleOf(m) === 'tool');
  if (tool) return typeof tool.content === 'string' ? tool.content : JSON.stringify(tool.content);
  // 拿不到就返回空串，让上面的断言以「不含标记」的形式红出来——**不得静默兜底**成
  // 「从 user 轮里捞」，那会把「结果没回灌」这个真问题掩盖掉。
  return '';
}

/** 从工具结果正文里抠出 JSON 块（结果正文是整段 JSON，或多行文本包着它）。 */
function parseJsonBlock(text) {
  const start = text.indexOf('{');
  const end = text.lastIndexOf('}');
  if (start < 0 || end <= start) return null;
  try {
    return JSON.parse(text.slice(start, end + 1));
  } catch {
    return null;
  }
}

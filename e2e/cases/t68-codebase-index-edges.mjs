import './_selfrun.mjs';
// T68 `codebase_search` 边界批（T-111 的③④⑤）：
//   - 索引的**增量时效性**：同一工作区连搜两次，第二次 `embedded_files == 0`
//     （文件集指纹未变 ⇒ 一次嵌入都不做，索引原样复用）；
//   - `rebuild: true` 强制**全量重建**（`embedded_files == files`）；
//   - `keyword_fallback` 分支的**单独驱动**。
//
// ## 为什么这一条值得单独写
//
// `codebase_search.rs` 的模块头把「索引的时效性」当**首要设计约束**，并明写
// 「**静默的过期索引**比慢更糟——agent 会拿一份不包含自己刚写的代码的索引去搜，
// 且完全无从察觉」。这句话此前只有 Rust 侧单测，端到端没有读数。`index` 统计
// （`files` / `chunks` / `reused_files` / `embedded_files`）就是为把这件事变可见而
// 放进返回体的 ⇒ 本用例直接判它。
//
// ## 回灌口径
//
// `codebase_search` 的返回体没有 `content` / `output` / `success`，落到
// `tool_executor::extract_result` 的末档 `data.to_string()` ⇒ 模型看到的是**整段 JSON**。
// 与 T64 同一口径。
//
// ## 关于 `keyword_fallback`：为什么只驱动「无可索引文件」这条路径
//
// `codebase_search` 里嵌入服务是 `creator_create_object(EMBEDDING_LOCAL, ctx)`
// **硬编码**的，模型 `include_bytes!` 编进二进制（`providers/embedding/local.rs`）——
// 全仓**没有配置位**能把它切成 `EMBEDDING_NOOP`。因此「嵌入服务不可用」这条降级
// 路径在 e2e 里驱动不了（要驱动得先加配置面，属能力新增，不在本批范围）。
//
// 但 `keyword_fallback` 还有一条**可驱动**路径：工作区没有可索引的源码文件时
// `get_index` 返回 `None`，直接落到 `keyword_search` 并置 `mode = "keyword_fallback"`、
// `index = null`。本用例用空工作区驱动它——那条 `mode = "keyword_fallback"` 赋值与
// 「fallback ⇒ index 为 null」的形状因此**真的被执行过**（T64 的同类断言在本机
// semantic 环境下从未走到 fallback 分支）。
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
} from '../helpers.mjs';

/** 随机标记词：只存在于工作区文件里，模型侧无从得知 ⇒ 「出现」即「真读了盘」。 */
function marker(prefix) {
  return `${prefix}_${Math.random().toString(36).slice(2, 10).toUpperCase()}`;
}

const MARK = marker('MKIDX');

/** 标准 gateway 入站配置（转写走生产读出口要它开着）。 */
function gatewayConfig(port) {
  return {
    inbound_enabled: true,
    inbound_protocol: 'http',
    inbound_bind: '127.0.0.1',
    inbound_port: port,
    inbound_token: '',
    inbound_readonly: false,
  };
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

/** 取「本轮工具结果回灌」的那条 `role=tool` 消息正文（取**最后一条**，见 t64）。 */
function toolResultText(req) {
  const msgs = req?.body?.messages ?? [];
  const tool = [...msgs].reverse().find((m) => m.role === 'tool');
  if (tool) return typeof tool.content === 'string' ? tool.content : JSON.stringify(tool.content);
  return '';
}

export default defineCase(
  'T68 codebase_search 边界：索引增量（embedded_files==0）· rebuild 全量 · fallback 单独驱动',
  async () => {
    // ══ 阶段一：索引时效性（同一进程内连搜三次）══
    {
      const llm = await new MockLlm([
        {
          id: 'first',
          match: '语义搜一下',
          toolCalls: [{ id: 'cb_first', name: 'codebase_search', arguments: { query: MARK } }],
        },
        {
          id: 'second',
          afterTool: true,
          once: true,
          toolCalls: [{ id: 'cb_second', name: 'codebase_search', arguments: { query: MARK } }],
        },
        {
          id: 'rebuild',
          afterTool: true,
          once: true,
          toolCalls: [
            { id: 'cb_rebuild', name: 'codebase_search', arguments: { query: MARK, rebuild: true } },
          ],
        },
        { id: 'final', afterTool: true, content: '索引边界验完了。' },
      ]).start();

      const GATEWAY_PORT = nextPort();
      const hd = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT), session: { ...DIALOG_FACE_OFF } },
      });
      // 两个源码文件（各 1 块）⇒ 块数 ≤ 默认 limit(8) ⇒ 两模式下标记所在块都必然进结果集。
      mkdirSync(join(hd.workdir, 'src'), { recursive: true });
      writeFileSync(
        join(hd.workdir, 'src', 'a.ts'),
        `// 解析命令行参数的入口\nconst TAG = '${MARK}';\nexport function parseArgs(input: string) {\n  return input.split(' ');\n}\n`,
      );
      writeFileSync(
        join(hd.workdir, 'src', 'b.ts'),
        `// 另一个文件里的标记 ${MARK}\nexport const VERSION = '1.0.0';\n`,
      );

      let api = null;
      try {
        const r = runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message: '帮我语义搜一下标记',
          provider: PROVIDER_ID,
          session: 'e2e-t68',
        });
        assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
        assert(r.stdout.includes('索引边界验完了'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

        const reqs = await llm.requests();
        assertEq(reqs.length, 4, `应四次请求（用户轮 + 三次工具结果轮），实际 ${reqs.length}`);

        const first = parseJsonBlock(toolResultText(reqs[1]));
        const second = parseJsonBlock(toolResultText(reqs[2]));
        const rebuild = parseJsonBlock(toolResultText(reqs[3]));
        for (const [label, cb] of [['第一次', first], ['第二次', second], ['rebuild', rebuild]]) {
          assert(cb, `${label} 的结果应是一段 JSON`);
          assert(
            cb.mode === 'semantic' || cb.mode === 'keyword_fallback',
            `${label} mode 只能是 semantic / keyword_fallback（实际: ${cb.mode}）`,
          );
          assertEq(cb.count, (cb.results ?? []).length, `${label} count 必须等于 results 长度`);
          assertEq(cb.mode, first.mode, `${label} 与第一次的 mode 应一致（同一工作区、同一嵌入服务）`);
        }
        assert(
          (first.results ?? []).some((x) => String(x.snippet ?? '').includes(MARK)),
          `首次结果里应有携带标记的那一块（实际: ${JSON.stringify(first.results ?? []).slice(0, 400)}）`,
        );

        if (first.mode === 'semantic') {
          // ③ 首次：全量嵌入（`embedded_files == files`，一个文件都没复用）。
          assert(first.index, 'semantic 必须带索引统计');
          assertEq(
            first.index.embedded_files,
            first.index.files,
            '首次检索应全量嵌入（embedded_files == files）',
          );
          assertEq(first.index.reused_files, 0, '首次没有可复用的旧块');

          // ③ 第二次：文件集指纹未变 ⇒ **一次嵌入都不做**（这正是「时效性」机制的核心读数）。
          assert(second.index, 'semantic 必须带索引统计');
          assertEq(
            second.index.embedded_files,
            0,
            '第二次检索应零嵌入（索引已是最新，embedded_files 必须为 0）',
          );
          assertEq(
            second.index.reused_files,
            second.index.files,
            '第二次应复用全部文件（reused_files == files）',
          );

          // ④ rebuild=true：忽略现有索引、全量重嵌。
          assert(rebuild.index, 'semantic 必须带索引统计');
          assertEq(
            rebuild.index.embedded_files,
            rebuild.index.files,
            'rebuild=true 应全量重建（embedded_files == files）',
          );
        } else {
          // 嵌入不可用（本机不该走到这里）：索引里应 0 块，且 fallback 仍给出形状自洽的结果。
          assert(second.index === null || second.index.chunks === 0, 'fallback 下索引不该有块');
        }

        api = await startReadbackCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          session: 'e2e-t68',
          provider: PROVIDER_ID,
          gatewayPort: GATEWAY_PORT,
        });
        const msgs = await readSessionMessages(api, 'e2e-t68');
        assertTranscriptInvariants(msgs, 'T68a');
        const calls = msgs.filter((m) => m.type === 'tool_call');
        assertEq(calls.length, 3, '应有三次工具调用节点');
        for (const c of calls) assertEq(c.status, 'completed', `tool_call 终态应为 completed（${c.name}）`);
      } finally {
        api?.stop();
        llm.stop();
        cleanupHomedir(hd);
        await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
          what: 'T68 阶段一 读回 CLI / mock-llm 子进程退出',
        });
      }
    }

    // ══ 阶段二：`keyword_fallback` 分支的单独驱动（空工作区 ⇒ 无可索引文件）══
    {
      const llm = await new MockLlm([
        {
          id: 'empty',
          match: '空工作区',
          toolCalls: [{ id: 'cb_empty', name: 'codebase_search', arguments: { query: MARK } }],
        },
        { id: 'final', afterTool: true, content: '空工作区也跑通了。' },
      ]).start();

      const GATEWAY_PORT = nextPort();
      const hd = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { gateway: gatewayConfig(GATEWAY_PORT), session: { ...DIALOG_FACE_OFF } },
      });
      // **刻意什么都不写**：工作区无可索引源码文件 ⇒ `scan_files` 空 ⇒ `get_index` 返回 None
      // ⇒ 落到 `keyword_search`，`mode = "keyword_fallback"`、`index = null`。

      let api = null;
      try {
        const r = runCli({
          homedir: hd.homedir,
          workdir: hd.workdir,
          message: '在空工作区里搜一下',
          provider: PROVIDER_ID,
          session: 'e2e-t68-empty',
        });
        assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 400)}）`);
        assert(r.stdout.includes('空工作区也跑通了'), `stdout 应为收尾正文（实际: ${JSON.stringify(r.stdout)}）`);

        const reqs = await llm.requests();
        assertEq(reqs.length, 2, `应两次请求（用户轮 + 一次工具结果轮），实际 ${reqs.length}`);
        const cb = parseJsonBlock(toolResultText(reqs[1]));
        assert(cb, `空工作区结果应是一段 JSON（实际: ${toolResultText(reqs[1]).slice(0, 400)}）`);
        assertEq(cb.mode, 'keyword_fallback', '无可索引文件 ⇒ 必须落到 keyword_fallback');
        assertEq(cb.index, null, 'keyword_fallback（无可索引文件）不带索引统计');
        assertEq(cb.count, 0, '空工作区里搜不到任何东西');
      } finally {
        api?.stop();
        llm.stop();
        cleanupHomedir(hd);
        await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
          what: 'T68 阶段二 读回 CLI / mock-llm 子进程退出',
        });
      }
    }
  },
);

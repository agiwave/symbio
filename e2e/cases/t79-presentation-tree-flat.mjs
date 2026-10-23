import './_selfrun.mjs';
// T79 建树策略 flat：本轮响应骨架的直接子节点升到会话根，工具结果仍挂 tool_call。
//
// ## 本用例钉的是什么
//
// 相应批次 的**判据核心**：`presentation_tree` 是后端的建树档位，前端**不知道它存在**
// ——它只按 `parent_id` 拼树。所以「切档前端零代码改动」只能这样证：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 脱父发生在**生产读出口**（不是只在内存里） | `readSessionMessages`（gateway `vdfs/read` 会话叶子 = 前端 `loadMessages` 同一出口） |
// | `flat` 与 `grouped` 是**同一个会话数据**的两种呈现 | 本用例只翻 `PLUGIN.yml` 的一个键，其它逐字同 T26 |
// | 工具结果**不受建树策略影响**（那是工具调用内部的骨架） | tool_call 之下仍挂 `role=tool` 结果 |
//
// ## 形状契约（flat）
//
// - `Skeleton{run}`（一轮响应骨架）留在会话根，是本轮的**分组锚**。
// - 它的**直接子节点**（思考 / 正文 / ToolCall 骨架）`parent_id = null`、
//   `meta.turn_root = <骨架 id>`——升到根、靠记号认回本轮。
// - Tool 结果节点 `parent_id = <tool_call id>`——**不升根**，与建树策略正交。
//
// ## 失败→重试的并集在哪里钉
//
// 脱父子节点靠 `meta.turn_root` 被后端的子树遍历器（`failure::subtree_of`、
// `resume::process_retry_subtree`）认回——若那并集没接上，重试删不干净、残留下一次
// 重建。这是 flat 相对 grouped 唯一新增的**后端行为**，由 Rust 单测
// `failure.test.rs::subtree_of_flats_reaches_detached_children_via_turn_root` 直接锁
// （本 e2e 用例专注呈现出口的形状，不重跑一次失败/恢复）。
//
// 对话面按 `DIALOG_FACE_OFF` 钉死（与建树正交，见该常量）。
import { join } from 'node:path';
import {
  E2E_ROOT,
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  addMcpServer,
  cleanupHomedir,
  runCli,
  startReadbackCli,
  readSessionMessages,
  nextPort,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  waitFor,
  msgTypeOf,
  msgRoleOf,
} from '../helpers.mjs';

const SID = 'e2e-t79';

export default defineCase(
  'T79 建树策略 flat：本轮子节点升会话根（带 turn_root 记号）、工具结果仍挂 tool_call、无孤儿',
  async () => {
    const llm = await new MockLlm([
      {
        id: 'call-echo',
        match: '帮我回显',
        once: true,
        content: '我先回显一下。',
        toolCalls: [
          { id: 'call_1', name: 'mcp__mockserv__echo', arguments: { text: 'flat 建树' } },
        ],
      },
      { id: 'after-echo', content: '工具调用完成，回显成功。' },
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
        // 本用例的全部差异就在这一行：建树策略翻到 flat，其它逐字同 T26。
        session: { ...DIALOG_FACE_OFF, presentation_tree: 'flat' },
      },
    });
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs')],
      enabled: true,
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我回显一下',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      const api = await startReadbackCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        const msgs = await readSessionMessages(api, SID);
        assertTranscriptInvariants(msgs, 'T79');

        // 骨架在会话根：本轮响应骨架是 grouped 里所有子节点的那个父。
        const run = msgs.find((m) => msgTypeOf(m) === 'turn');
        assert(run, `应有本轮响应骨架（实得类型: ${msgs.map((m) => msgTypeOf(m)).join(',')}）`);
        assertEq(run.parent_id ?? null, null, '响应骨架自身在会话根');

        // flat 判据①：骨架的直接子节点（思考/正文/ToolCall）升根 + 带 turn_root 记号。
        const kids = msgs.filter(
          (m) => msgTypeOf(m) !== 'user_prompt' && m.meta?.turn_root === run.id
        );
        assert(kids.length > 0, 'flat：本轮至少有一个升根的子节点带 turn_root 记号');
        for (const k of kids) {
          assertEq(k.parent_id ?? null, null, `flat：${msgTypeOf(k)} 应升会话根`);
        }
        // ToolCall 骨架既升根、又认得本轮。
        const tc = msgs.find((m) => msgTypeOf(m) === 'tool_call');
        assert(tc, 'flat：应有 tool_call 骨架');
        assertEq(tc.parent_id ?? null, null, 'flat：tool_call 升会话根');
        assertEq(tc.meta?.turn_root, run.id, 'flat：tool_call 靠 turn_root 认回本轮骨架');

        // flat 判据②：工具结果**不升根**，仍挂自己的 tool_call 下（建树策略正交）。
        const result = msgs.find((m) => m.parent_id === tc.id && msgRoleOf(m) === 'tool');
        assert(result, '工具结果必须仍挂在 tool_call 之下（不受 flat 影响）');

        // 无游离节点：每个非根节点的 parent_id 必须指向真实存在的节点。
        const ids = new Set(msgs.map((m) => m.id));
        for (const m of msgs) {
          if (m.parent_id != null) {
            assert(ids.has(m.parent_id), `无孤儿：${msgTypeOf(m)} 的父 ${m.parent_id} 应存在`);
          }
        }
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

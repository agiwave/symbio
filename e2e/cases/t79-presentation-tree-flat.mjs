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
// ## 形状契约（flat，D-335）
//
// - 一轮的可见锚是**用户消息节点**（`parent_id = null`，其 id 就是本轮分组 id）；
//   没有独立的轮骨架节点——轮信息并入用户消息节点。
// - 本轮其余节点（思考 / 正文 / ToolCall 骨架）`parent_id = null`、
//   `meta.turn_root = <用户消息节点 id>`——升到根、靠记号认回本轮。
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
  dialogFaceOff,
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
        ...dialogFaceOff({ session: { presentation_tree: 'flat' } }),
      },
    });
    addMcpServer(hd, 'mockserv', {
      type: 'stdio',
      command: process.execPath,
      args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs')],
      enabled: true,
    });

    try {
      const r = await runCli({
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

        // 轮锚 = 用户消息节点（没有独立的轮骨架可见节点）
        const anchor = msgs.find(
          (m) => msgRoleOf(m) === 'user' && msgTypeOf(m) === 'text',
        );
        assert(
          anchor,
          `应有用户消息节点作为轮锚（实得类型: ${msgs.map((m) => msgTypeOf(m)).join(',')}）`,
        );
        assertEq(anchor.parent_id ?? null, null, '用户消息节点（轮锚）应在会话根');

        // 主判据①：树是平的——除工具结果挂在自己的 tool_call 下，其余节点全在会话根
        const toolCalls = msgs.filter((m) => msgTypeOf(m) === 'tool_call');
        assert(toolCalls.length > 0, 'flat：应有 tool_call 骨架');
        for (const tc of toolCalls) {
          assertEq(
            tc.parent_id ?? null,
            null,
            `flat：tool_call 升会话根（实际父 ${tc.parent_id ?? 'null'}，轮锚 ${anchor.id}）`,
          );
        }
        for (const m of msgs) {
          if ((m.parent_id ?? null) === null) continue;
          assert(
            toolCalls.some((tc) => tc.id === m.parent_id),
            `树应是平的：唯一的父子关系是工具结果 → tool_call（${msgTypeOf(m)} 的父 ${m.parent_id}）`,
          );
        }

        // 主判据②：每个子节点（工具结果除外）的 turn_root 等于用户消息节点 id
        const kids = msgs.filter(
          (m) => m.id !== anchor.id && msgRoleOf(m) !== 'tool' && msgTypeOf(m) !== 'user_prompt',
        );
        assert(kids.length > 0, 'flat：本轮至少有一个升根的子节点');
        for (const k of kids) {
          assertEq(k.meta?.turn_root, anchor.id, `flat：${msgTypeOf(k)} 的 turn_root 应指向用户消息节点`);
        }

        // ToolCall 骨架既升根、又认得本轮。
        const tc = toolCalls[0];
        assertEq(tc.meta?.turn_root, anchor.id, 'flat：tool_call 靠 turn_root 认回本轮');

        // flat 判据③：工具结果**不升根**，仍挂自己的 tool_call 下（建树策略正交）。
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

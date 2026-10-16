import './_selfrun.mjs';
// T44 消息变更**入格且持久**（[ADR-053] / [ADR-054] 的 e2e 判据）：
// 跑两轮 ⇒ **编辑**第一问 ⇒ **截断**第二问 ⇒ **重启进程**（冷开新进程）⇒
// 从生产读出口读回的历史仍是**变更之后**的那份（编辑生效、被截的那轮没复活）。
//
// ## 本用例钉的是什么
//
// 「变更即事实」只能这样证：把变更做完，**换一个进程重开**——新进程的读不依赖
// 上一个进程的任何内存态，历史若还能完整地以「变更之后」的形态回来，变更就真的
// 落在了会话的真源里（而不是只活在上一个进程的缓存里）。
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 编辑进了真源 | 重启后读回的正文是**改过**的那句 |
// | 截断进了真源 | 重启后读回的历史里**没有**被截掉的那一轮 |
// | 变更即时可见 | 截断回执带权威被删 id 列表，且含目标节点 |
//
// 判读全程只走用户可见的边界：`vdfs/write` / `vdfs/action`（变更入口，与前端同一
// 路由）与 `vdfs/read`（读出口，与前端 `loadMessages` 同一出口）——不碰任何内部
// 存储文件。
//
// ## 为什么不比 `seq` / `timestamp`
//
// ADR-049 已判定切源后 `seq` 分配属呈现细节。而且本用例里被编辑的那条**刻意不重发号**
// ——位置锚在编辑前后逐字相同，这正是「编辑不动位置」的判据。
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
  msgRoleOf,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t44';

/** 两轮的触发串（mock 场景 `match` 与用户正文共用同一个词）。 */
const Q1 = '第一问';
const Q2 = '第二问';

/** 编辑后的第一问（与 `Q1` 不同，否则「编辑生效」与「没编辑」分不开）。 */
const Q1_EDITED = '改过的第一问';

/** 取消息的正文文本（与 helpers 的 `textOf` 同口径，就地复算以保持用例自洽）。 */
function contentText(m) {
  const c = m.content;
  if (c == null) return '';
  if (typeof c === 'string') return c;
  if (Array.isArray(c)) return c.map((p) => p.text ?? '').join('');
  return JSON.stringify(c);
}

/** 消息的**可比形状**（剔除呈现细节 `seq` / `timestamp`——ADR-049 判定切源后 seq 分配属呈现细节）。 */
function shapeOf(msgs) {
  return msgs.map((m) =>
    [
      m.id ?? '',
      m.parent_id ?? '',
      msgRoleOf(m) ?? '',
      m.type ?? m.msg_type ?? '',
      m.status ?? '',
      // [T-147d-ii] 顶层 name 已退役——工具名住 meta.tool_name
      m.meta?.tool_name ?? '',
      m.tool_call_id ?? '',
      contentText(m),
    ].join('\u0001'),
  );
}

export default defineCase(
  'T44 变更入格且持久：编辑 / 截断后重启进程，读回的历史是**变更之后**的那份（ADR-053/054 判据）',
  async () => {
    const llm = await new MockLlm([
      { id: 'q1', match: Q1, content: '第一答。' },
      { id: 'q2', match: Q2, content: '第二答。' },
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

    const base = { homedir: hd.homedir, workdir: hd.workdir, provider: PROVIDER_ID };

    try {
      // ── 跑两轮成功对话（同一会话 ⇒ 同一份真源）──────────────────────────
      const r1 = runCli({ ...base, session: SID, message: Q1 });
      assertEq(r1.code, 0, `第 1 轮退出码（stderr: ${r1.stderr.slice(0, 400)}）`);
      const r2 = runCli({ ...base, session: SID, message: Q2 });
      assertEq(r2.code, 0, `第 2 轮退出码（stderr: ${r2.stderr.slice(0, 400)}）`);

      const cli = startLongLivedCli({ ...base, session: SID, gatewayPort: GATEWAY_PORT });
      try {
        await cli.waitGatewayReady();

        const rootPath = (await cli.invoke('vdfs/root', {})).body?.data?.path;
        assert(typeof rootPath === 'string' && rootPath.length > 0, 'vdfs/root 应返回根地址');
        const sessionAddr = `${rootPath.replace(/\/+$/, '')}/session/${SID}`;

        /** 读会话叶子，解出 `{ id, messages, … }`（与前端 `loadMessages` 同一出口）。
         *  参数带进程与地址：重启判据要对**第二个进程**读同一个会话。 */
        const readLeaf = async (c, addr, what) => {
          const resp = await c.invoke('vdfs/read', { path: addr });
          assertEq(resp.status, 200, `${what} 应成功（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
          const text = resp.body?.data?.text;
          assert(
            typeof text === 'string' && text.length > 0,
            `${what} 应带回文本正文（${JSON.stringify(resp.body)?.slice(0, 300)}）`,
          );
          return JSON.parse(text);
        };

        const beforeCrud = await readLeaf(cli, sessionAddr, '变更前读会话叶子');
        const q1Node = beforeCrud.messages.find(
          (m) => msgRoleOf(m) === 'user' && contentText(m).includes(Q1),
        );
        const q2Node = beforeCrud.messages.find(
          (m) => msgRoleOf(m) === 'user' && contentText(m).includes(Q2),
        );
        assert(q1Node?.id, `应能找到第一问的节点（实际: ${JSON.stringify(shapeOf(beforeCrud.messages))}）`);
        assert(q2Node?.id, '应能找到第二问的节点');
        assert(q1Node.id !== q2Node.id, '两条用户消息必须是两个节点');

        // ── ① 编辑：`vdfs/write(<会话>/message/<mid>, { content })` ──────────
        const editResp = await cli.invoke('vdfs/write', {
          path: `${sessionAddr}/message/${q1Node.id}`,
          text: JSON.stringify({ content: Q1_EDITED }),
        });
        assertEq(
          editResp.status,
          200,
          `编辑应成功（${JSON.stringify(editResp.body)?.slice(0, 300)}）`,
        );

        // ── ② 截断：`vdfs/action(<会话>/message/<mid>, truncate)` ────────────
        const truncResp = await cli.invoke('vdfs/action', {
          path: `${sessionAddr}/message/${q2Node.id}`,
          action: 'truncate',
        });
        assertEq(
          truncResp.status,
          200,
          `截断应成功（${JSON.stringify(truncResp.body)?.slice(0, 300)}）`,
        );
        const deletedIds = truncResp.body?.data?.data;
        assert(
          Array.isArray(deletedIds) && deletedIds.length > 0,
          `截断回执应带被删 id 列表（实际: ${JSON.stringify(truncResp.body?.data)}）`,
        );
        assert(
          deletedIds.includes(q2Node.id),
          `第二问必须在被删列表里（实际: ${JSON.stringify(deletedIds)}）`,
        );

        // ── 证据 ①：变更**即时可见**（同一进程内读回 = 变更之后的那份）───────
        const afterCrud = await readLeaf(cli, sessionAddr, '变更后读会话叶子');
        const afterShape = shapeOf(afterCrud.messages);
        assert(
          afterShape.some((r) => r.includes(Q1_EDITED)),
          `变更后历史里应是改过的正文（实际: ${JSON.stringify(afterShape)}）`,
        );
        assert(
          !afterShape.some((r) => r.includes(Q2)),
          `截断后历史里不该还有第二问（实际: ${JSON.stringify(afterShape)}）`,
        );
        assert(
          !afterShape.some((r) => r.includes('第二答。')),
          `截断后历史里不该还有第二答（实际: ${JSON.stringify(afterShape)}）`,
        );

        // ── 证据 ②：**重启进程**（冷开新进程）⇒ 读回的历史仍是变更后的那份 ─────
        // 新进程与上一个不共享任何内存态：历史能原样回来，变更就真的落在了会话的
        // 真源里，而不是只活在上一个进程的会话态里。
        cli.stop();
        await waitFor(() => cli.out.code !== '', { what: 'CLI #1 退出' });

        const cli2 = startLongLivedCli({
          ...base, session: SID, gatewayPort: GATEWAY_PORT,
        });
        try {
          await cli2.waitGatewayReady(20_000, 'CLI #2');
          const rootPath2 = (await cli2.invoke('vdfs/root', {})).body?.data?.path;
          assert(typeof rootPath2 === 'string' && rootPath2.length > 0, '重启后 vdfs/root 应仍返回根地址');
          const addr2 = `${rootPath2.replace(/\/+$/, '')}/session/${SID}`;

          const reopened = await readLeaf(cli2, addr2, '重启后读会话叶子');
          const reopenedShape = shapeOf(reopened.messages);
          assertEq(
            reopenedShape.join('\n'),
            afterShape.join('\n'),
            `重启后读回的历史应与变更后的那份逐字一致：\n  变更后 ${JSON.stringify(afterShape)}\n  重启后 ${JSON.stringify(reopenedShape)}`,
          );
          // 回退与复活是两种不同的病，各自成条报（逐字一致已覆盖，但分开写让失败
          // 信息直接指出**哪种**变更没入格）。
          assert(
            reopenedShape.some((r) => r.includes(Q1_EDITED)),
            `重启后第一问应是**改过**的那句（否则编辑回退了）：${JSON.stringify(reopenedShape)}`,
          );
          assert(
            !reopenedShape.some((r) => r.includes(Q1) && !r.includes(Q1_EDITED)),
            `重启后不该还有编辑前的原文（实际: ${JSON.stringify(reopenedShape)}）`,
          );
          assert(
            !reopenedShape.some((r) => r.includes(Q2)) &&
              !reopenedShape.some((r) => r.includes('第二答。')),
            `重启后被截断的那一轮不得复活（实际: ${JSON.stringify(reopenedShape)}）`,
          );

          assertTranscriptInvariants(reopened.messages, 'T44 重启');
        } finally {
          cli2.stop();
        }
      } finally {
        cli.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // 收尾必须等子进程真的退干净（与 t37 同一约定：Windows 上带活子进程退出
      // 会撞 libuv 断言，表现为「断言全过、退出码 0xC0000409」）。
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

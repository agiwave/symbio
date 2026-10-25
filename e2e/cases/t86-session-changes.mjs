import './_selfrun.mjs';
// T86 变更账出口 `session/changes`：一轮真实写盘 → 从生产出口读清单。
//
// ## 本用例钉的是什么
//
// [D-169](../../docs/003-design/001-核心架构.md "DE-001-核心架构") 的出口判据：
// 变更账是 `artifact.added` 的**纯投影**，出口只做「取数与排版」。端到端要证的是
// 只有这条链能看见的三件事：
//
// | 要证的事 | 唯一能看见它的地方 |
// |---|---|
// | 出口真的挂上了生产路由 | gateway `POST /api/v1/invoke` → 容器 → `session/changes` |
// | 行来自**真实写盘**留下的产物事实 | 真实 CLI 轮次 + 真实 `vdfs_*` 工具执行 |
// | 读侧闸真的按读方身份收放 | `payload.principal` → `ctx.payload` → `content_visible` |
//
// 口径（哪些工具算写、no-op 编辑不计、虚拟根/界外路径排除）已在 core 的
// `projection/changes.test.rs` 钉死。本用例**不复刻口径**，只对「已知动作」取值：
// 写两个文件 + 对其中一个做一次 no-op 编辑 ⇒ 两行、各 1 次、后写的在前。
//
// ## 为什么 no-op 编辑并进同一条消息
//
// `vdfs_edit` 的 `old_string == new_string` 会成功返回 `replaced: 0`。若投影把它当
// 一次变更，`src/lib.rs` 的 `changes` 会变成 2、动作词变成「编辑」——两个数都能在
// 出口看见，且都不报错。把它与两次写入放在**同一轮**，还顺带钉住「同一轮里多次改
// 同一路径按次数计、按最后一次有效动作呈现」。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  readFileSyncSafe,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';

/** 本用例的会话 id */
const SID = 'e2e-t86';

/** 从未跑过的会话 id：用来分辨「有源但不给看」与「没有源」 */
const NEVER_SID = 'e2e-t86-never';

export default defineCase(
  'T86 变更账 session/changes：真实写盘折成呈现就绪的清单，读侧闸随读方收放',
  async () => {
    const llm = await new MockLlm([
      // 一次请求发起三个工具调用：两次写、一次 no-op 编辑（同一轮）。
      // `once: true`：工具结果轮若再命中带 `match` 的场景会无限循环（见 t26 文件头）。
      {
        id: 'write-files',
        match: '帮我写两个文件',
        once: true,
        content: '我来写。',
        toolCalls: [
          {
            id: 'call_w1',
            name: 'vdfs_write',
            arguments: { path: 'docs/a.md', text: '# A\n' },
          },
          {
            id: 'call_w2',
            name: 'vdfs_write',
            arguments: { path: 'src/lib.rs', text: 'pub fn a() {}\n' },
          },
          {
            // old == new ⇒ provider 回 `replaced: 0`，读到结果的那一侧必须跳过它
            id: 'call_e1',
            name: 'vdfs_edit',
            arguments: {
              path: 'src/lib.rs',
              old_string: 'pub fn a() {}',
              new_string: 'pub fn a() {}',
            },
          },
        ],
      },
      { id: 'after-write', content: '写完了。' },
    ]).start();

    const GATEWAY_PORT = nextPort();
    const hd = makeHomedir({
      providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
      pluginConfigs: {
        // 出口经 gateway `invoke` 读（与前端同一条线），入站必须开着。
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: GATEWAY_PORT,
          inbound_token: '',
          inbound_readonly: false,
        },
        // 本用例与对话面正交 ⇒ 钉死三个开关，免得每轮多一次静默请求。
        session: { ...DIALOG_FACE_OFF },
      },
    });

    try {
      const r = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我写两个文件',
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(r.code, 0, `CLI 退出码（stderr: ${r.stderr.slice(0, 600)}）`);

      // ── 证据 ①：写盘真的发生了（进程外，不看后端自述）────────────────────
      assert(
        readFileSyncSafe(join(hd.workdir, 'src', 'lib.rs')).includes('pub fn a'),
        'src/lib.rs 应被真实写入（这是清单里那一行的本体）',
      );

      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      try {
        await api.waitGatewayReady();

        // ── 证据 ②：从生产路由读出口 ──────────────────────────────────────
        const inv = await api.invoke('session/changes', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/changes 应成功（实际: ${JSON.stringify(inv.body)}）`);
        const got = inv.body?.data;
        assert(got, `响应应带 data 载荷（实际: ${JSON.stringify(inv.body)}）`);
        assertEq(got.session_id, SID, '出口应回带会话 id');
        assertEq(got.has_wal, true, '跑过一轮 ⇒ 事实源在');
        assertEq(got.readable, true, '未声明读方 = 本机默认 ⇒ 可读');

        // 两次成功写、一次 no-op 编辑 ⇒ 恰好两行；最近改过的在前
        assertEq(got.rows.length, 2, `两行（实际: ${JSON.stringify(got.rows)}）`);
        const [first, second] = got.rows;
        assertEq(first.path, 'src/lib.rs', '后写的在前');
        assertEq(first.address, 'workdir/src/lib.rs', '地址相对会话容器，可直接与地址拼合');
        assertEq(first.op, 'write', 'no-op 编辑不计 ⇒ 最后一次有效动作仍是写入');
        assertEq(first.label, '写入', '展示文案由出口给，前端不认 op');
        assertEq(first.changes, 1, 'no-op 不算一次变更');
        assertEq(second.path, 'docs/a.md', '两次写都入账');
        assertEq(second.address, 'workdir/docs/a.md');
        assertEq(second.op, 'write');
        assertEq(second.changes, 1);
        // 轮号沿用事实源自己的编号：首轮从 0 起（`u-0` 开轮），本用例只跑一轮
        assertEq(first.turn, 0, '首轮 = 0（与开轮事实的既有口径一致）');
        assertEq(first.turn, second.turn, '同一轮里改的两处 ⇒ 同一个轮号');

        // ── 证据 ③：读侧闸（同一条源、三种读法，差异只来自闸）───────────────
        const asOwner = (
          await api.invoke('session/changes', { principal: 'person/local' }, { session_id: SID })
        ).body.data;
        assertEq(asOwner.readable, true, '属主恒放行');
        assertEq(asOwner.rows, got.rows, '属主读到与默认读法逐字相同');

        const outsider = (
          await api.invoke('session/changes', { principal: 'agent:ghost' }, { session_id: SID })
        ).body.data;
        assertEq(outsider.has_wal, true, 'ghost: 事实源存在，只是不给你看');
        assertEq(outsider.readable, false, '矩阵外主体 ⇒ 不给看要自己说出来');
        assertEq(outsider.rows.length, 0, '越界读取 0 行');

        // ── 证据 ④：没有事实源是「有据的空」，不是「没改过」────────────────
        const missing = (
          await api.invoke('session/changes', {}, { session_id: NEVER_SID })
        ).body.data;
        assertEq(missing.has_wal, false, '从未跑过的会话没有事实源');
        assertEq(missing.readable, true, '没有源 ⇒ 谈不上「不给看」');
        assertEq(missing.rows.length, 0);
      } finally {
        api.stop();
      }
    } finally {
      llm.stop();
      cleanupHomedir(hd);
      // Windows 上带活子进程退出会撞 libuv 断言（见 t28 收尾）：等进程真的退干净
      await waitFor(() => !process.getActiveResourcesInfo().includes('ProcessWrap'), {
        what: '长驻 CLI / mock-llm 子进程退出',
      });
    }
  },
);

import './_selfrun.mjs';
// T103 抽象协议设计（L4）是一条**插件能力**：两个工具（`protocol_propose` /
// `protocol_join`）写协议提案与加入，一条只读路由（`protocol/catalog`）把折叠交给
// 第二方；摘掉插件后工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉事实落格；
// 折叠由 owner 的 `decl.test.rs` 覆盖。
//
// ## 本用例钉的四件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `protocol_propose` / `protocol_join` |
// | 事实落格 | 直读会话 WAL 的 `protocol.proposed` / `protocol.joined` 事件 |
// | 第二方判据 | 网关 `/invoke` 调 `protocol/catalog`：`handoff` 仍 `draft`，`review` 为 `established` |
// | 第二方读入口 | `protocol/catalog` 路由是外部第二方的真实边界名 |
//
// 第三段是「协议需要第二方」的可执行形式：同一个行为既是提案也是加入，是否成立不由
// 谁声明，而由**不同参与者**数决定。`protocol/catalog` 路由没有随视图收回私有（它是
// 外部第二方的边界名），折叠读回因此只走这条路由。
//
// ## 停用侧断言什么、不断言什么
//
// 运行期停用拿掉的是**插件实例与它注册的工具**；事实声明、视图与行为是**链接期**
// 贡献（D-217 / D-218），它们不会因为翻一个装配位就从二进制里消失。因此这里断言
// 「工具面消失 + 会话照常跑完」，不把链接期贡献的存在与否伪装成运行期读数——
// 与 T88 / T89 / T93 / T100 / T101 / T102 同一口径。
import { join } from 'node:path';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  runCli,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';
import { toolNamesOf, envelopeOf, isSilentAuxRequest } from '../llm-contract.mjs';

const SID = 'e2e-t103';
const SID_OFF = 'e2e-t103-off';
const GW = nextPort();

/**
 * 五个调用：`handoff` 只有一个参与者（draft），`review` 有两个（established）。
 * 同一个参与者重复加入在视图里按不同值去重，因此 `handoff` 停在 1。
 */
const CALLS = [
  {
    id: 'call_t1',
    name: 'protocol_propose',
    arguments: {
      name: 'handoff',
      title: '任务交接',
      purpose: '把一个任务交给另一个代理',
      messages: ['request', 'accept', 'done'],
    },
  },
  {
    id: 'call_t2',
    name: 'protocol_join',
    arguments: { name: 'handoff', participant: 'reviewer' },
  },
  {
    id: 'call_t3',
    name: 'protocol_propose',
    arguments: {
      name: 'review',
      title: '评审',
      purpose: '提交后由第二方给结论',
      messages: ['submit', 'verdict'],
    },
  },
  {
    id: 'call_t4',
    name: 'protocol_join',
    arguments: { name: 'review', participant: 'main' },
  },
  {
    id: 'call_t5',
    name: 'protocol_join',
    arguments: { name: 'review', participant: 'reviewer' },
  },
];

const ASK = '提出 handoff 和 review 两个协议，并让相关方加入';
const ASK_OFF = '再提出一个协议：delete_all';

function scenarios() {
  return [
    {
      id: 'protocol-calls',
      match: '提出 handoff 和 review',
      times: 1,
      toolCalls: CALLS,
    },
    { id: 'protocol-final', afterTool: true, times: 1, content: '两个协议已登记。' },
    { id: 'off-turn', match: '再提出一个协议', times: 1, content: '好的。' },
  ];
}

function homedirFor(port) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      // 网关拉起才有 `protocol/catalog` 这条生产路由可打
      gateway: {
        inbound_enabled: true,
        inbound_protocol: 'http',
        inbound_bind: '127.0.0.1',
        inbound_port: GW,
        inbound_token: '',
        inbound_readonly: false,
      },
      session: { ...DIALOG_FACE_OFF },
    },
  });
}

/** 取第一条主线请求的工具名清单（辅助请求恒不带工具面，`DIALOG_FACE_OFF` 下也没有） */
function firstMainTools(reqs) {
  const main = reqs.find((q) => !isSilentAuxRequest(q) && envelopeOf(q).hasTools);
  return main ? toolNamesOf(main) : null;
}

export default defineCase(
  'T103 抽象协议设计（L4）：两个工具写事实、视图按第二方判成立、路由与视图同一折叠、停用后工具面消失',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);

    try {
      // ── A 工具面 ─────────────────────────────────────────────────────────
      const turn = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK,
        provider: PROVIDER_ID,
        session: SID,
      });
      assertEq(turn.code, 0, `第一轮 CLI 退出码（stderr: ${turn.stderr.slice(0, 600)}）`);

      const tools = firstMainTools(await llm.requests());
      assert(tools, '应有一条带工具面的主线请求');
      for (const name of ['protocol_propose', 'protocol_join']) {
        assert(tools.includes(name), `工具面应含 ${name}（实际: ${JSON.stringify(tools)}）`);
      }
      assert(
        turn.stdout.includes('两个协议已登记。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格：直读会话 WAL 里本插件的两条事实 ─────────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉五次有效断言落格。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const proposed = events.filter((e) => e.kind === 'protocol.proposed');
      assertEq(
        proposed.length,
        2,
        `两次提案：${JSON.stringify(proposed.map((e) => e.payload))}`,
      );
      assertEq(proposed.map((e) => e.payload?.name), ['handoff', 'review'], '提案按提出序');
      assertEq(
        proposed[0]?.payload?.messages,
        ['request', 'accept', 'done'],
        '消息序列原样落格',
      );

      const joined = events.filter((e) => e.kind === 'protocol.joined');
      assertEq(
        joined.length,
        3,
        `三次加入（2 提案 + 3 加入 = 五次有效断言）：${JSON.stringify(joined.map((e) => e.payload))}`,
      );
      assertEq(
        joined.map((e) => e.payload?.participant),
        ['reviewer', 'main', 'reviewer'],
        '加入者按加入序',
      );

      // ── 第二方读入口：`protocol/catalog` 路由是外部第二方的真实边界名 ─────
      // 网关不链接本 crate，也能拿到 owner 折出的成立状态；这是本用例唯一的折叠读口。
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GW,
      });
      try {
        await api.waitGatewayReady();
        const viaRoute = await api.invoke('protocol/catalog', {}, { session_id: SID });
        assertEq(
          viaRoute.status,
          200,
          `protocol/catalog 应成功（实际: ${JSON.stringify(viaRoute.body)}）`,
        );
        const catalog = viaRoute.body?.data;
        assertEq(catalog?.count, 2, `两个协议：${JSON.stringify(catalog)}`);
        assertEq(
          catalog?.observations,
          5,
          `五次有效断言（2 提案 + 3 加入）：${JSON.stringify(catalog)}`,
        );
        assertEq(catalog?.by_status?.draft, 1, `一个 draft：${JSON.stringify(catalog)}`);
        assertEq(
          catalog?.by_status?.established,
          1,
          `一个 established：${JSON.stringify(catalog)}`,
        );

        const handoff = catalog?.protocols?.find((p) => p.name === 'handoff');
        assert(handoff, `清单应有 handoff：${JSON.stringify(catalog)}`);
        assertEq(handoff.status, 'draft', '只有一个参与者 ⇒ 还不成立');
        assertEq(handoff.participants, ['reviewer'], '参与者按不同值去重');
        assertEq(handoff.messages, ['request', 'accept', 'done'], '消息序列按提出顺序透出');

        const review = catalog?.protocols?.find((p) => p.name === 'review');
        assert(review, `清单应有 review：${JSON.stringify(catalog)}`);
        assertEq(review.status, 'established', '第二个参与者加入 ⇒ 成立');
        assertEq(review.participants, ['main', 'reviewer'], '参与者按首次加入序');
      } finally {
        api.stop();
      }

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      const manifest = join(hd.homedir, 'protocol', 'PLUGIN.yml');
      assert(existsSync(manifest), `抽象协议设计插件应已被装配出来（缺 ${manifest}）`);
      const kept = readFileSync(manifest, 'utf8')
        .split('\n')
        .filter((l) => !/^plugin_enabled\s*:/.test(l))
        .join('\n');
      writeFileSync(manifest, `${kept.trimEnd()}\nplugin_enabled: false\n`);
      await llm.reset();

      const off = await runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: ASK_OFF,
        provider: PROVIDER_ID,
        session: SID_OFF,
      });
      assertEq(off.code, 0, `停用后 CLI 应正常退出（stderr: ${off.stderr.slice(0, 600)}）`);

      const offTools = firstMainTools(await llm.requests());
      assert(offTools, '停用后仍应有主线请求（系统照常运行）');
      for (const name of ['protocol_propose', 'protocol_join']) {
        assert(
          !offTools.includes(name),
          `停用后工具面不该再有 ${name}（实际: ${JSON.stringify(offTools)}）`,
        );
      }
      assert(
        off.stdout.includes('好的。'),
        `停用后这一轮仍应跑完（实际: ${JSON.stringify(off.stdout)}）`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);

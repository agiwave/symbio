import './_selfrun.mjs';
// T103 抽象协议设计（L4）是一条**插件能力**：两个工具（`protocol_propose` /
// `protocol_join`）写协议提案与加入，一张视图（`protocol.catalog`）折出成立状态，
// 一条只读路由（`protocol/catalog`）把同一份折叠交给第二方；摘掉插件后工具面消失而
// 系统照常运行。
//
// ## 本用例钉的四件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `protocol_propose` / `protocol_join` |
// | 事实落格 | 工具调用之后，`session/stats` 的 `capability_views['protocol.catalog']` 有两条协议 |
// | 第二方判据 | `handoff` 只有一个参与者仍是 `draft`，`review` 有两个参与者才是 `established` |
// | 第二方读入口 | 网关 `/invoke` 调 `protocol/catalog`，拿到的值与视图**逐字相同** |
//
// 第三段是「协议需要第二方」的可执行形式：同一个行为既是提案也是加入，是否成立不由
// 谁声明，而由**不同参与者**数决定。第四段钉住路由与视图没有第二份折叠——第二方
// 从网关看到的成立状态，必须与宿主里任何插件读到的相同。
//
// ## 为什么经 `session/stats` 读视图
//
// 视图是 owner 对自己事实的纯折叠，读它必须经**生产读出口**（`session/stats` 遍历
// 清单生成的公开表，D-221）——只有这条路能证明「装配真的把 owner 的 `ViewDecl`
// 冻结进了同一张表」。直接调用本插件的求值函数是同义反复。
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
      // 网关拉起才有 `session/stats` 与 `protocol/catalog` 两条生产路由可打
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

      // ── B 事实落格 + 第二方判据 + 路由与视图同源 ─────────────────────────
      const api = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GW,
      });
      try {
        await api.waitGatewayReady();

        const inv = await api.invoke('session/stats', {}, { session_id: SID });
        assertEq(inv.status, 200, `session/stats 应成功（实际: ${JSON.stringify(inv.body)}）`);

        const views = inv.body?.data?.capability_views;
        const view = views?.['protocol.catalog'];
        assert(view, `能力视图列应有 protocol.catalog 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(view.status, 'ok', `跨插件读应成功：${JSON.stringify(view)}`);
        assertEq(view.owner, 'protocol', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');

        assertEq(view.value?.count, 2, `两个协议：${JSON.stringify(view.value)}`);
        assertEq(
          view.value?.observations,
          5,
          `五次有效断言（2 提案 + 3 加入）：${JSON.stringify(view.value)}`,
        );
        assertEq(view.value?.by_status?.draft, 1, `一个 draft：${JSON.stringify(view.value)}`);
        assertEq(
          view.value?.by_status?.established,
          1,
          `一个 established：${JSON.stringify(view.value)}`,
        );

        const handoff = view.value?.protocols?.find((p) => p.name === 'handoff');
        assert(handoff, `清单应有 handoff：${JSON.stringify(view.value)}`);
        assertEq(handoff.status, 'draft', '只有一个参与者 ⇒ 还不成立');
        assertEq(handoff.participants, ['reviewer'], '参与者按不同值去重');
        assertEq(handoff.messages, ['request', 'accept', 'done'], '消息序列按提出顺序透出');

        const review = view.value?.protocols?.find((p) => p.name === 'review');
        assert(review, `清单应有 review：${JSON.stringify(view.value)}`);
        assertEq(review.status, 'established', '第二个参与者加入 ⇒ 成立');
        assertEq(review.participants, ['main', 'reviewer'], '参与者按首次加入序');

        // 第二方读入口：网关不链接本 crate，也能拿到同一份折叠。
        const viaRoute = await api.invoke('protocol/catalog', {}, { session_id: SID });
        assertEq(
          viaRoute.status,
          200,
          `protocol/catalog 应成功（实际: ${JSON.stringify(viaRoute.body)}）`,
        );
        assertEq(
          viaRoute.body?.data,
          view.value,
          '路由与视图必须逐字相同——第二方看到的成立状态不能分叉',
        );
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

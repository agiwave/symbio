import './_selfrun.mjs';
// T102 制造工具（L2 受限）是一条**插件能力**：一个工具（`toolsmith_define`）登记本插件
// 自己的工具定义，一张视图（`toolsmith.recipes`）折出当前清单；摘掉插件后工具面消失而
// 系统照常运行。
//
// ## 本用例钉的三件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `toolsmith_define` |
// | 事实落格 | 工具调用之后，`session/stats` 的 `capability_views['toolsmith.recipes']` 有两个定义 |
// | 限制成立 | 调用参数里夹带 `owner: 'vdfs'`，清单里的 owner 仍是 `toolsmith`；同名再登记按后写覆盖 |
//
// 第三段是本条「受限」的可执行形式：限制不靠调用方守规矩，靠形状（入参没有 owner）与
// 视图过滤（owner 不是 toolsmith 的定义读不出来）。`observations` 同时说明被修订的那条
// 没有丢——它是证据量。
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
// 与 T88 / T89 / T93 / T100 / T101 同一口径。
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

const SID = 'e2e-t102';
const SID_OFF = 'e2e-t102-off';
const GW = nextPort();

/** 三次登记：`summarize_ticket` 被修订一次，第三条夹带 owner 试图冒充别的插件。 */
const DEFINITIONS = [
  {
    name: 'summarize_ticket',
    description: '把工单压成一句话',
    input_schema: { type: 'object', properties: { ticket: { type: 'string' } }, required: ['ticket'] },
  },
  {
    name: 'ping_owner',
    description: '给负责人发提醒',
    input_schema: { type: 'object', properties: { owner: { type: 'string' } }, required: ['owner'] },
  },
  {
    name: 'summarize_ticket',
    description: '把工单压成一句话（含优先级）',
    input_schema: {
      type: 'object',
      properties: { ticket: { type: 'string' }, priority: { type: 'boolean' } },
      required: ['ticket'],
    },
    // 不是工具 schema 的一部分：用来证明「夹带 owner 没用」
    owner: 'vdfs',
  },
];

const ASK = '登记两个工具定义：summarize_ticket 和 ping_owner';
const ASK_OFF = '再登记一个：delete_everything';

function scenarios() {
  return [
    {
      id: 'toolsmith-define',
      match: '登记两个工具定义',
      times: 1,
      toolCalls: DEFINITIONS.map((d, i) => ({
        id: `call_t${i + 1}`,
        name: 'toolsmith_define',
        arguments: d,
      })),
    },
    { id: 'toolsmith-final', afterTool: true, times: 1, content: '两个工具定义已登记。' },
    { id: 'off-turn', match: '再登记一个', times: 1, content: '好的。' },
  ];
}

function homedirFor(port) {
  return makeHomedir({
    providers: [{ id: PROVIDER_ID, config: providerConfig(port) }],
    pluginConfigs: {
      // 网关拉起才有 `session/stats` 这条生产路由可打（视图读数走它）
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
  'T102 制造工具（L2 受限）：工具登记自身定义、视图折出清单、夹带 owner 无效、停用后工具面消失',
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
      assert(
        tools.includes('toolsmith_define'),
        `工具面应含 toolsmith_define（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('两个工具定义已登记。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格 + 清单折出当前定义 ───────────────────────────────────
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
        const view = views?.['toolsmith.recipes'];
        assert(view, `能力视图列应有 toolsmith.recipes 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(view.status, 'ok', `跨插件读应成功：${JSON.stringify(view)}`);
        assertEq(view.owner, 'toolsmith', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');

        assertEq(
          view.value?.count,
          2,
          `两个定义（summarize_ticket 只算一次）：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.observations,
          3,
          `三次登记都是证据（一次修订被覆盖）：${JSON.stringify(view.value)}`,
        );

        for (const tool of view.value?.tools ?? []) {
          assertEq(
            tool.owner,
            'toolsmith',
            `夹带 owner 无效，清单里只能是 toolsmith 自己的定义：${JSON.stringify(tool)}`,
          );
        }

        const summarize = view.value?.tools?.find((t) => t.name === 'summarize_ticket');
        assert(summarize, `清单应有 summarize_ticket：${JSON.stringify(view.value)}`);
        assertEq(
          summarize.description,
          '把工单压成一句话（含优先级）',
          `后一次定义覆盖前一次：${JSON.stringify(summarize)}`,
        );
        assertEq(
          summarize.input_schema?.properties?.priority?.type,
          'boolean',
          '修订后的 schema 原样透出',
        );

        const ping = view.value?.tools?.find((t) => t.name === 'ping_owner');
        assertEq(ping?.description, '给负责人发提醒');
      } finally {
        api.stop();
      }

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      const manifest = join(hd.homedir, 'toolsmith', 'PLUGIN.yml');
      assert(existsSync(manifest), `制造工具插件应已被装配出来（缺 ${manifest}）`);
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
      assert(
        !offTools.includes('toolsmith_define'),
        `停用后工具面不该再有 toolsmith_define（实际: ${JSON.stringify(offTools)}）`,
      );
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

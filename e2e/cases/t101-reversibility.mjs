import './_selfrun.mjs';
// T101 可逆性设计是一条**插件能力**：一个工具（`reversibility_annotate`）落下领域事实，
// 一张视图（`reversibility.register`）把它们折成「每个动作当前等级 + 等级分布」；
// 摘掉插件后工具面消失而系统照常运行。
//
// ## 本用例钉的三件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `reversibility_annotate` |
// | 事实落格 | 工具调用之后，`session/stats` 的 `capability_views['reversibility.register']` 有三次登记 |
// | 判断可修正 | 同一动作的第二次标注把第一次压过（`改配置` 从可补偿升为不可逆），`observations` 仍记四次 |
//
// 第三段是这张能力与「把事实原样吐出来」的分水岭：等级分布 `by_level` 必须按**当前**
// 判断算（`compensable` 归零、`irreversible` 计二），而 `observations` 同时说明被覆盖的
// 那条没有丢——它是证据量。
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
// 与 T88 / T89 / T93 / T100 同一口径。
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

const SID = 'e2e-t101';
const SID_OFF = 'e2e-t101-off';
const GW = nextPort();

/** 四次标注：`改配置` 被判两次（后一次更保守），其余各一次。 */
const ASSESSMENTS = [
  { action: '删缓存', level: 'reversible' },
  { action: '发邮件给客户', level: 'irreversible' },
  { action: '改配置', level: 'compensable', undo: '把旧配置写回' },
  { action: '改配置', level: 'irreversible' },
];

const ASK = '标注这几个动作：删缓存、发邮件给客户、改配置';
const ASK_OFF = '再标注一个：删日志';

function scenarios() {
  return [
    {
      id: 'reversibility-annotate',
      match: '标注这几个动作',
      times: 1,
      toolCalls: ASSESSMENTS.map((a, i) => ({
        id: `call_r${i + 1}`,
        name: 'reversibility_annotate',
        arguments: a,
      })),
    },
    { id: 'reversibility-final', afterTool: true, times: 1, content: '这几个动作已标注。' },
    { id: 'off-turn', match: '再标注一个', times: 1, content: '好的。' },
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
  'T101 可逆性设计：工具落可逆等级、视图按动作折出当前分布、停用后工具面消失而会话照常',
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
        tools.includes('reversibility_annotate'),
        `工具面应含 reversibility_annotate（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('这几个动作已标注。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格 + 视图折出当前分布 ───────────────────────────────────
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
        const view = views?.['reversibility.register'];
        assert(view, `能力视图列应有 reversibility.register 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(view.status, 'ok', `跨插件读应成功：${JSON.stringify(view)}`);
        assertEq(view.owner, 'reversibility', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');

        assertEq(
          view.value?.count,
          3,
          `三个动作（改配置只算一次）：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.observations,
          4,
          `四次标注都是证据（一次被覆盖）：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.by_level?.reversible,
          1,
          `可逆动作一个：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.by_level?.compensable,
          0,
          `改配置已升为不可逆，可补偿归零：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.by_level?.irreversible,
          2,
          `不可逆动作两个（发邮件、改配置）：${JSON.stringify(view.value)}`,
        );

        const config = view.value?.actions?.find((a) => a.action === '改配置');
        assert(config, `动作清单应有改配置：${JSON.stringify(view.value)}`);
        assertEq(config.level, 'irreversible', `后一次判断覆盖前一次：${JSON.stringify(config)}`);
        assertEq(config.undo, '', '不可逆动作没有补偿做法');

        const send = view.value?.actions?.find((a) => a.action === '发邮件给客户');
        assertEq(send?.level, 'irreversible');
        const cache = view.value?.actions?.find((a) => a.action === '删缓存');
        assertEq(cache?.level, 'reversible');
      } finally {
        api.stop();
      }

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      const manifest = join(hd.homedir, 'reversibility', 'PLUGIN.yml');
      assert(existsSync(manifest), `可逆性插件应已被装配出来（缺 ${manifest}）`);
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
        !offTools.includes('reversibility_annotate'),
        `停用后工具面不该再有 reversibility_annotate（实际: ${JSON.stringify(offTools)}）`,
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

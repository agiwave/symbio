import './_selfrun.mjs';
// T100 世界模型是一条**插件能力**：一个工具（`world_observe`）落下领域事实，一张视图
// （`world.model`）把它们折成「当前世界」；摘掉插件后工具面消失而系统照常运行。
//
// ## 本用例钉的三件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `world_observe` |
// | 事实落格 | 工具调用之后，`session/stats` 的 `capability_views['world.model']` 有两条观察 |
// | 后写覆盖 | 同一实体同一属性的第二条观察把第一条压过（`状态 = 离线`，置信度跟随后一条） |
//
// 第三段是这张视图与「把事实原样吐出来」的分水岭：`count` 只证明落格，覆盖才证明
// 折叠真的按「现在是什么」算过。`observations` 同时说明被覆盖的那条没有丢——它是证据量。
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
// 与 T88 / T89 / T93 同一口径。
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

const SID = 'e2e-t100';
const SID_OFF = 'e2e-t100-off';
const GW = nextPort();

/** 三次观察：`服务器/状态` 被观察两次（后写覆盖），`数据库/版本` 是另一个实体。 */
const OBSERVATIONS = [
  { entity: '服务器', attribute: '状态', value: '在线', confidence: 80 },
  { entity: '服务器', attribute: '状态', value: '离线', confidence: 60 },
  { entity: '数据库', attribute: '版本', value: '16', confidence: 90 },
];

const ASK = '记下这几条观察：服务器先在线、后来离线，数据库版本 16';
const ASK_OFF = '再记一条：磁盘满了';

function scenarios() {
  return [
    {
      id: 'world-observe',
      match: '记下这几条观察',
      times: 1,
      // 一条响应里三次调用：两次同键观察让覆盖在同一轮可见
      toolCalls: OBSERVATIONS.map((o, i) => ({
        id: `call_w${i + 1}`,
        name: 'world_observe',
        arguments: o,
      })),
    },
    { id: 'world-final', afterTool: true, times: 1, content: '这几条观察已记下。' },
    // 停用侧：工具已经不存在，模型只能给一句话
    { id: 'off-turn', match: '再记一条', times: 1, content: '好的。' },
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
  'T100 世界模型：工具落领域事实、视图按实体折出当前值、停用后工具面消失而会话照常',
  async () => {
    const llm = await new MockLlm(scenarios()).start();
    const hd = homedirFor(llm.port);

    try {
      // ── A 工具面：能力包的工具有没有出现在模型看得到的地方 ──────────────
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
        tools.includes('world_observe'),
        `工具面应含 world_observe（实际: ${JSON.stringify(tools)}）`,
      );
      assert(
        turn.stdout.includes('这几条观察已记下。'),
        `第一轮 stdout 应为收尾正文（实际: ${JSON.stringify(turn.stdout)}）`,
      );

      // ── B 事实落格 + 视图折出当前世界：经生产读出口读 owner 的折叠 ───────
      //
      // 事实在**轮末**写口落格（D-213），因此读数必须在同一进程里读同一份存储——
      // 故这里起一个长驻 CLI 并等网关就绪，而不是再跑一个 one-shot。
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
        const view = views?.['world.model'];
        assert(view, `能力视图列应有 world.model 这一格（实际: ${JSON.stringify(views)}）`);
        assertEq(view.status, 'ok', `跨插件读应成功，不是缺席态：${JSON.stringify(view)}`);
        assertEq(view.owner, 'world_model', 'owner 来自清单生成物');
        assertEq(view.ver, 1, '版本随 owner 的 ViewDecl 走');

        assertEq(
          view.value?.count,
          2,
          `两个实体（服务器、数据库）：${JSON.stringify(view.value)}`,
        );
        assertEq(
          view.value?.observations,
          3,
          `三次观察都是证据（一次被覆盖）：${JSON.stringify(view.value)}`,
        );

        const server = view.value?.entities?.find((e) => e.entity === '服务器');
        const db = view.value?.entities?.find((e) => e.entity === '数据库');
        assert(server, `实体清单应有服务器：${JSON.stringify(view.value)}`);
        assert(db, `实体清单应有数据库：${JSON.stringify(view.value)}`);
        assertEq(
          server.attributes?.['状态']?.value,
          '离线',
          `同实体同属性后写覆盖：${JSON.stringify(server)}`,
        );
        assertEq(
          server.attributes?.['状态']?.confidence,
          60,
          '置信度跟随后一条观察',
        );
        assertEq(
          !('状态' in (db.attributes ?? {})),
          true,
          `别的实体的属性不能漏进这一格：${JSON.stringify(db)}`,
        );
        assertEq(db.attributes?.['版本']?.value, '16');
      } finally {
        api.stop();
      }

      // ── C 卸任：工具面消失，会话照常跑完 ─────────────────────────────────
      //    「可卸载」的可执行形式是**装配期**不构造（`plugin_enabled: false`），
      //    而不是运行期的某个开关分支。
      const manifest = join(hd.homedir, 'world_model', 'PLUGIN.yml');
      assert(existsSync(manifest), `世界模型插件应已被装配出来（缺 ${manifest}）`);
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
        !offTools.includes('world_observe'),
        `停用后工具面不该再有 world_observe（实际: ${JSON.stringify(offTools)}）`,
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

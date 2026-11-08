import './_selfrun.mjs';
// T102 制造工具（L2 受限）是一条**插件能力**：一个工具（`toolsmith_define`）登记本插件
// 自己的工具定义；摘掉插件后工具面消失而系统照常运行。视图已收回插件私有，e2e 只钉
// 事实落格；折叠由 owner 的 `decl.test.rs` 覆盖。
//
// ## 本用例钉的两件事
//
// | 段 | 唯一能看见它的地方 |
// |---|---|
// | 工具面 | 首次请求的 `tools` 里有 `toolsmith_define` |
// | 事实落格 | 直读会话 WAL 的 `toolsmith.defined` 事件有三次登记，owner 恒为 `toolsmith` |
//
// 限制的可执行形式在事实载荷里：调用参数夹带 `owner: 'vdfs'`，落格的定义 owner 仍是
// `toolsmith`（写方写死）；同名再登记留下两条事实，后一条描述是修订后的版本。
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
  runCli,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';
import { toolNamesOf, envelopeOf, isSilentAuxRequest } from '../llm-contract.mjs';

const SID = 'e2e-t102';
const SID_OFF = 'e2e-t102-off';

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
  'T102 制造工具（L2 受限）：工具登记自身定义、夹带 owner 无效、停用后工具面消失',
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

      // ── B 事实落格：直读会话 WAL 里本插件自己的领域事实 ─────────────────
      //
      // 事实在**轮末**写口落格（D-213）；one-shot CLI 已退出，WAL 就是事实源本身。
      // 视图已收回插件私有，这里只钉三次登记都落格、owner 恒为 toolsmith。
      const events = readFileSyncSafe(
        join(hd.homedir, 'session', SID, 'v2-events.wal'),
      )
        .split('\n')
        .filter(Boolean)
        .map((l) => JSON.parse(l));

      const defined = events.filter((e) => e.kind === 'toolsmith.defined');
      assertEq(
        defined.length,
        3,
        `三次登记都落格（含被修订的那条）：${JSON.stringify(defined.map((e) => e.payload))}`,
      );
      for (const e of defined) {
        assertEq(
          e.payload?.owner,
          'toolsmith',
          `夹带 owner 无效，落格的 owner 只能是 toolsmith：${JSON.stringify(e.payload)}`,
        );
      }
      assertEq(defined[0]?.payload?.name, 'summarize_ticket', '第一次登记的工具名');
      assertEq(defined[1]?.payload?.name, 'ping_owner', '第二次登记的工具名');
      assertEq(defined[1]?.payload?.description, '给负责人发提醒', '第二个工具的描述');
      assertEq(defined[2]?.payload?.name, 'summarize_ticket', '第三次又是同名工具');
      assertEq(
        defined[2]?.payload?.description,
        '把工单压成一句话（含优先级）',
        '第二条同名事实是修订后的描述',
      );
      assertEq(
        defined[2]?.payload?.input_schema?.properties?.priority?.type,
        'boolean',
        '修订后的 schema 原样落格',
      );

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

import './_selfrun.mjs';
// T59 工具面契约（LC-005）：发给模型的工具集合 = 出厂集 + 用例声明的增量，不多不少。
//
// ## 本用例钉的是什么
//
// 工具面是「模型能做什么」的**唯一声明**：少一个 = 能力静默消失（回答照常，
// 只是模型不再有那个选择）；多一个 = 内部面泄漏或幽灵工具（未连接的 MCP、
// 空 enum 的条件工具）。响应侧对两者都没有形态——判据只能从请求体取。
//
// 四种形态逐条判（一个 homedir 一种形态，各自独立可红）：
// ① 出厂面：默认 homedir ⇒ 恰好 19 个内置工具，与 `docs/CURRENT.md` §2 对账
//    （`FACTORY_TOOLS` 常量是运行时事实的唯一副本；那次文档修复——ToolSpec
//     表模式补抽取——正是这个对账发现的，见 T-096）；
// ② 条件工具：落一个技能 ⇒ `read_skill` 进场（无技能时它**不在**场——
//    空 enum 的工具给了模型也选不出东西）；
// ③ MCP 桥接：连一个 mock server ⇒ `mcp__<server>__<tool>` 进场；
// ④ 内部面黑名单：路由 / 插件管理 / 通道类名字永远不得进场。
//
// 另判子智能体挂载形态（LC-005b）：绑 agent 后根工具一个不少，且每个根工具
// 都有 `agent_<id>_<工具>` 对偶——子空间能力经前缀寻址（t30 主体不串包的
// 名字基础；对偶缺一半 = 委派面残缺）。
import { join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  addMcpServer,
  makeAgentDir,
  runCli,
  E2E_ROOT,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
} from '../helpers.mjs';
import {
  PLATFORM_VARIANT_TOOL,
  PLATFORM_VARIANT_TOOL_NAMES,
} from '../../scripts/prompt-shape-audit.mjs';
import {
  FACTORY_TOOLS,
  assertToolSurface,
  assertAgentPrefixedTwins,
  toolNamesOf,
} from '../llm-contract.mjs';

/** wire 名里的 shell 平台变体归一（与 prompt-shape-audit 同一记号，不抄第二份规则） */
const normalizeShell = (name) =>
  PLATFORM_VARIANT_TOOL_NAMES.includes(name) ? PLATFORM_VARIANT_TOOL : name;

const U = '随便说点什么';

/** 落一个技能（`<homedir>/skills/<id>/SKILL.md`——默认 `skill_dirs` 第二来源，
 *  `{HOMEDIR}/skills`，见 skill `plugin.rs` 的默认配置；格式见 loader.test.rs 夹具） */
function makeSkill(homedir, id) {
  const dir = join(homedir, 'skills', id);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, 'SKILL.md'),
    `---\nname: ${id}\ndescription: T59 条件工具探针技能\n---\n\n技能正文：按步骤来。\n`,
  );
}

export default defineCase(
  'T59 工具面契约：出厂 19 + 条件工具 + MCP 桥接 + 内部面黑名单 + agent 对偶（LC-005）',
  async () => {
    const llm = await new MockLlm([{ id: 'rest', content: '收到。' }]).start();

    try {
      // ── ① 出厂面：恰好 19 个，一个不多一个不少 ─────────────────────────
      // 四个场景共用同一个 mock 实例 ⇒ 每个场景开跑前清空请求记录，
      // 否则读到的 reqs[0] 永远是场景 ① 的那条（症状：后面的场景全红在
      // 「缺 read_skill」，真因是读错了请求）。
      await llm.reset();
      const hd1 = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
      });
      try {
        const r1 = runCli({
          homedir: hd1.homedir,
          workdir: hd1.workdir,
          message: U,
          provider: PROVIDER_ID,
          session: 'e2e-t59-a',
        });
        assertEq(r1.code, 0, `出厂面 CLI 退出码（stderr: ${r1.stderr.slice(0, 600)}）`);
        const reqs = await llm.requests();
        assertEq(reqs.length, 1, '应恰好一次请求');
        assertToolSurface(reqs[0], {}, 'T59① 出厂面');
      } finally {
        cleanupHomedir(hd1);
      }

      // ── ② 条件工具：有技能 ⇒ read_skill 进场 ────────────────────────────
      await llm.reset();
      const hd2 = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
      });
      makeSkill(hd2.homedir, 'probe-skill');
      try {
        const r2 = runCli({
          homedir: hd2.homedir,
          workdir: hd2.workdir,
          message: U,
          provider: PROVIDER_ID,
          session: 'e2e-t59-b',
        });
        assertEq(r2.code, 0, `技能面 CLI 退出码（stderr: ${r2.stderr.slice(0, 600)}）`);
        const reqs = await llm.requests();
        assertEq(reqs.length, 1, '技能面应恰好一次请求');
        assertToolSurface(reqs[reqs.length - 1], { extra: ['read_skill'] }, 'T59② 技能条件工具');
      } finally {
        cleanupHomedir(hd2);
      }

      // ── ③ MCP 桥接：连一个 server ⇒ mcp__mockserv__echo 进场 ────────────
      await llm.reset();
      const hd3 = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
      });
      addMcpServer(hd3, 'mockserv', {
        type: 'stdio',
        command: process.execPath,
        args: [join(E2E_ROOT, 'e2e', 'mock-mcp.mjs')],
        enabled: true,
      });
      try {
        const r3 = runCli({
          homedir: hd3.homedir,
          workdir: hd3.workdir,
          message: U,
          provider: PROVIDER_ID,
          session: 'e2e-t59-c',
        });
        assertEq(r3.code, 0, `MCP 面 CLI 退出码（stderr: ${r3.stderr.slice(0, 600)}）`);
        const reqs = await llm.requests();
        assertEq(reqs.length, 1, 'MCP 面应恰好一次请求');
        // mock-mcp 默认暴露两个工具（echo / add，见 mock-mcp.mjs 的默认 tools）——
        // 桥接工具按已连接 server 的 tools/list 动态生成，两个都该进场。
        assertToolSurface(
          reqs[reqs.length - 1],
          { extra: ['mcp__mockserv__echo', 'mcp__mockserv__add'] },
          'T59③ MCP 桥接工具',
        );
      } finally {
        cleanupHomedir(hd3);
      }

      // ── ④ 子智能体挂载：根工具不少 + agent 前缀对偶齐 ────────────────────
      await llm.reset();
      const hd4 = makeHomedir({
        providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }],
        pluginConfigs: { session: { ...DIALOG_FACE_OFF } },
      });
      makeAgentDir(hd4.homedir, {
        id: 't59-agent',
        name: 'T59 智能体',
        persona: '你是 T59 智能体。',
        providerId: PROVIDER_ID,
        providerPort: llm.port,
      });
      try {
        const r4 = runCli({
          homedir: hd4.homedir,
          workdir: hd4.workdir,
          message: U,
          provider: PROVIDER_ID,
          agent: 't59-agent',
          session: 'e2e-t59-d',
        });
        assertEq(r4.code, 0, `agent 面 CLI 退出码（stderr: ${r4.stderr.slice(0, 600)}）`);
        const reqs = await llm.requests();
        assertEq(reqs.length, 1, 'agent 面应恰好一次请求');
        assertAgentPrefixedTwins(reqs[reqs.length - 1], 't59-agent', 'T59④ agent 对偶');
        // 对偶集合与出厂集**双向相等**：根工具全有对偶（上一步已判），对偶也
        // 不许有「子树独有」的未声明工具（那是收集广播把别的东西也带进来了）。
        // shell 平台变体两侧同一规则，归一后比。
        const prefix = 'agent_t59-agent_';
        const actualTwins = toolNamesOf(reqs[reqs.length - 1])
          .filter((n) => n.startsWith(prefix))
          .map((n) => normalizeShell(n.slice(prefix.length)));
        const expectedTwins = FACTORY_TOOLS.map(normalizeShell).sort();
        const actualSorted = [...new Set(actualTwins)].sort();
        assertEq(
          JSON.stringify(actualSorted),
          JSON.stringify(expectedTwins),
          'agent 前缀对偶应与出厂集逐项相等（多一个 = 子树独有工具泄漏，少一个 = 委派面残缺）',
        );
      } finally {
        cleanupHomedir(hd4);
      }
    } finally {
      llm.stop();
    }
  },
);

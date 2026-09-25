import './_selfrun.mjs';
// T17 插件装配全链路：空 homedir → 装配（身份落位）→ 工具调用 → 对话收尾 → 停用生效。
//
// 与 T2/T3 的分工：那两个用例验的是「工具回路怎么走」，本用例验的是**插件从哪来**——
// 插件目录与 `PLUGIN.yml` 都不是夹具预置的，而是这一次跑起来才由容器装配出来的。
import { join } from 'node:path';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import {
  MockLlm,
  makeHomedir,
  cleanupHomedir,
  runCli,
  readMessagesJson,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  assertTranscriptInvariants,
} from '../helpers.mjs';

/** 构造者声明的必需插件（`home::SYSTEM_PLUGINS`）中的一组代表 */
const REQUIRED_PLUGINS = ['agent', 'local', 'mcp', 'model', 'session', 'skill', 'vdfs', 'work'];

/** 从 `PLUGIN.yml` 正文里取一个顶层标量（剥掉可选的 YAML 引号） */
function yamlValue(text, key) {
  const m = text.match(new RegExp(`^${key}:\\s*"?([^"\\n]*?)"?\\s*$`, 'm'));
  return m ? m[1] : null;
}

export default defineCase(
  'T17 插件装配全链路：空目录 → 装配（身份落位）→ 工具调用 → 收尾 → 停用生效',
  async () => {
    // 场景不标 `once`：第二轮（停用后）要复用同一份编排
    const llm = await new MockLlm([
      {
        id: 'call-write',
        match: '写文件',
        toolCalls: [
          { id: 'call_w1', name: 'vdfs_write', arguments: { path: 'notes.md', text: '# T17\n由 mock 写入' } },
        ],
      },
      { id: 'after-write', afterTool: true, content: '文件已写入。' },
    ]).start();
    // 只给 model provider —— **不预置任何插件**：插件树由这一次运行装配出来
    const hd = makeHomedir({ providers: [{ id: PROVIDER_ID, config: providerConfig(llm.port) }] });
    try {
      const r1 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我写文件',
        provider: PROVIDER_ID,
        session: 'e2e-t17',
      });
      assertEq(r1.code, 0, `CLI 退出码（stderr: ${r1.stderr.slice(0, 400)}）`);

      // ① 装配：每个必需插件的目录与 manifest 都是这次补出来的（此前磁盘上什么都不存在）
      for (const p of REQUIRED_PLUGINS) {
        const yml = join(hd.homedir, p, 'PLUGIN.yml');
        assert(existsSync(yml), `插件 ${p} 应被装配出来（缺 ${yml}）`);
        const text = readFileSync(yml, 'utf8');
        assertEq(yamlValue(text, 'plugin_provider'), p, `${p} 的 plugin_provider`);
        assertEq(yamlValue(text, 'plugin_name'), p, `${p} 的 plugin_name`);
      }

      // ② ADR-032：出厂身份在**装配期**投影进 manifest
      //    这是「停用插件也有身份」的唯一来路——它只在插件被构造的那一刻落盘一次。
      const localYml = readFileSync(join(hd.homedir, 'local', 'PLUGIN.yml'), 'utf8');
      assertEq(yamlValue(localYml, 'plugin_title'), '本地工具', 'local 的出厂标题应投影进 manifest');
      assertEq(yamlValue(localYml, 'plugin_version'), '0.1.0', 'local 的出厂版本应投影进 manifest');
      assert(yamlValue(localYml, 'plugin_description'), 'local 的出厂描述应投影进 manifest');

      // ③ 工具调用回路：文件真实落盘 + 转写不变量
      const written = readFileSync(join(hd.workdir, 'notes.md'), 'utf8');
      assert(written.includes('# T17'), `文件内容应写入工作目录（实际: ${JSON.stringify(written)}）`);

      const msgs = readMessagesJson(hd.homedir, 'e2e-t17');
      assertTranscriptInvariants(msgs, 'T17');
      const tc = msgs.find((m) => m.type === 'tool_call');
      assert(tc, '应有 tool_call 节点');
      assertEq(tc.status, 'completed', 'tool_call 终态应为 completed');

      // ④ 停用：装配位写盘后，该插件**不再被构造** ⇒ 它提供的工具从清单里消失。
      //    工具名不硬编码（各 OS 的 shell 工具名不同），按「首轮有、次轮没有」的差集判。
      const reqs1 = await llm.requests();
      const tools1 = (reqs1[0].body.tools ?? []).map((t) => t.function?.name);
      assert(tools1.includes('vdfs_write'), `首轮 tools 应含 vdfs_write（实际: ${tools1.join(',')}）`);

      const ymlPath = join(hd.homedir, 'local', 'PLUGIN.yml');
      writeFileSync(ymlPath, `${readFileSync(ymlPath, 'utf8')}plugin_enabled: false\n`);
      await llm.reset();

      const r2 = runCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        message: '帮我写文件',
        provider: PROVIDER_ID,
        session: 'e2e-t17-b',
      });
      assertEq(r2.code, 0, `停用后 CLI 退出码（stderr: ${r2.stderr.slice(0, 400)}）`);
      assert(existsSync(join(hd.homedir, 'local')), '停用不删目录（那是卸载的语义）');

      const reqs2 = await llm.requests();
      const tools2 = (reqs2[0].body.tools ?? []).map((t) => t.function?.name);
      const gone = tools1.filter((t) => !tools2.includes(t));
      assert(
        gone.length > 0,
        `停用 local 后它提供的工具应消失（首轮 ${tools1.length} 个 → 次轮 ${tools2.length} 个）`,
      );
    } finally {
      llm.stop();
      cleanupHomedir(hd);
    }
  },
);

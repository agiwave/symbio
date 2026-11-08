import './_selfrun.mjs';
// T104 驱动安全阀出口的**反向**用例（`D-393`）。
//
// 两条安全阀（`Exhausted` / `Stalled`）原先只有 `eprintln!` 一个出口：CLI 的 stderr
// 就是调用面的读数，但 Tauri / gateway 这类没有控制台的宿主读不到。出口落成一条
// 机制事实 `driver.stalled`（`crates/symbio/src/drive.rs`），会话投影再把它折成
// `outcome = "failed"`（`symbio-plugin-session` 的 `runtime`）。
//
// 正向（停摆必须落格）由 Rust 用例钉住，且能在 CI 里判红：
// - `crates/symbio/src/drive.test.rs`：两条出口各落一条、正常出口一条不落；
// - `crates/symbio-plugin-session/src/runtime.test.rs`：事实 → `failed` + 文案。
//
// ## 为什么正向不放在 e2e 里
//
// e2e 用的真实装配里，工具结果会**开新一轮**（每一轮各自有完整的步数预算），
// 一圈跑不到 128 步；`Stalled` 又要求「有债、前提端口本圈无动作」。今天没有一条
// 从用户输入可达的路径能确定性地把安全阀顶开——硬凑成「模型无限调用工具」只会
// 撞上 e2e 的静止超时，而不是断言红（试过：T# 一路涨到 600+，始终不出安全阀）。
// 因此 e2e 只钉**反向**：正常轮不得落这条事实、不得在 stderr 出声。
import { join } from 'node:path';
import {
  MockLlm,
  makeHomedir,
  DIALOG_FACE_OFF,
  cleanupHomedir,
  nextPort,
  startLongLivedCli,
  waitFor,
  assert,
  assertEq,
  defineCase,
  providerConfig,
  PROVIDER_ID,
  readFileSyncSafe,
} from '../helpers.mjs';

const SID = 'e2e-t104';
const GATEWAY_PORT = nextPort();
const ASK = '普通提问';

const factsOf = (homedir) =>
  readFileSyncSafe(join(homedir, 'session', SID, 'v2-events.wal'))
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l));

export default defineCase(
  'T104 驱动安全阀：正常轮不落 driver.stalled、stderr 不出声、会话节点仍 completed',
  async () => {
    const llm = await new MockLlm([
      { id: 'normal', match: ASK, times: 1, content: '这是正常回答。' },
    ]).start();

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

    let cli = null;
    try {
      cli = startLongLivedCli({
        homedir: hd.homedir,
        workdir: hd.workdir,
        session: SID,
        provider: PROVIDER_ID,
        gatewayPort: GATEWAY_PORT,
      });
      await cli.waitGatewayReady();

      cli.send(ASK);
      await waitFor(() => factsOf(hd.homedir).some((e) => e.kind === 'chat.assistant.final'), {
        what: '正常轮落到 chat.assistant.final',
        timeoutMs: 30_000,
      });

      // ① 事实面：正常出口（没欠债 / 在飞 / 交还人工）不落安全阀事实。
      const stalls = factsOf(hd.homedir).filter((e) => e.kind === 'driver.stalled');
      assertEq(stalls.length, 0, `正常轮不得落 driver.stalled：${JSON.stringify(stalls)}`);

      // ② CLI 面：文案只在安全阀触发时出现。
      assert(
        !cli.out.stderr.includes('驱动停摆('),
        `正常轮 stderr 不得有安全阀文案（尾部：${cli.out.stderr.slice(-300)}）`,
      );

      // ③ 非 CLI 面：会话节点（VDFS `vdfs/stat`，Tauri 前端读的就是它）仍是 completed。
      const root = await cli.invoke('vdfs/root', {});
      const rootPath = root.body?.data?.path;
      assert(typeof rootPath === 'string' && rootPath.length > 0, 'vdfs/root 应有根地址');
      const url = `${rootPath.replace(/\/+$/, '')}/session/${SID}`;
      await waitFor(
        async () => (await cli.invoke('vdfs/stat', { path: url })).body?.data?.outcome === 'completed',
        { what: '会话节点 outcome=completed', timeoutMs: 20_000 },
      );
      const node = (await cli.invoke('vdfs/stat', { path: url })).body?.data;
      assertEq(node.error ?? null, null, '正常轮不得带停摆 error');
    } finally {
      if (process.env.E2E_KEEP) console.log('DEBUG homedir:', hd.homedir);
      if (!process.env.E2E_KEEP) {
        try {
          cli?.stop();
        } catch {
          /* 已退出 */
        }
        llm.stop();
        cleanupHomedir(hd);
      }
    }
  },
);

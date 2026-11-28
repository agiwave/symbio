import './_selfrun.mjs';
// T113 插件市场完整闭环：候选由后端现场计算，安装 → 停用 → 启用 → 卸载都经同一棵 VDFS。
//
// 这是「模拟市场」而不是远程下载：候选来自本构建已链接的可选插件工厂。
// 它钉的是已实现的市场契约，不把外部插件包/更新偷渡进本批。
import {
  makeHomedir,
  cleanupHomedir,
  nextPort,
  startLongLivedCli,
  assert,
  assertEq,
  defineCase,
} from '../helpers.mjs';

const PLUGIN = 'slo';

export default defineCase(
  'T113 插件市场：模拟候选发现、安装、停用、启用与卸载全链路',
  async () => {
    const port = nextPort();
    const hd = makeHomedir({
      pluginConfigs: {
        gateway: {
          inbound_enabled: true,
          inbound_protocol: 'http',
          inbound_bind: '127.0.0.1',
          inbound_port: port,
          inbound_token: '',
          inbound_readonly: false,
        },
      },
    });
    const cli = startLongLivedCli({
      homedir: hd.homedir,
      workdir: hd.workdir,
      session: 'unused-t113',
      gatewayPort: port,
    });
    const dataOf = (resp, what) => {
      assertEq(resp.status, 200, `${what} 应受理（${JSON.stringify(resp.body)?.slice(0, 300)}）`);
      assert(resp.body?.data != null, `${what} 应返回 Data 载荷`);
      return resp.body.data;
    };
    const names = (resp) => (resp.items ?? []).map((item) => item.path?.split('/').pop());
    const stateOf = (resp) => {
      const text = typeof resp.text === 'string' ? resp.text : JSON.stringify(resp);
      try { return JSON.parse(text); } catch { return {}; }
    };

    try {
      await cli.waitGatewayReady();
      const root = dataOf(await cli.invoke('vdfs/root', {}), 'vdfs/root').path;
      const market = `${root.replace(/\/+$/, '')}/plugin_manager`;
      const entry = `${market}/${PLUGIN}`;

      const node = dataOf(await cli.invoke('vdfs/stat', { path: market }), 'market stat');
      assertEq(node.new_type?.ext, 'market', '根节点应声明市场渲染器');
      const schema = node.new_type?.schema ?? {};
      assert(Array.isArray(schema.candidates), '候选随 schema 下发，不是第二份清单');
      assertEq(schema.provider_field, 'provider', '安装载荷键名由后端定义');
      assert(
        schema.candidates.some((c) => c.name === PLUGIN && c.source === 'builtin'),
        `模拟市场候选应含 ${PLUGIN}（实际: ${JSON.stringify(schema.candidates)}）`,
      );

      const before = names(dataOf(await cli.invoke('vdfs/list', { path: market }), 'list before'));
      assert(!before.includes(PLUGIN), `安装前不应已有 ${PLUGIN}（实际: ${before}）`);

      const installed = dataOf(
        await cli.invoke('vdfs/write', {
          path: market,
          create: true,
          text: JSON.stringify({ provider: PLUGIN }),
        }),
        'market install',
      );
      assert(installed.name === PLUGIN || installed.provider === PLUGIN, '安装回执应指名插件');
      const afterInstall = names(dataOf(await cli.invoke('vdfs/list', { path: market }), 'list after install'));
      assert(afterInstall.includes(PLUGIN), `安装后列表应出现 ${PLUGIN}（实际: ${afterInstall}）`);

      const enabled = stateOf(dataOf(await cli.invoke('vdfs/read', { path: entry }), 'read enabled'));
      assertEq(enabled.plugin_enabled, true, '安装后插件应处于启用态');

      const disabled = dataOf(
        await cli.invoke('vdfs/action', { path: entry, action: 'disable', payload: {} }),
        'disable',
      );
      assert(disabled !== null, '停用应受理');
      const afterDisable = dataOf(await cli.invoke('vdfs/list', { path: market }), 'list after disable');
      const disabledItem = (afterDisable.items ?? []).find((i) => i.path?.endsWith(`/${PLUGIN}`));
      assert(disabledItem, '停用后条目仍可见');
      assert(disabledItem.meta_tags?.includes('已停用'), '列表应如实显示已停用');
      assertEq(
        stateOf(dataOf(await cli.invoke('vdfs/read', { path: entry }), 'read disabled')).plugin_enabled,
        false,
        '停用后读回装配态应为 false',
      );

      const reenabled = dataOf(
        await cli.invoke('vdfs/action', { path: entry, action: 'enable', payload: {} }),
        'enable',
      );
      assert(reenabled !== null, '启用应受理');
      const afterEnable = dataOf(await cli.invoke('vdfs/list', { path: market }), 'list after enable');
      const enabledItem = (afterEnable.items ?? []).find((i) => i.path?.endsWith(`/${PLUGIN}`));
      assert(!enabledItem?.meta_tags?.includes('已停用'), '启用后列表不应仍标已停用');
      assertEq(
        stateOf(dataOf(await cli.invoke('vdfs/read', { path: entry }), 'read enabled again')).plugin_enabled,
        true,
        '启用后读回装配态应为 true',
      );

      dataOf(await cli.invoke('vdfs/delete', { path: entry, recursive: false }), 'uninstall');
      const afterUninstall = names(dataOf(await cli.invoke('vdfs/list', { path: market }), 'list after uninstall'));
      assert(!afterUninstall.includes(PLUGIN), `卸载后列表不应再有 ${PLUGIN}（实际: ${afterUninstall}）`);

      const again = dataOf(await cli.invoke('vdfs/stat', { path: market }), 'market stat after uninstall');
      assert(
        (again.new_type?.schema?.candidates ?? []).some((c) => c.name === PLUGIN),
        '卸载后插件应重新出现在市场候选',
      );
    } finally {
      cli.stop();
      cleanupHomedir(hd);
    }
  },
);

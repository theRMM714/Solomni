/// 脚本化验收：与宿主共用同一个 assemble()，全链路断言后退出。
/// 自举假 LLM 服务器 + 临时 userdata（不碰真实密钥/历史），带环境重启自身。
/// 用法（在 apps/solomni 下）：
///   dart tool/smoke.dart                       全链路
///   dart tool/smoke.dart --without=llm_gateway 卸载降级链路
library;

import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';
import 'package:solomni/assembler.dart';

const fakePort = 9123;
var failures = 0;
var checks = 0;

void check(String name, bool ok) {
  checks++;
  print((ok ? '[ok] ' : '[FAIL] ') + name);
  if (!ok) failures++;
}

/// 验收驱动：也是普通模块，只消费 ui.render/ui.command/chatSend/chatHistory/secretsList
final class _Driver implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration('driver', needs: [
        Need('ui.render', NeedVia.preferShared),
        Need('ui.command', NeedVia.preferShared),
        Need(Caps.chatSend, NeedVia.preferShared),
        Need(Caps.chatHistory, NeedVia.preferShared),
        Need(Caps.secretsList, NeedVia.preferShared),
      ]);

  @override
  ModuleHandler bind(Outbound outbound) =>
      (env) async => throw UnsupportedError('driver 不提供服务');
}

Future<void> main(List<String> args) async {
  final ourLlm = 'http://127.0.0.1:' + fakePort.toString() + '/v1';
  // 临时 userdata：验收不碰真实密钥/历史；由子进程读回并收尾删除
  final tmp = Directory.systemTemp.createTempSync('solomni_smoke_').path;
  if (Platform.environment['LLM_BASE_URL'] != ourLlm) {
    // 自举：子进程带确定性的 LLM_BASE_URL 与临时 userdata 重跑自身
    final child = await Process.start(Platform.resolvedExecutable,
        [Platform.script.toFilePath(), ...args],
        environment: {
          ...Platform.environment,
          'LLM_BASE_URL': ourLlm,
          'SOLOMNI_USERDATA': tmp,
        },
        mode: ProcessStartMode.inheritStdio);
    exit(await child.exitCode);
  }

  final excluded = args
      .where((a) => a.startsWith('--without='))
      .map((a) => a.substring(10))
      .toSet();
  final noLlm = excluded.contains('llm_gateway');

  // 假 LLM 服务器：llm_gateway 自己的开发工具（纯 dart:io，无 package 依赖）
  final gwBin = defaultModulesDir() + '/llm_gateway/bin';
  final server = await Process.start(Platform.resolvedExecutable,
      ['fake_server.dart', '--port=' + fakePort.toString()],
      workingDirectory: gwBin);
  final banner = await server.stdout
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .first;
  if (!banner.contains('fake openai server')) {
    print('[FAIL] 假服务器未启动: ' + banner);
    exit(1);
  }

  final asm = await assemble(excluded: excluded);
  await asm.waitServices();
  final procs = <Process>[...asm.services];

  // 拉起 ui_cli 无头守护（提供 ui.render / ui.command 的 service 形态，供验收驱动）
  final uiCli = asm.folders.where((f) => f.id == 'ui_cli').firstOrNull;
  if (uiCli != null) {
    procs.add(await spawnService(uiCli, asm.daemon.port));
    await asm.waitDeclaration('ui_cli');
  }

  final driver =
      await ModuleClient.connect(asm.daemon.address, asm.daemon.port, _Driver());

  // 1. 交互面：调色板与命令来自模块贡献
  final palette = (await driver.rpc('ui.render', null)).toString();
  check('ui.render 调色板', palette.contains('调色板') && palette.contains('send'));

  if (noLlm) {
    // 2a. 卸载链路：conversation 回退内建直连（降级而非崩溃）
    final r = (await driver
        .rpc('ui.command', {'name': 'send', 'args': ['你好']})).toString();
    check('卸载 llm_gateway 回退内建', r.contains('内建直连'));
  } else {
    // 2b. 无密钥：聊天链路自决拒绝（宿主不参与）
    Object? err;
    try {
      await driver.rpc('ui.command', {'name': 'send', 'args': ['你好']});
    } catch (e) {
      err = e;
    }
    check('无密钥拒绝聊天（模块自决）', err != null && err.toString().contains('密钥'));

    // 3. 配钥后可聊
    check(
        'set-key 写入',
        await driver.rpc('ui.command', {
          'name': 'set-key',
          'args': ['llm', 'test-key-0123456789']
        }) ==
            'ok');
    final r = (await driver
        .rpc('ui.command', {'name': 'send', 'args': ['你好']})).toString();
    check('配钥后经 ui.command 聊天', r.contains('假AI回复') && r.contains('你好'));

    // 4. 单参数命令多词合并（终端词法归 UI 模块）
    final r2 = (await driver
        .rpc('ui.command', {'name': 'send', 'args': ['你', '好', '世界']})).toString();
    check('单参数命令多词合并', r2.contains('「你 好 世界」'));

    // 5. 流式：token 逐块到达
    var chunks = 0;
    final buf = StringBuffer();
    await for (final t in driver.rpcStream(Caps.chatSend, {'text': '讲个故事'})) {
      chunks++;
      buf.write(t);
    }
    check('流式 token 多块到达', chunks > 1 && buf.toString().contains('假AI回复'));
  }

  // 6. 多轮历史：归属 conversation 模块
  final h1 = (await driver.rpc(Caps.chatHistory, null)) as List;
  await driver.rpc('ui.command', {'name': 'send', 'args': ['再聊一句']});
  final h2 = (await driver.rpc(Caps.chatHistory, null)) as List;
  check('多轮历史增长', h2.length > h1.length);

  // 7. 钥匙圈列举：只报名字/备注，绝不出钥匙值（全链路才配过钥）
  if (!noLlm) {
    final listed = (await driver.rpc(Caps.secretsList, null)) as List;
    final s = listed.toString();
    check(
        '钥匙圈只报名字备注不出值',
        listed.any((e) => (e as Map)['name'] == 'llm') &&
            !s.contains('test-key'));
  }

  // 收尾：杀服务进程与假服务器，清临时 userdata
  for (final p in procs) {
    await killProcess(p);
  }
  server.kill();
  try {
    Directory(tmp).deleteSync(recursive: true);
  } catch (_) {}

  print(failures == 0
      ? '全部通过: ' + checks.toString() + ' 项'
      : '失败 ' + failures.toString() + ' / ' + checks.toString());
  exit(failures == 0 ? 0 : 1);
}

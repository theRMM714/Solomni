/// 脚本化验收：与产品入口共用同一个 assemble()，全链路断言后退出。
/// 自举假 LLM 服务器（装配在进程内读环境变量，故带环境重启自身）。
/// 用法（在 apps/solomni 下）：
///   dart tool/smoke.dart                       全链路
///   dart tool/smoke.dart --without=llm_gateway 卸载降级链路
library;

import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'package:protocol/protocol.dart';
import 'package:solomni/assembler.dart';

const fakePort = 9123;
var failures = 0;
var checks = 0;

void check(String name, bool ok) {
  checks++;
  print((ok ? '[ok] ' : '[FAIL] ') + name);
  if (!ok) failures++;
}

Future<void> main(List<String> args) async {
  final ourLlm = 'http://127.0.0.1:' + fakePort.toString() + '/v1';
  if (Platform.environment['LLM_BASE_URL'] != ourLlm) {
    // 自举：子进程带确定性的 LLM_BASE_URL 重跑自身
    final child = await Process.start(Platform.resolvedExecutable,
        [Platform.script.toFilePath(), ...args],
        environment: {'LLM_BASE_URL': ourLlm},
        mode: ProcessStartMode.inheritStdio);
    exit(await child.exitCode);
  }

  final excluded = args
      .where((a) => a.startsWith('--without='))
      .map((a) => a.substring(10))
      .toSet();
  final noLlm = excluded.contains('llm_gateway');

  // 假 LLM 服务器：llm_gateway 自己的开发工具
  final gw = Isolate.resolvePackageUriSync(
      Uri.parse('package:llm_gateway/llm_gateway.dart'))!;
  final server = await Process.start(Platform.resolvedExecutable,
      ['fake_server.dart', '--port=' + fakePort.toString()],
      workingDirectory: gw.resolve('../bin').toFilePath());
  final banner = await server.stdout
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .first;
  if (!banner.contains('fake openai server')) {
    print('[FAIL] 假服务器未启动: ' + banner);
    exit(1);
  }

  final asm = await assemble(excluded: excluded);
  final driver = asm.driver;

  // 1. 交互面：调色板与命令来自模块贡献
  final palette = (await driver.rpc('ui.render', null)).toString();
  check('ui.render 调色板', palette.contains('调色板') && palette.contains('send'));

  if (noLlm) {
    // 2a. 卸载链路：conversation 回退内建直连（降级而非崩溃）
    final r = (await driver
        .rpc('ui.command', {'name': 'send', 'args': ['你好']})).toString();
    check('卸载 llm_gateway 回退内建', r.contains('内建直连'));
  } else {
    // 2b. 无密钥：聊天链路自决拒绝（产品入口不参与）
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

  server.kill();
  print(failures == 0
      ? '全部通过: ' + checks.toString() + ' 项'
      : '失败 ' + failures.toString() + ' / ' + checks.toString());
  exit(failures == 0 ? 0 : 1);
}

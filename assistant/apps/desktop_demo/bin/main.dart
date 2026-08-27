/// 桌面产品（demo）：目录发现模块 -> TCP 拓扑 -> 撮合 -> 经 ui_cli 模块交互。
/// 装配者（本文件）是唯一的策略所在：发现谁、排除谁。
/// --without=llm_gateway 演示卸载：conversation 自动回退内建。
library;

import 'dart:async';
import 'dart:io';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';
import 'package:conversation/conversation.dart';
import 'package:llm_gateway/llm_gateway.dart';
import 'package:secrets/secrets.dart';
import 'package:ui_cli/ui_cli.dart';

/// 驱动程序：也是个普通模块，只消费 ui_cli 的能力
final class DriverProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration('driver', needs: [
        Need('ui.render', NeedVia.preferShared),
        Need('ui.command', NeedVia.preferShared),
        Need(Caps.chatSend, NeedVia.preferShared),
      ]);

  @override
  ModuleHandler bind(Outbound outbound) =>
      (env) => throw UnsupportedError('driver 不提供服务');
}

Future<void> main(List<String> args) async {
  final excluded = args
      .where((a) => a.startsWith('--without='))
      .map((a) => a.substring(10))
      .toSet();
  final serve = args.contains('--serve');
  var servePort = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) servePort = int.parse(a.substring(7));
  }

  // 1. 目录发现：一个文件夹 = 一个可装卸单元
  final modulesDir = Platform.script.resolve('../../modules').toFilePath();
  final folders = scanModules(modulesDir);
  print('[发现] ' + folders.map((f) => f.id).join(', ') + '   <- 扫描 ' + modulesDir);

  // 2. 产品内编译进来的模块工厂（进程内组合是编译期引用，Dart 硬约束）
  final registry = <String, ModuleProgram Function()>{
    'conversation': ConversationProgram.new,
    'llm_gateway': LlmGatewayProgram.new,
    'secrets': SecretsProgram.new,
    'ui_cli': UiCliProgram.new,
  };

  // 3. 起核心 + 逐模块连接（hello 声明经协议到达，核心不读任何文件）
  //    --serve 模式固定端口常驻，供外部模块进程（如 Flutter UI）接入
  final daemon = await BrokerDaemon.bind(
      InternetAddress.loopbackIPv4, serve ? servePort : 0);
  for (final f in folders) {
    if (excluded.contains(f.id)) {
      print('[卸载] ' + f.id + ' 被排除（文件夹保留，可随时恢复）');
      continue;
    }
    final make = registry[f.id];
    if (make == null) {
      print('[跳过] ' + f.id + ' 未编译进本产品');
      continue;
    }
    await ModuleClient.connect(daemon.address, daemon.port, make());
  }

  // 4. 撮合：fail-fast
  await daemon.start();

  if (serve) {
    print('[serve] 核心常驻 :' +
        daemon.port.toString() +
        '（Ctrl+C 退出；UI 进程接入：flutter run -t bin/coordinated.dart -- --port=' +
        daemon.port.toString() +
        '）');
    await Completer<void>().future; // 常驻
  }

  final driver =
      await ModuleClient.connect(daemon.address, daemon.port, DriverProgram());

  // 5. UI 模块看一眼 -> 调色板 + 画布；然后经命令路由事件
  final palette = await driver.rpc('ui.render', null);
  print(palette as String);
  print('[命令] send 你好');
  final reply = await driver.rpc('ui.command', {
        'name': 'send',
        'args': ['你好'],
      });
  print('[回复] ' + (reply as String));

  print('[流式] send 讲个故事（token 逐个到达）');
  final tokens = <String>[];
  await for (final t in driver.rpcStream(Caps.chatSend, {'text': '讲个故事'})) {
    stdout.write(t);
    tokens.add(t as String);
  }
  print('');
  print('[流式] 共收到 ' + tokens.length.toString() + ' 块');

  print('[命令] set-key llm user-key-0123456789abcdef');
  final setResult = await driver.rpc('ui.command', {
        'name': 'set-key',
        'args': ['llm', 'user-key-0123456789abcdef'],
      });
  print('[结果] ' + (setResult as String));
  final reply2 = await driver.rpc('ui.command', {
        'name': 'send',
        'args': ['再见'],
      });
  print('[回复] ' + (reply2 as String));
  exit(0);
}

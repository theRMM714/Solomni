/// 发现链路复现：模拟 GUI 的真实调用序列（无需求 surface -> core.modules -> 定向拉贡献）。
/// 用法：宿主 --serve 后，dart tool/discover_sim.dart --port=<宿主端口>
library;

import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';

final class _Sim implements ModuleProgram {
  Outbound? outbound; // 与 GUI 相同：bind 时捕获出边（定向 call 在这里）
  @override
  Declaration get declaration => const Declaration(
        'guisim',
        kind: ModuleKind.surface,
        needs: [Need(HostCaps.logsAppend, NeedVia.preferShared)],
      );

  @override
  ModuleHandler bind(Outbound outbound) {
    this.outbound = outbound;
    return (env) async => throw UnsupportedError('sim 不提供服务');
  }
}

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  final program = _Sim();
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, program);
  final out = program.outbound!;
  print('[sim] 已连接');
  // logs.append：有 logs 能力则写宿主日志（preferShared 消费）
  try {
    await out.rpc(HostCaps.logsAppend,
        {'from': 'guisim', 'text': 'sim 验证写入'});
    print('[sim] logs.append 写入 ok');
  } catch (e) {
    print('[sim] logs.append 降级（无提供方）: ' + e.toString());
  }
  try {
    final res = await out.rpc(CoreCaps.modules, null);
    final mods = (res as Map)['modules'] as List;
    print('[sim] modules: ' + mods.map((m) => (m as Map)['id']).join(', '));
    for (final raw in mods) {
      final decl =
          Declaration.fromJson(Map<String, Object?>.from(raw as Map));
      if (decl.provides.any((p) => p.cap == UiVocab.contribution)) {
        try {
          final cfg = await out.call(decl.id, 'ui.contribution', null);
          print('[sim] ' +
              decl.id +
              ' contribution ok（' +
              cfg.toString().length.toString() +
              ' chars）');
        } catch (e) {
          print('[sim] ' + decl.id + ' contribution 失败: ' + e.toString());
        }
      }
    }
  } catch (e) {
    print('[sim] 发现失败: ' + e.toString());
  }
  exit(0);
}

abstract final class UiVocab {
  static const contribution = 'ui.contribution';
}

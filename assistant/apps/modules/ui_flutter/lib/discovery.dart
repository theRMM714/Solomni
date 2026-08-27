/// 贡献发现：核心注册表只读列举 -> 定向拉取各模块 ui.contribution。
/// 与 ui_cli 相同的机制，共享词汇 ui_vocab/ui_canvas。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';

Future<List<Contributed>> discover(Outbound out) async {
  final res = (await out.rpc(CoreCaps.modules, null)) as Map;
  final found = <Contributed>[];
  for (final raw in res['modules'] as List) {
    final decl = Declaration.fromJson(Map<String, Object?>.from(raw as Map));
    if (decl.provides.any((p) => p.cap == UiCaps.contribution)) {
      final cfg = await out.call(decl.id, UiCaps.contribution, null);
      found.add(Contributed(decl.id,
          UiContribution.fromJson(Map<String, Object?>.from(cfg as Map))));
    }
  }
  return found;
}

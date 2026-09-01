/// ui_flutter 模块程序：UI 也是普通模块（渲染器）。
/// 命令执行消费 ui.command（由 ui_cli 等模块提供，无提供方则降级禁用）。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

final class UiFlutterProgram implements ModuleProgram {
  /// bind 时捕获出边，供 widget 树使用
  Outbound? outbound;

  @override
  Declaration get declaration => const Declaration('ui_flutter', needs: [
        Need('ui.command', NeedVia.preferShared),
      ]);

  @override
  ModuleHandler bind(Outbound outbound) {
    this.outbound = outbound;
    return (env) async =>
        throw UnsupportedError('ui_flutter 不提供服务: ' + env.method);
  }
}

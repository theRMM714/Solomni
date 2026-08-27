/// ui_flutter 模块程序：UI 也是普通模块（渲染器）。
/// 声明为空：本模块不提供共同物，只消费贡献与路由事件。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

final class UiFlutterProgram implements ModuleProgram {
  /// bind 时捕获出边，供 widget 树使用
  Outbound? outbound;

  @override
  Declaration get declaration => const Declaration('ui_flutter');

  @override
  ModuleHandler bind(Outbound outbound) {
    this.outbound = outbound;
    return (env) async =>
        throw UnsupportedError('ui_flutter 不提供服务: ' + env.method);
  }
}

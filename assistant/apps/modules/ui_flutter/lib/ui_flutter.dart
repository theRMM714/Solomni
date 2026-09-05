/// ui_flutter 模块程序：UI 也是普通模块（GUI 渲染器，交互面）。
/// 只渲染各模块 ui.contribution 并把手势经 ui.event 定向路由回声明模块；
/// 命令执行不在此处（命令是文本交互面的词汇，GUI 纯图形）。
/// logs 用 preferShared：宿主的 logs.append 在则写日志，不在则仅控制台。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

final class UiFlutterProgram implements ModuleProgram {
  /// bind 时捕获出边，供 widget 树使用
  Outbound? outbound;

  @override
  Declaration get declaration => const Declaration(
        'ui_flutter',
        kind: ModuleKind.surface,
        needs: [Need(HostCaps.logsAppend, NeedVia.preferShared)],
      );

  @override
  ModuleHandler bind(Outbound outbound) {
    this.outbound = outbound;
    return (env) async =>
        throw UnsupportedError('ui_flutter 不提供服务: ' + env.method);
  }
}

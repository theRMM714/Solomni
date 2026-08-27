/// ui_cli 模块：UI 也是普通模块。
/// 流程：core.modules 看一眼 -> 定向拉取各模块 ui.contribution ->
/// 渲染列表与画布（布局数据是本模块私有数据）-> ui.event 定向路由回声明模块。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_vocab/ui_vocab.dart';

/// 一个模块的 UI 贡献与它的来源 id
class Contributed {
  final String moduleId;
  final UiContribution contribution;
  const Contributed(this.moduleId, this.contribution);
}

final class UiCliProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration(
        'ui_cli',
        provides: [Provide('ui.render'), Provide('ui.command')],
      );

  @override
  ModuleHandler bind(Outbound outbound) {
    return (env) async {
      if (env.method == 'ui.render') {
        return render(outbound);
      }
      if (env.method == 'ui.command') {
        return command(outbound, env.params as Map);
      }
      throw UnsupportedError('ui_cli 不认识 ' + env.method);
    };
  }

  /// 发现：核心注册表只读列举（事实，非决策），过滤出有贡献的模块
  Future<List<Contributed>> discover(Outbound out) async {
    final res = (await out.rpc(CoreCaps.modules, null)) as Map;
    final found = <Contributed>[];
    for (final raw in res['modules'] as List) {
      final decl = Declaration.fromJson(Map<String, Object?>.from(raw as Map));
      if (decl.provides.any((p) => p.cap == UiCaps.contribution)) {
        final cfg = await out.call(
            decl.id, UiCaps.contribution, null);
        found.add(Contributed(
            decl.id, UiContribution.fromJson(Map<String, Object?>.from(cfg as Map))));
      }
    }
    return found;
  }

  /// 渲染：调色板（列表）+ 画布（布局是本模块私有数据，模块不知道自己被摆在哪）
  Future<String> render(Outbound out) async {
    final found = await discover(out);
    final buf = StringBuffer();
    buf.writeln('[调色板]');
    final publics = <String, List<String>>{};
    for (final c in found) {
      for (final comp in c.contribution.components) {
        buf.writeln('  ' +
            (comp.scope == UiScope.public ? '*' : ' ') +
            ' ' +
            c.moduleId +
            '.' +
            comp.id +
            ' (' +
            comp.kind +
            ') "' +
            comp.label +
            '"');
        (publics.putIfAbsent(c.moduleId, () => []))
            .add(comp.id + (comp.scope == UiScope.public ? '' : ' [私有]'));
      }
      for (final cmd in c.contribution.commands) {
        buf.writeln('    命令: ' +
            cmd.name +
            ' ' +
            cmd.args.join(' ') +
            '  -> ' +
            cmd.component +
            '.' +
            cmd.event);
      }
    }
    buf.writeln('[画布]（布局归 UI 模块，可拖动/改样式，组件 id 不可改）');
    for (final e in publics.entries) {
      buf.writeln('  == ' + e.key + (e.key == 'conversation' ? ' 默认画布' : ' 私有画布') + ' ==');
      buf.writeln('  ' + e.value.join('  |  '));
    }
    return buf.toString();
  }

  /// 命令：找到声明它的模块，事件定向路由回去
  Future<String> command(Outbound out, Map params) async {
    final name = params['name'] as String;
    final args = [for (final a in (params['args'] as List? ?? const [])) a as String];
    final found = await discover(out);
    for (final c in found) {
      for (final cmd in c.contribution.commands) {
        if (cmd.name != name) continue;
        final payload = <String, Object?>{
          for (var i = 0; i < cmd.args.length && i < args.length; i++)
            cmd.args[i]: args[i],
        };
        final reply = await out.call(c.moduleId, UiCaps.event, {
          'component': cmd.component,
          'event': cmd.event,
          'payload': payload,
        });
        return reply as String;
      }
    }
    throw UnsupportedError('未声明的命令: ' + name);
  }
}

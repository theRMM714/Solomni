/// 左面板：模块列表。命令可点击执行（经 ui.command 路由）；
/// 有私有控件的模块提供[创建画布]入口（每模块一张）。
/// 组件不在此列出——组件的入口在画布"+"。
library;

import 'package:flutter/material.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';

class PaletteView extends StatelessWidget {
  final List<Contributed> found;
  final LayoutDoc layout; // 判断模块画布是否已建
  final Outbound? outbound; // 命令执行经 ui.command（null = standalone 禁用）
  final ValueChanged<String> onCreateModuleCanvas;
  const PaletteView({
    super.key,
    required this.found,
    required this.layout,
    required this.outbound,
    required this.onCreateModuleCanvas,
  });

  bool _canvasExists(String moduleId) =>
      layout.canvases.any((c) => c.ownerModuleId == moduleId);

  @override
  Widget build(BuildContext context) {
    if (found.isEmpty) {
      return const Center(child: Text('无模块声明 UI'));
    }
    return ListView(children: [
      const Padding(
        padding: EdgeInsets.all(12),
        child: Text('模块（命令可点击执行）',
            style: TextStyle(fontWeight: FontWeight.bold)),
      ),
      for (final c in found) ...[
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 10, 12, 0),
          child: Row(children: [
            Expanded(
              child: Text(c.moduleId,
                  style: TextStyle(
                      color: Theme.of(context).colorScheme.primary,
                      fontSize: 12)),
            ),
            if (c.contribution.components
                .any((k) => k.scope == UiScope.private))
              if (!_canvasExists(c.moduleId))
                TextButton.icon(
                  icon: const Icon(Icons.dashboard_customize, size: 14),
                  label: const Text('创建画布', style: TextStyle(fontSize: 11)),
                  onPressed: () => onCreateModuleCanvas(c.moduleId),
                ),
          ]),
        ),
        for (final cmd in c.contribution.commands)
          ListTile(
            dense: true,
            leading: const Icon(Icons.play_arrow, size: 18),
            title: Text(cmd.name),
            subtitle: Text(cmd.args.join(' '),
                style: const TextStyle(fontSize: 11)),
            enabled: outbound != null,
            onTap: outbound == null
                ? null
                : () => _runCommand(context, cmd),
          ),
      ],
    ]);
  }

  /// 命令执行：按声明的参数名逐个弹输入 -> 经 ui.command 路由（与终端 REPL 同一条路）
  Future<void> _runCommand(BuildContext context, UiCommand cmd) async {
    final out = outbound!;
    final args = <String>[];
    for (final argName in cmd.args) {
      final v = await _promptArg(context, cmd.name, argName);
      if (v == null) return; // 取消
      args.add(v);
    }
    try {
      final r = await out.rpc('ui.command', {'name': cmd.name, 'args': args});
      if (context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text(cmd.name + ' -> ' + (r ?? '').toString())));
      }
    } catch (e) {
      if (context.mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(e.toString())));
      }
    }
  }

  Future<String?> _promptArg(
      BuildContext context, String cmdName, String argName) async {
    final ctl = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(cmdName + ' · ' + argName),
        content: TextField(controller: ctl, autofocus: true),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: const Text('取消')),
          FilledButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: const Text('确定')),
        ],
      ),
    );
    return (ok ?? false) ? ctl.text.trim() : null;
  }
}

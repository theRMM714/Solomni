/// 新建画布弹窗：二选一——模块画布（仅含私有组件的模块，每模块一张）
/// 或自定义画布。弹窗只做选择，创建动作经回调归 shell。
library;

import 'package:flutter/material.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';

/// 模块画布候选规则：有私有组件的模块才需要专属画布（public 组件任意画布可放）
bool _hasPrivate(UiContribution c) =>
    c.components.any((k) => k.scope == UiScope.private);

Future<void> showNewCanvasDialog(
  BuildContext context, {
  required List<Contributed> found,
  required LayoutDoc layout,
  required ValueChanged<String> onModuleCanvas,
  required void Function(String title) onCustomCanvas,
}) {
  return showDialog<void>(
    context: context,
    builder: (ctx) => _NewCanvasDialog(
      found: found,
      layout: layout,
      onModuleCanvas: onModuleCanvas,
      onCustomCanvas: onCustomCanvas,
    ),
  );
}

/// Stateful：控制器随 Widget 生命周期释放（退出动画完成才 dispose）
class _NewCanvasDialog extends StatefulWidget {
  final List<Contributed> found;
  final LayoutDoc layout;
  final ValueChanged<String> onModuleCanvas;
  final void Function(String title) onCustomCanvas;

  const _NewCanvasDialog({
    required this.found,
    required this.layout,
    required this.onModuleCanvas,
    required this.onCustomCanvas,
  });

  @override
  State<_NewCanvasDialog> createState() => _NewCanvasDialogState();
}

class _NewCanvasDialogState extends State<_NewCanvasDialog> {
  final _ctl = TextEditingController();

  @override
  void dispose() {
    _ctl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final candidates =
        widget.found.where((c) => _hasPrivate(c.contribution)).toList();
    return AlertDialog(
      title: const Text('新建画布'),
      content: SizedBox(
        width: 380,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Padding(
                padding: EdgeInsets.only(bottom: 4),
                child: Text('模块画布',
                    style: TextStyle(fontWeight: FontWeight.bold)),
              ),
              for (final c in candidates)
                _moduleTile(c),
              if (candidates.isEmpty)
                const Padding(
                  padding: EdgeInsets.all(12),
                  child: Text('暂无含私有组件的模块',
                      style: TextStyle(fontSize: 12)),
                ),
              const Divider(),
              const Padding(
                padding: EdgeInsets.only(bottom: 4),
                child: Text('自定义画布',
                    style: TextStyle(fontWeight: FontWeight.bold)),
              ),
              Row(children: [
                Expanded(
                  child: TextField(
                    controller: _ctl,
                    autofocus: true,
                    decoration: const InputDecoration(
                        isDense: true, hintText: '画布标题'),
                    onSubmitted: _createCustom,
                  ),
                ),
                const SizedBox(width: 8),
                FilledButton(
                  onPressed: () => _createCustom(_ctl.text),
                  child: const Text('创建'),
                ),
              ]),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('取消'),
        ),
      ],
    );
  }

  /// 模块候选条目：已建画布的禁用（每模块一张），私有组件标签做副标题
  Widget _moduleTile(Contributed c) {
    final created =
        widget.layout.canvases.any((cv) => cv.ownerModuleId == c.moduleId);
    final privates = [
      for (final k in c.contribution.components)
        if (k.scope == UiScope.private) (k.label.isEmpty ? k.id : k.label)
    ];
    return ListTile(
      dense: true,
      leading: const Icon(Icons.lock_outline, size: 18),
      title: Text(c.moduleId),
      subtitle: Text(privates.join('、'),
          style: const TextStyle(fontSize: 11)),
      enabled: !created,
      trailing: created
          ? const Text('已创建', style: TextStyle(fontSize: 11))
          : null,
      onTap: () {
        Navigator.pop(context);
        widget.onModuleCanvas(c.moduleId);
      },
    );
  }

  void _createCustom(String text) {
    final t = text.trim();
    if (t.isEmpty) return;
    Navigator.pop(context);
    widget.onCustomCanvas(t);
  }
}

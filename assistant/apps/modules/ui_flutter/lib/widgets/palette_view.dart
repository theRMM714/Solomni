/// 调色板（列表）：各模块声明的组件。想用哪个组件就拖进画布。
library;

import 'package:flutter/material.dart';
import 'package:ui_canvas/ui_canvas.dart';

class PaletteView extends StatelessWidget {
  final List<Contributed> found;
  const PaletteView({super.key, required this.found});

  @override
  Widget build(BuildContext context) {
    if (found.isEmpty) {
      return const Center(child: Text('调色板为空（无模块声明 UI）'));
    }
    return ListView(children: [
      const Padding(
        padding: EdgeInsets.all(12),
        child: Text('调色板（拖入画布）',
            style: TextStyle(fontWeight: FontWeight.bold)),
      ),
      for (final c in found) ...[
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
          child: Text(c.moduleId,
              style: TextStyle(
                  color: Theme.of(context).colorScheme.primary,
                  fontSize: 12)),
        ),
        for (final comp in c.contribution.components)
          _draggable(PaletteEntry(c.moduleId, comp)),
        for (final cmd in c.contribution.commands)
          ListTile(
            dense: true,
            leading: const Icon(Icons.terminal, size: 18),
            title: Text(cmd.name),
            subtitle: Text(cmd.args.join(' '), style: const TextStyle(fontSize: 11)),
          ),
      ],
    ]);
  }

  Widget _draggable(PaletteEntry e) {
    return Draggable<PaletteEntry>(
      data: e,
      feedback: Material(
        color: Colors.blueGrey,
        borderRadius: BorderRadius.circular(6),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Text(e.component.id,
              style: const TextStyle(color: Colors.white)),
        ),
      ),
      childWhenDragging: Opacity(opacity: 0.4, child: _tile(e)),
      child: _tile(e),
    );
  }

  Widget _tile(PaletteEntry e) => ListTile(
        dense: true,
        leading: Icon(
            e.isPrivate ? Icons.lock_outline : Icons.widgets_outlined,
            size: 20),
        title: Text(e.component.id),
        subtitle: Text(
          e.component.label +
              ' · ' +
              e.component.kind +
              (e.isPrivate ? ' · 私有' : ' · 公用'),
          style: const TextStyle(fontSize: 11),
        ),
      );
}

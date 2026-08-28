/// 画布区：tab 切换画布；画布接收拖放、支持拖动位置与缩放。
/// 组件 id 不可改（契约）；位置/尺寸/样式归本模块。
library;

import 'package:flutter/material.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';
import 'component_views.dart';

class CanvasArea extends StatelessWidget {
  final LayoutEngine engine;
  final LayoutDoc layout;
  final List<Contributed> found;
  final int tab;
  final int tick;
  final Outbound? outbound;
  final ValueChanged<int> onSelectTab;
  final void Function(String title) onNewCanvas;
  final VoidCallback onChanged;

  const CanvasArea({
    super.key,
    required this.engine,
    required this.layout,
    required this.found,
    required this.tab,
    required this.tick,
    required this.outbound,
    required this.onSelectTab,
    required this.onNewCanvas,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    if (layout.canvases.isEmpty) {
      return const Center(child: Text('无画布'));
    }
    final current = layout.canvases[tab.clamp(0, layout.canvases.length - 1)];
    return Column(children: [
      SizedBox(
        height: 44,
        child: ListView(scrollDirection: Axis.horizontal, children: [
          for (var i = 0; i < layout.canvases.length; i++)
            Padding(
              padding: const EdgeInsets.all(6),
              child: ChoiceChip(
                label: Text(layout.canvases[i].title),
                selected: i == tab,
                onSelected: (_) => onSelectTab(i),
              ),
            ),
          Padding(
            padding: const EdgeInsets.all(6),
            child: ActionChip(
              avatar: const Icon(Icons.add, size: 16),
              label: const Text('新建画布'),
              onPressed: () => _promptNew(context),
            ),
          ),
        ]),
      ),
      const Divider(height: 1),
      Expanded(
        child: CanvasView(
          engine: engine,
          canvas: current,
          found: found,
          tick: tick,
          outbound: outbound,
          onChanged: onChanged,
        ),
      ),
    ]);
  }

  Future<void> _promptNew(BuildContext context) async {
    final ctl = TextEditingController();
    await showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('新建画布'),
        content: TextField(controller: ctl, autofocus: true),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () {
              if (ctl.text.trim().isNotEmpty) onNewCanvas(ctl.text.trim());
              Navigator.pop(ctx);
            },
            child: const Text('创建'),
          ),
        ],
      ),
    );
  }
}

class CanvasView extends StatefulWidget {
  final LayoutEngine engine;
  final CanvasDoc canvas;
  final List<Contributed> found;
  final int tick;
  final Outbound? outbound;
  final VoidCallback onChanged;

  const CanvasView({
    super.key,
    required this.engine,
    required this.canvas,
    required this.found,
    required this.tick,
    required this.outbound,
    required this.onChanged,
  });

  @override
  State<CanvasView> createState() => _CanvasViewState();
}

class _CanvasViewState extends State<CanvasView> {
  final _stackKey = GlobalKey();

  UiComponent? _componentOf(String moduleId, String componentId) {
    for (final c in widget.found) {
      if (c.moduleId != moduleId) continue;
      for (final comp in c.contribution.components) {
        if (comp.id == componentId) return comp;
      }
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final engine = widget.engine;
    final canvas = widget.canvas;
    return DragTarget<PaletteEntry>(
      onWillAcceptWithDetails: (d) =>
          engine.canPlace(canvas, d.data),
      onAcceptWithDetails: (d) {
        final box = _stackKey.currentContext?.findRenderObject() as RenderBox?;
        final local = box?.globalToLocal(d.offset);
        setState(() {
          engine.place(canvas, d.data,
              x: (local?.dx ?? 24) - 60, y: (local?.dy ?? 24) - 20);
        });
        widget.onChanged();
      },
      builder: (context, candidates, rejected) => Container(
        color: candidates.isNotEmpty
            ? Theme.of(context).colorScheme.primaryContainer
            : Theme.of(context).colorScheme.surfaceContainerLowest,
        child: Stack(key: _stackKey, children: [
          for (final p in canvas.placements)
            Positioned(
              left: p.x,
              top: p.y,
              width: p.w,
              height: p.h,
              child: _placementCard(p),
            ),
        ]),
      ),
    );
  }

  Widget _placementCard(Placement p) {
    final comp = _componentOf(p.moduleId, p.componentId);
    return GestureDetector(
      onPanUpdate: (d) =>
          setState(() => widget.engine.move(p, d.delta.dx, d.delta.dy)),
      onPanEnd: (_) => widget.onChanged(),
      child: Card(
        margin: EdgeInsets.zero,
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            decoration: BoxDecoration(
              color: Theme.of(context).colorScheme.surfaceContainerHighest,
              borderRadius:
                  const BorderRadius.vertical(top: Radius.circular(8)),
            ),
            child: Row(children: [
              Expanded(
                child: Text(
                  p.moduleId + '.' + p.componentId +
                      (comp == null ? '（未知组件）' : ''),
                  style: const TextStyle(fontSize: 11),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              GestureDetector(
                onTap: () {
                  setState(() => widget.engine.remove(widget.canvas, p));
                  widget.onChanged();
                },
                child: const Icon(Icons.close, size: 14),
              ),
              GestureDetector(
                onPanUpdate: (d) => setState(
                    () => widget.engine.resize(p, d.delta.dx, d.delta.dy)),
                onPanEnd: (_) => widget.onChanged(),
                child: const MouseRegion(
                  cursor: SystemMouseCursors.resizeDownRight,
                  child: Icon(Icons.aspect_ratio, size: 14),
                ),
              ),
            ]),
          ),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.all(6),
              child: comp == null
                  ? const Center(child: Text('未知组件（已降级渲染）'))
                  : ComponentView(
                      moduleId: p.moduleId,
                      component: comp,
                      outbound: widget.outbound,
                      tick: widget.tick,
                      onAction: widget.onChanged,
                    ),
            ),
          ),
        ]),
      ),
    );
  }
}

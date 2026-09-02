/// 画布区：顶部画布栏是主导航——点击切换、×删除（确认）、[新建画布]弹窗二选一
/// （模块画布/自定义画布）；组件经"+"选择器进入（规则过滤）；
/// 离线模块的组件冻结禁用（贡献缓存渲染），重连自动恢复。
/// 组件 id 不可改（契约）；位置/尺寸/样式归本模块。
library;

import 'package:flutter/material.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';
import 'component_views.dart';
import 'new_canvas_dialog.dart';

/// 组件查询结果：comp 为 null 且 offline=false 表示从未见过（占位降级）
class _Lookup {
  final UiComponent? comp;
  final bool offline;
  const _Lookup(this.comp, this.offline);
}

class CanvasArea extends StatelessWidget {
  final LayoutEngine engine;
  final LayoutDoc layout;
  final List<Contributed> found;
  final Map<String, UiContribution> cache; // 贡献缓存：离线冻结渲染
  final int tab;
  final int tick;
  final Outbound? outbound;
  final ValueChanged<int> onSelectTab;
  final ValueChanged<String> onModuleCanvas; // 新建模块画布（弹窗选择后）
  final void Function(String title) onCustomCanvas; // 新建自定义画布
  final ValueChanged<int> onDeleteCanvas; // 删除画布（含确认，归 shell）
  final VoidCallback onChanged;

  const CanvasArea({
    super.key,
    required this.engine,
    required this.layout,
    required this.found,
    required this.cache,
    required this.tab,
    required this.tick,
    required this.outbound,
    required this.onSelectTab,
    required this.onModuleCanvas,
    required this.onCustomCanvas,
    required this.onDeleteCanvas,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final hasCanvases = layout.canvases.isNotEmpty;
    final current = hasCanvases
        ? layout.canvases[tab.clamp(0, layout.canvases.length - 1)]
        : null;
    return Column(children: [
      SizedBox(
        height: 44,
        child: ListView(scrollDirection: Axis.horizontal, children: [
          for (var i = 0; i < layout.canvases.length; i++)
            Padding(
              padding: const EdgeInsets.all(6),
              child: InputChip(
                avatar: Icon(
                    layout.canvases[i].ownerModuleId != null
                        ? Icons.extension
                        : Icons.dashboard_outlined,
                    size: 14),
                label: Text(layout.canvases[i].title),
                selected: i == tab,
                onPressed: () => onSelectTab(i),
                onDeleted: () => onDeleteCanvas(i),
                deleteIcon: const Icon(Icons.close, size: 16),
              ),
            ),
          Padding(
            padding: const EdgeInsets.all(6),
            child: ActionChip(
              avatar: const Icon(Icons.add, size: 16),
              label: const Text('新建画布'),
              onPressed: () => _openNewDialog(context),
            ),
          ),
          if (current != null)
            Padding(
              padding: const EdgeInsets.all(6),
              child: ActionChip(
                avatar: const Icon(Icons.add_box_outlined, size: 16),
                label: const Text('添加组件'),
                onPressed: () => _pickComponent(context, current),
              ),
            ),
        ]),
      ),
      const Divider(height: 1),
      Expanded(
        child: current == null
            ? _emptyState(context)
            : CanvasView(
                engine: engine,
                canvas: current,
                found: found,
                cache: cache,
                tick: tick,
                outbound: outbound,
                onChanged: onChanged,
              ),
      ),
    ]);
  }

  /// 空状态：唯一动作就是创建第一张画布（弹窗二选一）
  Widget _emptyState(BuildContext context) => Center(
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          Icon(Icons.dashboard_outlined,
              size: 44, color: Theme.of(context).disabledColor),
          const SizedBox(height: 8),
          const Text('还没有画布'),
          const SizedBox(height: 12),
          FilledButton.icon(
            icon: const Icon(Icons.add),
            label: const Text('新建画布'),
            onPressed: () => _openNewDialog(context),
          ),
        ]),
      );

  void _openNewDialog(BuildContext context) {
    showNewCanvasDialog(context,
        found: found,
        layout: layout,
        onModuleCanvas: onModuleCanvas,
        onCustomCanvas: onCustomCanvas);
  }

  /// "+"选择器：内容 = canPlace 规则过滤后的全部组件（边界由构造保证）
  Future<void> _pickComponent(BuildContext context, CanvasDoc canvas) async {
    final entries = <PaletteEntry>[
      for (final c in found)
        for (final comp in c.contribution.components)
          if (engine.canPlace(canvas,
              PaletteEntry(c.moduleId, comp)))
            PaletteEntry(c.moduleId, comp),
    ];
    if (!context.mounted) return;
    await showModalBottomSheet<void>(
      context: context,
      builder: (ctx) => ListView(children: [
        Padding(
          padding: const EdgeInsets.all(12),
          child: Text('添加到「' + canvas.title + '」',
              style: const TextStyle(fontWeight: FontWeight.bold)),
        ),
        if (entries.isEmpty)
          const Padding(
            padding: EdgeInsets.all(16),
            child: Text('没有可放置的组件'),
          ),
        for (final e in entries)
          ListTile(
            dense: true,
            leading: Icon(
                e.isPrivate ? Icons.lock_outline : Icons.widgets_outlined,
                size: 20),
            title: Text(e.moduleId + '.' + e.component.id),
            subtitle: Text(
              e.component.label + ' · ' + e.component.kind,
              style: const TextStyle(fontSize: 11),
            ),
            onTap: () {
              Navigator.pop(ctx);
              // 选中落默认位置（级联偏移避开叠放）
              final n = canvas.placements.length;
              engine.place(canvas, e,
                  x: 24.0 + (n % 4) * 60, y: 24.0 + (n % 4) * 50);
              onChanged();
            },
          ),
      ]),
    );
  }
}

class CanvasView extends StatefulWidget {
  final LayoutEngine engine;
  final CanvasDoc canvas;
  final List<Contributed> found;
  final Map<String, UiContribution> cache;
  final int tick;
  final Outbound? outbound;
  final VoidCallback onChanged;

  const CanvasView({
    super.key,
    required this.engine,
    required this.canvas,
    required this.found,
    required this.cache,
    required this.tick,
    required this.outbound,
    required this.onChanged,
  });

  @override
  State<CanvasView> createState() => _CanvasViewState();
}

class _CanvasViewState extends State<CanvasView> {
  /// 在线声明优先；离线用贡献缓存（冻结渲染）；两者皆无则占位
  _Lookup _lookup(String moduleId, String componentId) {
    for (final c in widget.found) {
      if (c.moduleId != moduleId) continue;
      for (final comp in c.contribution.components) {
        if (comp.id == componentId) return _Lookup(comp, false);
      }
      return const _Lookup(null, false); // 在线但组件不存在（契约变化）
    }
    final cached = widget.cache[moduleId];
    if (cached != null) {
      for (final comp in cached.components) {
        if (comp.id == componentId) return _Lookup(comp, true);
      }
    }
    return const _Lookup(null, false);
  }

  @override
  Widget build(BuildContext context) {
    final canvas = widget.canvas;
    return Container(
      color: Theme.of(context).colorScheme.surfaceContainerLowest,
      child: Stack(children: [
        for (final p in canvas.placements)
          Positioned(
            left: p.x,
            top: p.y,
            width: p.w,
            height: p.h,
            child: _placementCard(p),
          ),
      ]),
    );
  }

  Widget _placementCard(Placement p) {
    final look = _lookup(p.moduleId, p.componentId);
    final comp = look.comp;
    final offline = look.offline;
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
                      (offline ? ' · 离线' : '') +
                      (comp == null ? '（未知组件）' : ''),
                  style: TextStyle(
                      fontSize: 11,
                      color: offline ? Theme.of(context).disabledColor : null),
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
                  : Opacity(
                      // 冻结禁用：样式保持、交互禁用，重连自动恢复
                      opacity: offline ? 0.55 : 1.0,
                      child: AbsorbPointer(
                        absorbing: offline,
                        child: ComponentView(
                          moduleId: p.moduleId,
                          component: comp,
                          outbound: widget.outbound,
                          tick: widget.tick,
                          onAction: widget.onChanged,
                          disabled: offline,
                        ),
                      ),
                    ),
            ),
          ),
        ]),
      ),
    );
  }
}

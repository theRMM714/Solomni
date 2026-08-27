/// 主壳：左调色板（列表）+ 右画布区。呈现主权归本模块。
library;

import 'package:flutter/material.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_canvas/ui_canvas.dart';
import '../discovery.dart';
import '../layout_store.dart';
import 'canvas_view.dart';
import 'palette_view.dart';

class UiShell extends StatefulWidget {
  final Outbound? outbound; // null = standalone（未连核心）
  const UiShell({super.key, this.outbound});

  @override
  State<UiShell> createState() => _UiShellState();
}

class _UiShellState extends State<UiShell> {
  final _engine = LayoutEngine();
  final _store = LayoutStore();
  List<Contributed> _found = const [];
  LayoutDoc _layout = LayoutDoc();
  int _tab = 0;
  int _tick = 0; // 每次动作后递增：驱动数据组件（楼层等）刷新

  @override
  void initState() {
    super.initState();
    _boot();
  }

  Future<void> _boot() async {
    if (widget.outbound == null) {
      setState(() => _layout = LayoutDoc());
      return;
    }
    final found = await discover(widget.outbound!);
    final saved = await _store.load();
    final layout = saved ?? _engine.initial(found);
    setState(() {
      _found = found;
      _layout = layout;
      _tab = 0;
    });
    if (saved == null) await _store.save(layout);
  }

  void _afterAction() {
    setState(() => _tick++);
    _store.save(_layout);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'assistant',
      home: Scaffold(
        appBar: AppBar(
          title: const Text('assistant'),
          actions: [
            if (widget.outbound == null)
              const Padding(
                padding: EdgeInsets.only(right: 12),
                child: Center(child: Text('未连接核心（standalone）')),
              ),
          ],
        ),
        body: Row(children: [
          SizedBox(width: 280, child: PaletteView(found: _found)),
          const VerticalDivider(width: 1),
          Expanded(
            child: CanvasArea(
              engine: _engine,
              layout: _layout,
              found: _found,
              tab: _tab,
              tick: _tick,
              outbound: widget.outbound,
              onSelectTab: (i) => setState(() => _tab = i),
              onNewCanvas: (title) {
                setState(() => _engine.newCanvas(_layout, title));
                _store.save(_layout);
              },
              onChanged: _afterAction,
            ),
          ),
        ]),
      ),
    );
  }
}

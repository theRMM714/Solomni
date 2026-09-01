/// 主壳：左模块列表 + 右画布区。呈现主权归本模块。
/// 空白起步：画布全由用户创建；离线模块冻结禁用（贡献缓存）。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_canvas/ui_canvas.dart';
import 'package:ui_vocab/ui_vocab.dart';
import '../discovery.dart';
import '../layout_store.dart';
import 'canvas_view.dart';
import 'palette_view.dart';

class UiShell extends StatefulWidget {
  final Outbound? outbound; // null = standalone（未连核心）
  final LayoutStore? store; // 布局存储注入点（测试替换内存实现）；默认用户数据目录
  const UiShell({super.key, this.outbound, this.store});

  @override
  State<UiShell> createState() => _UiShellState();
}

class _UiShellState extends State<UiShell> {
  final _engine = LayoutEngine();
  late final LayoutStore _store = widget.store ?? LayoutStore();
  List<Contributed> _found = const [];
  final _cache = <String, UiContribution>{}; // 贡献缓存：离线冻结渲染用
  LayoutDoc _layout = LayoutDoc();
  int _tab = 0;
  int _tick = 0; // 每次动作后递增：驱动数据组件（楼层等）刷新
  StreamSubscription<WiringSnapshot>? _wiringSub; // 配对推送订阅
  Future<void> _refreshChain = Future.value(); // 串行化刷新，防竞态

  @override
  void initState() {
    super.initState();
    _boot();
    // 配对推送：成员变化即刷新模块列表（勿轮询）。
    // 布局保留（组件 id 是契约），新组件经画布"+"进入。
    _wiringSub = widget.outbound?.wiring.listen((_) {
      _refreshChain = _refreshChain.then((_) => _refresh());
    });
  }

  @override
  void dispose() {
    _wiringSub?.cancel();
    super.dispose();
  }

  Future<void> _refresh() async {
    final out = widget.outbound;
    if (out == null || !mounted) return;
    final found = await discover(out);
    if (!mounted) return;
    setState(() {
      _found = found;
      for (final c in found) {
        _cache[c.moduleId] = c.contribution;
      }
    });
  }

  Future<void> _boot() async {
    if (widget.outbound == null) {
      setState(() => _layout = LayoutDoc());
      return;
    }
    final found = await discover(widget.outbound!);
    for (final c in found) {
      _cache[c.moduleId] = c.contribution;
    }
    // 空白起步：无保存布局则从零开始，画布由用户创建
    final saved = await _store.load();
    setState(() {
      _found = found;
      _layout = saved ?? LayoutDoc();
      _tab = 0;
    });
  }

  void _afterAction() {
    setState(() => _tick++);
    _store.save(_layout);
  }

  // ---- 画布创建 ----

  void _newModuleCanvas(String moduleId) {
    setState(() {
      _engine.newCanvas(_layout, moduleId, ownerModuleId: moduleId);
      _tab = _layout.canvases.length - 1; // 切到新画布
    });
    _store.save(_layout);
  }

  void _newCustomCanvas(String title) {
    setState(() {
      _engine.newCanvas(_layout, title);
      _tab = _layout.canvases.length - 1; // 切到新画布
    });
    _store.save(_layout);
  }

  // ---- 布局导入导出（用户数据可携带） ----

  Future<void> _exportLayout() async {
    final path = await _promptPath(
        '导出布局', _store.defaultExportPath);
    if (path == null) return;
    try {
      final file = File(path);
      await file.parent.create(recursive: true);
      await file.writeAsString(
          const JsonEncoder.withIndent('  ').convert(_layout.toJson()));
      _toast('已导出: ' + path);
    } catch (e) {
      _toast('导出失败: ' + e.toString());
    }
  }

  Future<void> _importLayout() async {
    final path = await _promptPath('导入布局', _store.defaultExportPath);
    if (path == null) return;
    try {
      final j = jsonDecode(await File(path).readAsString());
      if (j is! Map) throw const FormatException('布局文件格式错误');
      setState(() {
        _layout = _engine.importFrom(Map<String, Object?>.from(j));
        _tab = 0;
      });
      await _store.save(_layout);
      _toast('已导入（画布 id 已重生成）');
    } catch (e) {
      _toast('导入失败: ' + e.toString());
    }
  }

  Future<String?> _promptPath(String title, String initial) async {
    final ctl = TextEditingController(text: initial);
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(title + '（文件路径）'),
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
    return (ok ?? false) && ctl.text.trim().isNotEmpty
        ? ctl.text.trim()
        : null;
  }

  void _toast(String msg) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text(msg)));
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
            PopupMenuButton<String>(
              onSelected: (v) =>
                  v == 'export' ? _exportLayout() : _importLayout(),
              itemBuilder: (_) => const [
                PopupMenuItem(value: 'export', child: Text('导出布局')),
                PopupMenuItem(value: 'import', child: Text('导入布局')),
              ],
            ),
          ],
        ),
        body: Row(children: [
          SizedBox(
            width: 280,
            child: PaletteView(
              found: _found,
              layout: _layout,
              outbound: widget.outbound,
              onCreateModuleCanvas: _newModuleCanvas,
            ),
          ),
          const VerticalDivider(width: 1),
          Expanded(
            child: CanvasArea(
              engine: _engine,
              layout: _layout,
              found: _found,
              cache: _cache,
              tab: _tab,
              tick: _tick,
              outbound: widget.outbound,
              onSelectTab: (i) => setState(() => _tab = i),
              onNewCanvas: _newCustomCanvas,
              onChanged: _afterAction,
            ),
          ),
        ]),
      ),
    );
  }
}

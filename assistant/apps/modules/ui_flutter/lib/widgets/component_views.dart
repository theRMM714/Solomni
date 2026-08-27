/// 组件渲染：按 kind 渲染，未知类型降级占位。
/// 数据组件按 bind 定向拉取；动作组件把 ui.event 路由回声明模块。
library;

import 'package:flutter/material.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_vocab/ui_vocab.dart';

class ComponentView extends StatelessWidget {
  final String moduleId;
  final UiComponent component;
  final Outbound? outbound;
  final int tick;
  final VoidCallback onAction;

  const ComponentView({
    super.key,
    required this.moduleId,
    required this.component,
    required this.outbound,
    required this.tick,
    required this.onAction,
  });

  @override
  Widget build(BuildContext context) {
    switch (component.kind) {
      case UiKind.textStream:
        return _FloorView(
            moduleId: moduleId, bind: component.bind, outbound: outbound, tick: tick);
      case UiKind.textInput:
      case UiKind.formField:
        return _InputView(
            moduleId: moduleId, componentId: component.id, outbound: outbound, onAction: onAction);
      case UiKind.list:
      case UiKind.textOutput:
        return _BoundTextView(
            moduleId: moduleId, bind: component.bind, outbound: outbound, tick: tick);
      default:
        return Center(child: Text('未知组件类型 ' + component.kind));
    }
  }
}

/// 对话楼层：按 bind 定向拉取历史消息（用户左、助手右）
class _FloorView extends StatefulWidget {
  final String moduleId;
  final String? bind;
  final Outbound? outbound;
  final int tick;

  const _FloorView(
      {required this.moduleId, required this.bind, required this.outbound, required this.tick});

  @override
  State<_FloorView> createState() => _FloorViewState();
}

class _FloorViewState extends State<_FloorView> {
  List<Map<String, Object?>> _messages = const [];
  String? _error;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  @override
  void didUpdateWidget(_FloorView old) {
    super.didUpdateWidget(old);
    if (old.tick != widget.tick) _refresh();
  }

  Future<void> _refresh() async {
    if (widget.outbound == null) {
      setState(() => _error = '未连接核心');
      return;
    }
    if (widget.bind == null) return;
    try {
      final r = await widget.outbound!
          .call(widget.moduleId, widget.bind!, null);
      if (!mounted) return;
      setState(() {
        _error = null;
        _messages = [
          for (final m in (r as List? ?? const []))
            Map<String, Object?>.from(m as Map)
        ];
      });
    } catch (e) {
      if (mounted) setState(() => _error = e.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) {
      return Center(child: Text(_error!, style: const TextStyle(fontSize: 11)));
    }
    return ListView.builder(
      itemCount: _messages.length,
      itemBuilder: (context, i) {
        final m = _messages[i];
        final mine = m['role'] == 'user';
        return Align(
          alignment: mine ? Alignment.centerRight : Alignment.centerLeft,
          child: Container(
            margin: const EdgeInsets.symmetric(vertical: 3),
            padding: const EdgeInsets.all(8),
            constraints: const BoxConstraints(maxWidth: 260),
            decoration: BoxDecoration(
              color: mine
                  ? Theme.of(context).colorScheme.primaryContainer
                  : Theme.of(context).colorScheme.surfaceContainerHighest,
              borderRadius: BorderRadius.circular(8),
            ),
            child: Text((m['content'] ?? '').toString()),
          ),
        );
      },
    );
  }
}

/// 输入框：submit 事件路由回声明模块（业务不出模块边界）
class _InputView extends StatefulWidget {
  final String moduleId;
  final String componentId;
  final Outbound? outbound;
  final VoidCallback onAction;

  const _InputView(
      {required this.moduleId, required this.componentId, required this.outbound, required this.onAction});

  @override
  State<_InputView> createState() => _InputViewState();
}

class _InputViewState extends State<_InputView> {
  final _ctl = TextEditingController();
  bool _busy = false;

  @override
  void dispose() {
    _ctl.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final text = _ctl.text.trim();
    if (text.isEmpty || _busy) return;
    if (widget.outbound == null) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('未连接核心')));
      }
      return;
    }
    setState(() => _busy = true);
    try {
      await widget.outbound!.call(widget.moduleId, UiCaps.event, {
        'component': widget.componentId,
        'event': 'submit',
        'payload': {'text': text},
      });
      _ctl.clear();
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(e.toString())));
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
    widget.onAction(); // 刷新画布上的数据组件（楼层）
  }

  @override
  Widget build(BuildContext context) {
    return Row(children: [
      Expanded(
        child: TextField(
          controller: _ctl,
          enabled: !_busy,
          decoration: const InputDecoration(isDense: true, hintText: '输入…'),
          onSubmitted: (_) => _submit(),
        ),
      ),
      IconButton(
        onPressed: _busy ? null : _submit,
        icon: _busy
            ? const SizedBox(
                width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2))
            : const Icon(Icons.send),
      ),
    ]);
  }
}

/// 绑定数据文本视图（list/textOutput）：定向拉取 bind 能力展示
class _BoundTextView extends StatefulWidget {
  final String moduleId;
  final String? bind;
  final Outbound? outbound;
  final int tick;

  const _BoundTextView(
      {required this.moduleId, required this.bind, required this.outbound, required this.tick});

  @override
  State<_BoundTextView> createState() => _BoundTextViewState();
}

class _BoundTextViewState extends State<_BoundTextView> {
  String _text = '';
  String? _error;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  @override
  void didUpdateWidget(_BoundTextView old) {
    super.didUpdateWidget(old);
    if (old.tick != widget.tick) _refresh();
  }

  Future<void> _refresh() async {
    if (widget.outbound == null || widget.bind == null) return;
    try {
      final r = await widget.outbound!.call(widget.moduleId, widget.bind!, null);
      if (!mounted) return;
      setState(() {
        _error = null;
        _text = r is List ? r.map((e) => e.toString()).join(', ') : (r ?? '').toString();
      });
    } catch (e) {
      if (mounted) setState(() => _error = e.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) {
      return Center(child: Text(_error!, style: const TextStyle(fontSize: 11)));
    }
    return SingleChildScrollView(child: Text(_text));
  }
}

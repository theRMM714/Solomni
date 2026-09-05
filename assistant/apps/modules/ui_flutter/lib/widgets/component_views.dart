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
  final bool disabled; // 离线冻结：不刷新数据、禁用交互，保持最后状态

  const ComponentView({
    super.key,
    required this.moduleId,
    required this.component,
    required this.outbound,
    required this.tick,
    required this.onAction,
    this.disabled = false,
  });

  @override
  Widget build(BuildContext context) {
    switch (component.kind) {
      case UiKind.textStream:
        return _FloorView(
            moduleId: moduleId,
            bind: component.bind,
            outbound: outbound,
            tick: tick,
            disabled: disabled);
      case UiKind.textInput:
      case UiKind.formField:
        return _InputView(
            moduleId: moduleId,
            component: component,
            outbound: outbound,
            onAction: onAction,
            disabled: disabled);
      case UiKind.list:
      case UiKind.textOutput:
        return _BoundTextView(
            moduleId: moduleId,
            bind: component.bind,
            outbound: outbound,
            tick: tick,
            disabled: disabled);
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
  final bool disabled;

  const _FloorView(
      {required this.moduleId,
      required this.bind,
      required this.outbound,
      required this.tick,
      this.disabled = false});

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
    if (old.tick != widget.tick && !widget.disabled) _refresh();
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

/// 输入框：submit 事件路由回声明模块（业务不出模块边界）。
/// 声明了 fields 的组件渲染多字段表单（提交键 = 字段 id）；
/// 未声明则维持单字段旧形态（提交键 'text'）。
class _InputView extends StatefulWidget {
  final String moduleId;
  final UiComponent component;
  final Outbound? outbound;
  final VoidCallback onAction;
  final bool disabled;

  const _InputView(
      {required this.moduleId,
      required this.component,
      required this.outbound,
      required this.onAction,
      this.disabled = false});

  @override
  State<_InputView> createState() => _InputViewState();
}

class _InputViewState extends State<_InputView> {
  late final List<UiField> _fields = widget.component.fields.isEmpty
      ? const [UiField('text')]
      : widget.component.fields;
  late final Map<String, TextEditingController> _ctls = {
    for (final f in _fields) f.id: TextEditingController(),
  };
  bool _busy = false;

  @override
  void dispose() {
    for (final c in _ctls.values) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _submit() async {
    final payload = {
      for (final f in _fields) f.id: _ctls[f.id]!.text.trim(),
    };
    if (_busy || payload.values.every((t) => t.isEmpty)) return;
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
        'component': widget.component.id,
        'event': 'submit',
        'payload': payload,
      });
      for (final c in _ctls.values) {
        c.clear();
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(e.toString())));
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
    widget.onAction(); // 刷新画布上的数据组件（楼层/列表）
  }

  @override
  Widget build(BuildContext context) {
    Widget fieldBox(UiField f, {bool last = false}) => Padding(
          padding: EdgeInsets.only(bottom: last ? 0 : 6),
          child: TextField(
            controller: _ctls[f.id],
            enabled: !_busy && !widget.disabled,
            obscureText: f.secret,
            decoration: InputDecoration(
              isDense: true,
              hintText: f.label.isEmpty ? '输入…' : f.label,
            ),
            onSubmitted: (_) => _submit(),
          ),
        );
    final send = IconButton(
      onPressed: _busy ? null : _submit,
      icon: _busy
          ? const SizedBox(
              width: 14, height: 14, child: CircularProgressIndicator(strokeWidth: 2))
          : const Icon(Icons.send),
    );
    // 单字段：输入+键同行；多字段：末行并排放键，避免竖向溢出
    return _fields.length == 1
        ? Row(children: [Expanded(child: fieldBox(_fields.first)), send])
        : Column(children: [
            for (var i = 0; i < _fields.length - 1; i++) fieldBox(_fields[i]),
            Row(children: [
              Expanded(child: fieldBox(_fields.last, last: true)),
              send,
            ]),
          ]);
  }
}

/// 绑定数据文本视图（list/textOutput）：定向拉取 bind 能力展示
class _BoundTextView extends StatefulWidget {
  final String moduleId;
  final String? bind;
  final Outbound? outbound;
  final int tick;
  final bool disabled;

  const _BoundTextView(
      {required this.moduleId,
      required this.bind,
      required this.outbound,
      required this.tick,
      this.disabled = false});

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
    if (old.tick != widget.tick && !widget.disabled) _refresh();
  }

  Future<void> _refresh() async {
    if (widget.outbound == null || widget.bind == null) return;
    try {
      final r = await widget.outbound!.call(widget.moduleId, widget.bind!, null);
      if (!mounted) return;
      setState(() {
        _error = null;
        _text = r is List ? r.map(_fmtItem).join('\n') : (r ?? '').toString();
      });
    } catch (e) {
      if (mounted) setState(() => _error = e.toString());
    }
  }

  /// 条目格式化：{name, remark} 形态渲染为「名字（备注）」，其余按字符串
  String _fmtItem(dynamic e) {
    if (e is Map && e['name'] != null) {
      final remark = (e['remark'] ?? '').toString();
      return remark.isEmpty
          ? e['name'].toString()
          : '${e['name']}（$remark）';
    }
    return e.toString();
  }

  @override
  Widget build(BuildContext context) {
    if (_error != null) {
      return Center(child: Text(_error!, style: const TextStyle(fontSize: 11)));
    }
    return SingleChildScrollView(child: Text(_text));
  }
}

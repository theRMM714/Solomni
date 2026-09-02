/// conversation 模块：自治程序。
/// 对话楼层 + 输入框两个 UI 组件；多轮历史归属本模块（builtin-only）；
/// llm.chat 用 prefer-shared（无共享时内建直连兜底）。
library;

import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:ui_vocab/ui_vocab.dart';
import 'package:user_data/user_data.dart';

// ---- 需求端口：内建与共享插同一个口 ----

abstract interface class LlmPort {
  /// 输入完整多轮消息，返回一段回复（流式 token 后续协议扩展）
  Stream<String> chat(List<Map<String, Object?>> messages);
}

/// 内建实现：有意做薄，单机够用即止（回声，无网络）
final class DirectLlm implements LlmPort {
  @override
  Stream<String> chat(List<Map<String, Object?>> messages) async* {
    final last = messages.last['content'] as String;
    yield '[内建直连] 收到：「' + last + '」。（单机模式，无核心参与）';
  }
}

/// 共享适配：端口调用翻译成消息，token 流直通；NoProvider 时回退内建（中心化是能力不是前提）
final class CoordinatedLlm implements LlmPort {
  final Outbound _out;
  final LlmPort _builtin;
  CoordinatedLlm(this._out, this._builtin);

  @override
  Stream<String> chat(List<Map<String, Object?>> messages) async* {
    try {
      await for (final t in _out.rpcStream(Caps.llmChat, {'messages': messages})) {
        yield t as String;
      }
    } on NoProviderException {
      yield* _builtin.chat(messages);
    }
  }
}

// ---- 模块本体：多轮历史归属本模块 ----

class ConversationModule {
  final LlmPort _llm;
  final _messages = <Map<String, Object?>>[];
  final File _file;
  bool _loaded = false;

  ConversationModule(this._llm, {File? store})
      : _file = store ?? UserData.file('conversation', 'history.json');

  /// 历史持久化：数据归模块，落在 userdata/（模块自己文件夹内）
  Future<void> _ensureLoaded() async {
    if (_loaded) return;
    _loaded = true;
    if (await _file.exists()) {
      try {
        final j = jsonDecode(await _file.readAsString()) as List;
        for (final m in j) {
          _messages.add(Map<String, Object?>.from(m as Map));
        }
      } catch (_) {
        // 历史损坏则从空开始，不崩溃
      }
    }
  }

  Future<void> _save() async {
    await _file.parent.create(recursive: true);
    await _file.writeAsString(jsonEncode(_messages));
  }

  /// 多轮：历史 + 本条 -> LLM -> token 直通消费方，完整回复落史
  Stream<String> send(String text) async* {
    await _ensureLoaded();
    _messages.add({'role': 'user', 'content': text});
    final buf = StringBuffer();
    await for (final t in _llm.chat(List.unmodifiable(_messages))) {
      buf.write(t);
      yield t;
    }
    _messages.add({'role': 'assistant', 'content': buf.toString()});
    await _save();
  }

  Future<List<Map<String, Object?>>> history() async {
    await _ensureLoaded();
    return List.unmodifiable(_messages);
  }
}

// ---- 模块程序：自治程序对外的全部 ----

final class ConversationProgram implements ModuleProgram {
  @override
  Declaration get declaration => const Declaration(
        'conversation',
        kind: ModuleKind.service,
        provides: [
          Provide(Caps.chatSend),
          Provide(Caps.chatHistory),
          Provide(UiCaps.contribution),
          Provide(UiCaps.event),
        ],
        needs: [Need(Caps.llmChat, NeedVia.preferShared)],
      );

  /// UI 贡献：只有意图（组件 id/类型/绑定），没有像素。
  /// 当前只有两个组件：对话楼层 + 输入框。
  static const contribution = UiContribution(
    components: [
      UiComponent('chat_floor', UiKind.textStream, label: '对话楼层', bind: Caps.chatHistory),
      UiComponent('chat_input', UiKind.textInput, label: '输入', bind: Caps.chatSend),
    ],
    commands: [
      UiCommand('send', args: ['text'], description: '发送一条消息', component: 'chat_input', event: 'submit'),
    ],
  );

  @override
  ModuleHandler bind(Outbound outbound) {
    final conv = ConversationModule(CoordinatedLlm(outbound, DirectLlm()));
    Future<String> sendText(String text) async {
      final buf = StringBuffer();
      await for (final t in conv.send(text)) {
        buf.write(t);
      }
      return buf.toString();
    }

    return (env) async {
      if (env.method == Caps.chatSend) {
        // 流式：token 经核心事件封直达消费方；历史落库在流内完成
        final text = (env.params as Map)['text'] as String;
        return Streamed(conv.send(text));
      }
      if (env.method == Caps.chatHistory) {
        return await conv.history();
      }
      if (env.method == UiCaps.contribution) {
        return contribution.toJson();
      }
      if (env.method == UiCaps.event) {
        // 事件路由回本模块：组件语义归模块，UI 只转发手势
        final p = env.params as Map;
        if (p['component'] == 'chat_input' && p['event'] == 'submit') {
          final payload = (p['payload'] as Map?) ?? const {};
          return sendText(payload['text'] as String? ?? '');
        }
        throw UnsupportedError('conversation 不认识的组件事件: ' + p.toString());
      }
      throw UnsupportedError('conversation 不认识 ' + env.method);
    };
  }
}

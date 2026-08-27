import 'package:test/test.dart';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';

void main() {
  test('preferShared 无提供方 -> 内建兜底（空提供方）', () {
    final b = Broker()
      ..register(const Declaration('conversation', needs: [
        Need(Caps.llmChat, NeedVia.preferShared),
      ]));
    final w = b.resolve();
    expect(w['conversation']![Caps.llmChat], '');
  });

  test('唯一提供方 -> 接线到它', () {
    final b = Broker()
      ..register(const Declaration('llm_gateway',
          provides: [Provide(Caps.llmChat)]))
      ..register(const Declaration('conversation', needs: [
        Need(Caps.llmChat, NeedVia.preferShared),
      ]));
    final w = b.resolve();
    expect(w['conversation']![Caps.llmChat], 'llm_gateway');
  });

  test('多候选 -> fail-fast 列出候选，绝不静默选择', () {
    final b = Broker()
      ..register(const Declaration('llm_a', provides: [Provide(Caps.llmChat)]))
      ..register(const Declaration('llm_b', provides: [Provide(Caps.llmChat)]))
      ..register(const Declaration('conversation', needs: [
        Need(Caps.llmChat, NeedVia.preferShared),
      ]));
    expect(() => b.resolve(), throwsA(isA<WiringException>()));
  });

  test('显式接线优先：部署者的权力，不是核心的', () {
    final b = Broker()
      ..register(const Declaration('llm_a', provides: [Provide(Caps.llmChat)]))
      ..register(const Declaration('llm_b', provides: [Provide(Caps.llmChat)]))
      ..register(const Declaration('conversation', needs: [
        Need(Caps.llmChat, NeedVia.preferShared),
      ]));
    final w = b.resolve(explicit: {
      'conversation': {Caps.llmChat: 'llm_b'}
    });
    expect(w['conversation']![Caps.llmChat], 'llm_b');
  });

  test('sharedOnly 无提供方 -> 启动失败（功能降级需装配者显式处理）', () {
    final b = Broker()
      ..register(const Declaration('ui', needs: [
        Need('stt.transcribe', NeedVia.sharedOnly),
      ]));
    expect(() => b.resolve(), throwsA(isA<WiringException>()));
  });

  test('提供方依赖成环 -> 启动失败', () {
    final b = Broker()
      ..register(const Declaration('a', provides: [
        Provide('cap.y'),
      ], needs: [
        Need('cap.x', NeedVia.preferShared),
      ]))
      ..register(const Declaration('b', provides: [
        Provide('cap.x'),
      ], needs: [
        Need('cap.y', NeedVia.preferShared),
      ]));
    expect(() => b.resolve(), throwsA(isA<WiringException>()));
  });

  test('builtinOnly 不产生跨模块边，不成环', () {
    final b = Broker()
      ..register(const Declaration('a', provides: [
        Provide('cap.y'),
      ], needs: [
        Need('cap.x', NeedVia.builtinOnly),
      ]))
      ..register(const Declaration('b', provides: [
        Provide('cap.x'),
      ], needs: [
        Need('cap.y', NeedVia.builtinOnly),
      ]));
    expect(b.resolve(), isNotEmpty);
  });
}

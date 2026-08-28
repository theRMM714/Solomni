import 'dart:async';
import 'package:test/test.dart';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

/// 与 tool/smoke_test.dart 同一套用例（test 包版）。
Future<void> flush() => Future<void>.delayed(Duration.zero);

ModuleHandler get _echo => (env) async => {'to': env.to};

bool _same(List<String> a, List<String> b) =>
    a.length == b.length && a.every(b.contains);

void main() {
  test('preferShared 无提供方 -> 空集合（配对不失败）', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    expect((ex.snapshotOf('conversation')[Caps.llmChat] ?? const []).isEmpty,
        isTrue);
  });

  test('唯一提供方 -> 直达', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
        _echo);
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    expect(ex.routeOrNull('conversation', Caps.llmChat), 'llm_gateway');
  });

  test('多提供方 -> 全部入集合（不拒收）；无表态 -> Candidates（附候选清单）', () {
    final ex = Exchange();
    ex.connect(const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    expect(
        _same(ex.snapshotOf('conversation')[Caps.llmChat] ?? const [],
            ['llm_a', 'llm_b']),
        isTrue);
    expect(() => ex.routeOrNull('conversation', Caps.llmChat),
        throwsA(isA<CandidatesException>()));
  });

  test('显式接线 -> 路由到接线对象（装配者的权力，不改事实集合）', () {
    final ex = Exchange();
    ex.connect(const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    ex.start(explicit: {
      'conversation': {Caps.llmChat: 'llm_b'}
    });
    expect(ex.routeOrNull('conversation', Caps.llmChat), 'llm_b');
  });

  test('声明首选（Need.provider）-> 路由到首选', () {
    final ex = Exchange();
    ex.connect(const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(
        const Declaration('conversation', needs: [
          Need(Caps.llmChat, NeedVia.preferShared, provider: 'llm_b')
        ]),
        _echo);
    expect(ex.routeOrNull('conversation', Caps.llmChat), 'llm_b');
  });

  test('sharedOnly 无提供方 -> 集合空（降级不失败），路由 null', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('ui', needs: [Need('stt.transcribe', NeedVia.sharedOnly)]),
        _echo);
    expect((ex.snapshotOf('ui')['stt.transcribe'] ?? const []).isEmpty, isTrue);
    expect(ex.routeOrNull('ui', 'stt.transcribe'), isNull);
  });

  test('成环 -> 拒收新来者并回滚，既有成员保留', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('a',
            provides: [Provide('cap.y')],
            needs: [Need('cap.x', NeedVia.preferShared)]),
        _echo);
    expect(
        () => ex.connect(
            const Declaration('b',
                provides: [Provide('cap.x')],
                needs: [Need('cap.y', NeedVia.preferShared)]),
            _echo),
        throwsA(isA<WiringException>()));
    expect(ex.declarations.length, 1);
  });

  test('重复模块 id -> 拒收', () {
    final ex = Exchange();
    ex.connect(const Declaration('dup'), _echo);
    expect(() => ex.connect(const Declaration('dup'), _echo),
        throwsA(isA<WiringException>()));
  });

  test('builtinOnly -> 不入配对，路由 null', () {
    final ex = Exchange();
    ex.connect(const Declaration('a', provides: [Provide('cap.x')]), _echo);
    ex.connect(
        const Declaration('b', needs: [Need('cap.x', NeedVia.builtinOnly)]),
        _echo);
    expect(ex.snapshotOf('b').containsKey('cap.x'), isFalse);
    expect(ex.routeOrNull('b', 'cap.x'), isNull);
  });

  test('晚接入消费方 -> 自动配对（接入顺序无关）', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
        _echo);
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    expect(ex.routeOrNull('conversation', Caps.llmChat), 'llm_gateway');
  });

  test('提供方离线 -> 集合收缩、路由 null；回归 -> 自动恢复', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
        _echo);
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    ex.disconnect('llm_gateway');
    expect(ex.routeOrNull('conversation', Caps.llmChat), isNull);
    ex.connect(
        const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
        _echo);
    expect(ex.routeOrNull('conversation', Caps.llmChat), 'llm_gateway');
  });

  test('推送：成员变化 -> 每个在线模块收到自己的快照', () async {
    final ex = Exchange();
    final got = <String, List<Map<String, List<String>>>>{};
    ex.wiringChanges.listen((e) => (got[e.moduleId] ??= []).add(e.snapshot));
    ex.connect(
        const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
        _echo);
    await flush();
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    await flush();
    ex.disconnect('llm_gateway');
    await flush();
    expect(got['conversation']!.first[Caps.llmChat]!.length, 1);
    expect(got['llm_gateway']!.length, 2);
    expect((got['conversation']!.last[Caps.llmChat] ?? const []).isEmpty,
        isTrue);
  });

  test('outboundFor：NoProvider / Candidates 直达调用方；唯一直达', () async {
    final ex = Exchange();
    ex.connect(const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    final out = ex.outboundFor('conversation');
    expect(out.rpc(Caps.llmChat, null),
        throwsA(isA<CandidatesException>()));
    expect(out.rpc('stt.transcribe', null),
        throwsA(isA<NoProviderException>()));

    final ex2 = Exchange();
    ex2.connect(
        const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
        _echo);
    ex2.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    final r = await ex2.outboundFor('conversation').rpc(Caps.llmChat, null);
    expect((r as Map)['to'], 'llm_gateway');
  });

  test('显式接线指向未知 -> 装配期失败', () {
    final ex = Exchange();
    ex.connect(
        const Declaration('conversation',
            needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
        _echo);
    expect(
        () => ex.start(explicit: {
              'conversation': {Caps.llmChat: 'ghost'}
            }),
        throwsA(isA<WiringException>()));
  });

  test('Need.provider 线路往返', () {
    final d = Declaration.fromJson(const Declaration('x', needs: [
      Need('cap.c', NeedVia.preferShared, provider: 'p')
    ]).toJson());
    expect(d.needs.single.provider, 'p');
  });
}

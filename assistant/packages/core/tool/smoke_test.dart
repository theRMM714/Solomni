/// 骨架冒烟测试：配对（事实集合）与路由（机械规则）的断言版。
/// 与 test/broker_test.dart 同一套用例。
/// 环境限制（无子进程管道）下用 dart 直接 VM 调用执行。
library;

import 'dart:async';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

int _passed = 0;

void check(String name, bool cond) {
  if (!cond) {
    throw StateError('FAILED: ' + name);
  }
  _passed++;
  print('PASS: ' + name);
}

/// 广播流的事件在微任务里送达：一轮成员变化后冲刷
Future<void> flush() => Future<void>.delayed(Duration.zero);

/// 回显处理器：返回封套的 to（调用方据此断言路由目标）
ModuleHandler get _echo => (env) async => {'to': env.to};

bool _same(List<String> a, List<String> b) =>
    a.length == b.length && a.every(b.contains);

void main() async {
  // ---- 配对：事实集合，无单选 ----

  // 1. preferShared 无提供方：集合为空，配对不失败（降级在调用时发生）
  final ex1 = Exchange();
  ex1.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  check(
      'preferShared 无提供方 -> 空集合（配对不失败）',
      (ex1.snapshotOf('conversation')[Caps.llmChat] ?? const []).isEmpty);

  // 2. 唯一提供方：集合即它，路由直达
  final ex2 = Exchange();
  ex2.connect(
      const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
      _echo);
  ex2.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  check('唯一提供方 -> 直达',
      ex2.routeOrNull('conversation', Caps.llmChat) == 'llm_gateway');

  // 3. 多提供方：配对成功（诚实集合，新来者不拒收）；无表态 -> Candidates
  final ex3 = Exchange();
  ex3.connect(
      const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
  ex3.connect(
      const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
  ex3.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  check(
      '多提供方 -> 全部入集合（不拒收）',
      _same(ex3.snapshotOf('conversation')[Caps.llmChat] ?? const [],
          ['llm_a', 'llm_b']));
  var cand = const <String>[];
  try {
    ex3.routeOrNull('conversation', Caps.llmChat);
  } on CandidatesException catch (e) {
    cand = e.candidates;
  }
  check('多提供方无表态 -> Candidates（附候选清单）',
      _same(cand, ['llm_a', 'llm_b']));

  // 4. 显式接线：装配者的权力（只影响路由，不改事实集合）
  ex3.start(explicit: {
    'conversation': {Caps.llmChat: 'llm_b'}
  });
  check('显式接线 -> 路由到接线对象',
      ex3.routeOrNull('conversation', Caps.llmChat) == 'llm_b');

  // 5. 声明首选（Need.provider）：消费方策略选择，核心只机械执行
  final ex5 = Exchange();
  ex5.connect(
      const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
  ex5.connect(
      const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
  ex5.connect(
      const Declaration('conversation',
          needs: [
            Need(Caps.llmChat, NeedVia.preferShared, provider: 'llm_b')
          ]),
      _echo);
  check('声明首选 -> 路由到首选',
      ex5.routeOrNull('conversation', Caps.llmChat) == 'llm_b');

  // 6. sharedOnly 无提供方：配对不失败；路由 null = 降级信号
  final ex6 = Exchange();
  ex6.connect(
      const Declaration('ui',
          needs: [Need('stt.transcribe', NeedVia.sharedOnly)]),
      _echo);
  check('sharedOnly 无提供方 -> 集合空（降级不失败）',
      (ex6.snapshotOf('ui')['stt.transcribe'] ?? const []).isEmpty);
  check('sharedOnly 无提供方 -> 路由 null',
      ex6.routeOrNull('ui', 'stt.transcribe') == null);

  // 7. 成环 -> 校验拒收新来者，既有配对保留（核心继续服务）
  final ex7 = Exchange();
  ex7.connect(
      const Declaration('a',
          provides: [Provide('cap.y')], needs: [Need('cap.x', NeedVia.preferShared)]),
      _echo);
  var threw = false;
  try {
    ex7.connect(
        const Declaration('b',
            provides: [Provide('cap.x')],
            needs: [Need('cap.y', NeedVia.preferShared)]),
        _echo);
  } on WiringException {
    threw = true;
  }
  check('成环 -> 拒收新来者', threw);
  check('拒收后回滚（注册表只留既有成员）', ex7.declarations.length == 1);

  // 8. 重复 id -> 校验拒收（定向调用无歧义的根基）
  final ex8 = Exchange();
  ex8.connect(const Declaration('dup'), _echo);
  threw = false;
  try {
    ex8.connect(const Declaration('dup'), _echo);
  } on WiringException {
    threw = true;
  }
  check('重复模块 id -> 拒收', threw);

  // 9. builtinOnly：模块私有，不参与配对
  final ex9 = Exchange();
  ex9.connect(const Declaration('a', provides: [Provide('cap.x')]), _echo);
  ex9.connect(
      const Declaration('b', needs: [Need('cap.x', NeedVia.builtinOnly)]),
      _echo);
  check('builtinOnly -> 不入配对', !ex9.snapshotOf('b').containsKey('cap.x'));
  check('builtinOnly -> 路由 null（模块自有实现）',
      ex9.routeOrNull('b', 'cap.x') == null);

  // ---- 动态：配对随成员变化实时重算（接入顺序无关） ----

  // 10. 晚接入消费方：声明到达即与现有提供方配对
  final ex10 = Exchange();
  ex10.connect(
      const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
      _echo);
  ex10.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  check('晚接入消费方 -> 自动配对',
      ex10.routeOrNull('conversation', Caps.llmChat) == 'llm_gateway');

  // 11. 提供方离线：集合收缩，路由 null（消费方自动降级）
  ex10.disconnect('llm_gateway');
  check('提供方离线 -> 集合收缩',
      (ex10.snapshotOf('conversation')[Caps.llmChat] ?? const []).isEmpty);
  check('提供方离线 -> 路由 null（降级）',
      ex10.routeOrNull('conversation', Caps.llmChat) == null);

  // 12. 提供方回归：配对自动恢复
  ex10.connect(
      const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
      _echo);
  check('提供方回归 -> 自动恢复',
      ex10.routeOrNull('conversation', Caps.llmChat) == 'llm_gateway');

  // 13. 推送：每次成员变化，每个在线模块收到自己的快照（勿轮询）
  final ex13 = Exchange();
  final got = <String, List<Map<String, List<String>>>>{};
  ex13.wiringChanges.listen((e) => (got[e.moduleId] ??= []).add(e.snapshot));
  ex13.connect(const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]), _echo);
  await flush();
  ex13.connect(const Declaration('conversation',
      needs: [Need(Caps.llmChat, NeedVia.preferShared)]), _echo);
  await flush();
  ex13.disconnect('llm_gateway');
  await flush();
  check('新来者收到初始快照',
      got['conversation']!.first[Caps.llmChat]!.length == 1);
  check('成员变化 -> 在线模块各收到快照', got['llm_gateway']!.length == 2);
  check('提供方离线 -> 消费方快照更新为空集',
      (got['conversation']!.last[Caps.llmChat] ?? const []).isEmpty);

  // ---- 出边：路由异常直达调用方 ----

  // 14. outboundFor：NoProvider / Candidates 都能到达模块
  final ex14 = Exchange();
  ex14.connect(
      const Declaration('llm_a', provides: [Provide(Caps.llmChat)]), _echo);
  ex14.connect(
      const Declaration('llm_b', provides: [Provide(Caps.llmChat)]), _echo);
  ex14.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  final out = ex14.outboundFor('conversation');
  var candidates = false;
  var noProvider = false;
  try {
    await out.rpc(Caps.llmChat, null);
  } on CandidatesException {
    candidates = true;
  }
  try {
    await out.rpc('stt.transcribe', null);
  } on NoProviderException {
    noProvider = true;
  }
  check('rpc 多提供方无表态 -> CandidatesException', candidates);
  check('rpc 无提供方 -> NoProviderException', noProvider);

  // 15. outboundFor：路由结果送达声明的目标（回显 to 断言）
  final ex15 = Exchange();
  ex15.connect(
      const Declaration('llm_gateway', provides: [Provide(Caps.llmChat)]),
      _echo);
  ex15.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  final r15 = await ex15.outboundFor('conversation').rpc(Caps.llmChat, null);
  check('rpc 直达唯一提供方', (r15 as Map)['to'] == 'llm_gateway');

  // 16. 显式接线指向未知提供方 -> 装配期校验失败
  final ex16 = Exchange();
  ex16.connect(
      const Declaration('conversation',
          needs: [Need(Caps.llmChat, NeedVia.preferShared)]),
      _echo);
  threw = false;
  try {
    ex16.start(explicit: {
      'conversation': {Caps.llmChat: 'ghost'}
    });
  } on WiringException {
    threw = true;
  }
  check('显式接线指向未知 -> 装配期失败', threw);

  // 17. Need.provider 经线路往返（hello 的 params 编解码）
  final d17 = Declaration.fromJson(const Declaration('x', needs: [
    Need('cap.c', NeedVia.preferShared, provider: 'p')
  ]).toJson());
  check('Need.provider 线路往返', d17.needs.single.provider == 'p');

  print('');
  print('全部通过: ' + _passed.toString() + ' 项');
}

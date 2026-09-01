/// 核心包：内容无关的交易所。
/// 只提供机制（校验、配对、投递），不制定策略（什么该被共享）。
library;

import 'package:protocol/protocol.dart';

export 'exchange.dart';
export 'tcp.dart';

/// 校验失败（重复 id、成环、接线指向未知）：装配期/接入期 fail-fast，绝不静默决策
class WiringException implements Exception {
  final String message;
  WiringException(this.message);
  @override
  String toString() => 'WiringException: ' + message;
}

/// 配对结果：moduleId -> 能力地址 -> 当前全体提供方（事实集合，无单选）
typedef Wiring = Map<String, Map<String, List<String>>>;

class Broker {
  final _declarations = <String, Declaration>{};

  /// 模块 id 全局唯一：重复注册直接失败（id 唯一使定向调用无歧义）
  void register(Declaration d) {
    if (_declarations.containsKey(d.id)) {
      throw WiringException('模块 id 重复: ' + d.id);
    }
    _declarations[d.id] = d;
  }

  /// 模块离线即注销：进程重连（如热重启）可重新注册
  void unregister(String id) => _declarations.remove(id);

  /// 注册表只读列举（core.modules 元能力的事实来源）
  List<Declaration> get declarations => List.unmodifiable(_declarations.values);

  /// 配对：机械登记，无价值判断。
  /// need-cap -> 当前全体提供方（诚实集合：到达即加入，离线即移出；
  /// 多提供方是消费方的选择空间，不是错误；缺提供方是降级，不是失败）。
  /// builtinOnly 是模块私有，不参与配对。
  /// 成环 -> 校验拒收（唯一拒绝场景之一）。
  Wiring resolve() {
    _checkCycles();
    final wiring = <String, Map<String, List<String>>>{};
    for (final d in _declarations.values) {
      final w = <String, List<String>>{};
      for (final need in d.needs) {
        if (need.via == NeedVia.builtinOnly) continue;
        w[need.cap] = _providersOf(need.cap).toList();
      }
      wiring[d.id] = w;
    }
    return wiring;
  }

  /// 声明查询：模块在某能力上的首选（消费方策略选择，路由时机械执行）
  Need? needOf(String moduleId, String cap) {
    for (final n in _declarations[moduleId]?.needs ?? const <Need>[]) {
      if (n.cap == cap) return n;
    }
    return null;
  }

  Iterable<String> _providersOf(String cap) => _declarations.values
      .where((d) => d.provides.any((p) => p.cap == cap))
      .map((d) => d.id);

  /// 环检测：A 的提供方依赖 B，B 的提供方依赖 A -> 校验拒收（可预测的无限递归）
  void _checkCycles() {
    final edges = <String, Set<String>>{};
    for (final d in _declarations.values) {
      edges[d.id] = _providersOfDeep(d).toSet();
    }
    final state = <String, int>{}; // 0=unvisited 1=visiting 2=done
    void visit(String id, List<String> path) {
      if (state[id] == 2) return;
      if (state[id] == 1) {
        final cycle = [...path.skipWhile((x) => x != id), id].join(' -> ');
        throw WiringException('提供方依赖成环：' + cycle);
      }
      state[id] = 1;
      for (final next in edges[id] ?? const <String>{}) visit(next, [...path, id]);
      state[id] = 2;
    }
    for (final id in edges.keys) {
      if (state[id] == null) visit(id, []);
    }
  }

  /// 收集一个模块（经由其需求）可到达的提供方，忽略 builtinOnly（它不产生跨模块边）
  Iterable<String> _providersOfDeep(Declaration d) sync* {
    for (final need in d.needs) {
      if (need.via == NeedVia.builtinOnly) continue;
      yield* _providersOf(need.cap);
    }
  }
}

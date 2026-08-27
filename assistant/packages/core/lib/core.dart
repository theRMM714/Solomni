/// 核心包：内容无关的交易所。
/// 只提供机制（注册/撮合/路由），不制定策略（什么该被共享）。
library;

import 'package:protocol/protocol.dart';

export 'exchange.dart';
export 'tcp.dart';
export 'loader.dart';

/// 撮合失败：启动期 fail-fast，绝不静默决策
class WiringException implements Exception {
  final String message;
  WiringException(this.message);
  @override
  String toString() => 'WiringException: ' + message;
}

/// 装配结果：moduleId -> 能力地址 -> 提供方 moduleId
/// 内建兜底用空提供方 "" 表示
typedef Wiring = Map<String, Map<String, String>>;

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

  /// 撮合：机械规则，无价值判断。
  /// 1. explicit 接线优先（部署者的权力，不是核心的）
  /// 2. 多候选且未显式接线 -> 失败并列出候选
  /// 3. sharedOnly 无候选 -> 失败（该模块声明无法满足）
  /// 4. preferShared 无候选 -> 内建兜底 ""
  /// 5. 提供方依赖成环 -> 失败
  /// [explicit] 形如 {"conversation": {"llm.chat": "llm_openai"}}
  Wiring resolve({Map<String, Map<String, String>> explicit = const {}}) {
    _checkCycles();
    final wiring = <String, Map<String, String>>{};
    for (final d in _declarations.values) {
      final w = <String, String>{};
      for (final need in d.needs) {
        final candidates = _providersOf(need.cap).toList();
        final pinned = explicit[d.id]?[need.cap];
        if (pinned != null) {
          _assertKnown(pinned, need.cap);
          w[need.cap] = pinned;
          continue;
        }
        if (candidates.isEmpty) {
          if (need.via == NeedVia.sharedOnly) {
            throw WiringException(
                d.id + ' 的 ' + need.cap + ' (sharedOnly) 无提供方：按声明应功能降级，装配者需显式处理');
          }
          w[need.cap] = ''; // 内建兜底：中心化是能力不是前提
          continue;
        }
        if (candidates.length > 1) {
          throw WiringException(
              d.id + ' 的 ' + need.cap + ' 有多候选 ' + candidates.toString() + '：请显式接线（核心绝不静默选择）');
        }
        w[need.cap] = candidates.single;
      }
      wiring[d.id] = w;
    }
    return wiring;
  }

  Iterable<String> _providersOf(String cap) => _declarations.values
      .where((d) => d.provides.any((p) => p.cap == cap))
      .map((d) => d.id);

  void _assertKnown(String moduleId, String cap) {
    final d = _declarations[moduleId];
    if (d == null || !d.provides.any((p) => p.cap == cap)) {
      throw WiringException('显式接线指向未知提供方：' + moduleId + ' 不提供 ' + cap);
    }
  }

  /// 环检测：A 的提供方依赖 B，B 的提供方依赖 A -> 启动失败
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

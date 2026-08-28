/// Exchange：核心的进程内运行时面。
/// 只做三件事：收声明（校验）、配对（事实集合，成员变化即重算并推送）、按 to 投递。
library;

import 'dart:async';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'core.dart';

/// 配对事件：成员变化重算后，核心对每个在线模块的快照推送（事实，非决策）
class WiringEvent {
  final String moduleId;
  final WiringSnapshot snapshot; // 该模块各需求 -> 当前提供方集合
  const WiringEvent(this.moduleId, this.snapshot);
}

class Exchange {
  final Broker _broker = Broker();
  final _handlers = <String, ModuleHandler>{};
  Wiring _wiring = {};
  Map<String, Map<String, String>> _explicit = {};
  final _wiringEvents = StreamController<WiringEvent>.broadcast();

  /// 配对快照流：成员变化即推送（事实应随事实的变化到达，禁止轮询）
  Stream<WiringEvent> get wiringChanges => _wiringEvents.stream;

  /// 声明到达：登记并重算配对（重复 id -> 校验拒收；成环 -> 拒收并回滚新来者）
  void connect(Declaration d, ModuleHandler handler) {
    _broker.register(d);
    _handlers[d.id] = handler;
    try {
      _rewire();
    } on Object {
      // 校验失败：回滚新来者，既有配对保持原状（核心继续服务其它模块）
      _broker.unregister(d.id);
      _handlers.remove(d.id);
      rethrow;
    }
  }

  /// 模块离线：注销并重算（消费方集合自动收缩，降级在调用时自然发生）
  void disconnect(String id) {
    _broker.unregister(id);
    _handlers.remove(id);
    _rewire();
  }

  /// 装配者接线（部署者的权力）+ 装配期校验；接线只影响路由，不改变事实集合
  void start({Map<String, Map<String, String>> explicit = const {}}) {
    _validateExplicit(explicit);
    _explicit = explicit;
    _rewire();
  }

  void _validateExplicit(Map<String, Map<String, String>> explicit) {
    for (final e in explicit.entries) {
      for (final pin in e.value.entries) {
        final need = _broker.needOf(e.key, pin.key);
        if (need == null) {
          throw WiringException('显式接线对象未声明该需求：' +
              e.key + ' 不需要 ' + pin.key);
        }
        final target = _broker.declarations
            .where((t) => t.id == pin.value)
            .expand((t) => t.provides)
            .any((p) => p.cap == pin.key);
        if (!target) {
          throw WiringException(
              '显式接线指向未知提供方：' + pin.value + ' 不提供 ' + pin.key);
        }
      }
    }
  }

  void _rewire() {
    _wiring = _broker.resolve();
    // 推送：每个在线模块收到自己的快照（新来者含初始快照；无需求模块收到
    // 空快照，其流上每次发射即"成员已变化"信号，UI 据此实时刷新）
    for (final d in _broker.declarations) {
      _wiringEvents.add(WiringEvent(d.id, _wiring[d.id] ?? const {}));
    }
  }

  /// 模块当前的配对快照（推送用）：各需求 -> 提供方集合
  WiringSnapshot snapshotOf(String moduleId) => _wiring[moduleId] ?? const {};

  /// 路由（投递的机械规则）：显式接线 > 声明首选 > 唯一提供方。
  /// 多提供方且无表态 -> 抛 Candidates（选择权交回调用方，核心绝不代选）。
  /// 无提供方 -> null（消费方按声明策略降级）。
  String? routeOrNull(String moduleId, String cap) {
    final providers = _wiring[moduleId]?[cap];
    if (providers == null || providers.isEmpty) return null;
    if (providers.length == 1) return providers.single;
    final pinned = _explicit[moduleId]?[cap];
    if (pinned != null && providers.contains(pinned)) return pinned;
    final preferred = _broker.needOf(moduleId, cap)?.provider;
    if (preferred != null && providers.contains(preferred)) return preferred;
    throw CandidatesException(cap, providers);
  }

  /// 给模块的发边：路由延迟解析，配对变化实时生效
  Outbound outboundFor(String moduleId) => _ExchangeOutbound(this, moduleId);

  /// 模块间唯一通路：经核心按 to 投递。
  /// requestId：调用方 rpc id，供 TCP 守护把流式事件路由回原调用
  Future<Object?> rpcTo(String fromId, String toId, String method, Object? params,
      {String? requestId}) async {
    final handler = _handlers[toId] ?? (throw StateError('未知模块：' + toId));
    final env = Envelope(
      from: fromId,
      to: toId,
      kind: EnvelopeKind.rpc,
      id: requestId,
      method: method,
      params: params,
    );
    return handler(env);
  }

  /// 注册表只读列举（core.modules 元能力）
  List<Declaration> get declarations => _broker.declarations;
}

final class _ExchangeOutbound implements Outbound {
  final Exchange _ex;
  final String _from;
  _ExchangeOutbound(this._ex, this._from);

  @override
  Future<Object?> rpc(String method, Object? params) async {
    final provider = _ex.routeOrNull(_from, method);
    if (provider == null) throw NoProviderException(method);
    return _ex.rpcTo(_from, provider, method, params);
  }

  @override
  Stream<Object?> rpcStream(String method, Object? params) async* {
    final provider = _ex.routeOrNull(_from, method);
    if (provider == null) throw NoProviderException(method);
    final result = await _ex.rpcTo(_from, provider, method, params);
    // 进程内零拷贝：Streamed 直通流；普通结果回退单块
    if (result is Streamed) {
      yield* result.chunks;
    } else if (result != null) {
      yield result;
    }
  }

  @override
  Future<Object?> call(String moduleId, String method, Object? params) =>
      _ex.rpcTo(_from, moduleId, method, params);

  @override
  Stream<WiringSnapshot> get wiring => _ex.wiringChanges
      .where((e) => e.moduleId == _from)
      .map((e) => e.snapshot);
}

/// Exchange：核心的进程内运行时面。
/// 只做三件事：收声明、撮合接线、按 to 模块 id 机械投递封套。
library;

import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'core.dart';

class Exchange {
  final Broker _broker = Broker();
  final _handlers = <String, ModuleHandler>{};
  Wiring _wiring = {};

  void connect(Declaration d, ModuleHandler handler) {
    _broker.register(d);
    _handlers[d.id] = handler;
  }

  /// 撮合并启动：歧义/成环/sharedOnly 无着落都会在此 fail-fast
  void start({Map<String, Map<String, String>> explicit = const {}}) {
    _wiring = _broker.resolve(explicit: explicit);
  }

  /// 装配结果查询：某模块某能力的提供方（"" = 内建兜底；null = 未声明）
  String? providerOfOrNull(String moduleId, String cap) =>
      _wiring[moduleId]?[cap];

  String providerOf(String moduleId, String cap) =>
      providerOfOrNull(moduleId, cap) ??
      (throw StateError('未装配：' + moduleId + ' / ' + cap));

  /// 给模块的发边：能力地址延迟解析，内建兜底时抛 NoProvider
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

  /// 模块离线注销（进程重连可重新注册）
  void disconnect(String id) {
    _broker.unregister(id);
    _handlers.remove(id);
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
    final provider = _ex.providerOfOrNull(_from, method);
    if (provider == null || provider.isEmpty) {
      throw NoProviderException(method);
    }
    return _ex.rpcTo(_from, provider, method, params);
  }

  @override
  Stream<Object?> rpcStream(String method, Object? params) async* {
    final provider = _ex.providerOfOrNull(_from, method);
    if (provider == null || provider.isEmpty) {
      throw NoProviderException(method);
    }
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
}

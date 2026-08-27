/// TCP 守护：桌面拓扑的核心侧。模块各自为进程，边界上是 JSON 行封套。
/// 核心只认识封套与声明，不认识任何业务。
library;

import 'dart:async';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';
import 'exchange.dart';

/// rpc 应答：Streamed 则逐块发同 id 的 event，最后 ok 收全量；普通结果直接 ok
Future<void> _replyRpc(Socket s, Envelope env, Object? result) async {
  if (result is Streamed) {
    final buf = StringBuffer();
    await for (final chunk in result.chunks) {
      buf.write(chunk);
      sendEnvelope(
          s,
          Envelope(
              from: 'core',
              to: env.from,
              kind: EnvelopeKind.event,
              id: env.id,
              method: env.method,
              params: chunk));
    }
    sendEnvelope(
        s,
        Envelope(
            from: 'core',
            to: env.from,
            kind: EnvelopeKind.ok,
            id: env.id,
            method: env.method,
            params: buf.toString()));
  } else {
    sendEnvelope(
        s,
        Envelope(
            from: 'core',
            to: env.from,
            kind: EnvelopeKind.ok,
            id: env.id,
            method: env.method,
            params: result));
  }
}

class BrokerDaemon {
  final Exchange _ex = Exchange();
  late final ServerSocket _server;
  final _pending = <String, Completer<Object?>>{}; // 核心向模块的在途 rpc
  final _pendingMeta = <String, Envelope>{}; // 在途 rpc 的原始调用封套（事件转发用）
  final _pendingTarget = <String, String>{}; // 在途 rpc 的目标模块（断开时报错用）
  final _sockets = <String, Socket>{}; // 模块 id -> 连接
  var _seq = 0;

  static Future<BrokerDaemon> bind(InternetAddress address, int port) async {
    final d = BrokerDaemon();
    d._server = await ServerSocket.bind(address, port);
    d._server.listen(d._onConn, onError: (Object e) {});
    return d;
  }

  InternetAddress get address => _server.address;
  int get port => _server.port;

  /// 所有模块 hello 完成后调用：fail-fast 撮合
  Future<void> start({Map<String, Map<String, String>>? explicit}) async {
    _ex.start(explicit: explicit ?? const {});
  }

  Future<void> _onConn(Socket s) async {
    var moduleId = '';
    try {
      await for (final env in envelopeLines(s)) {
      if (env.kind == EnvelopeKind.hello) {
        moduleId = env.from;
        try {
          _ex.connect(Declaration.fromJson(env.params as Map<String, Object?>),
              (e) => _moduleRpc(s, moduleId, e));
        } catch (e) {
          // 拒绝该连接（如重复 id），核心继续服务其它模块：降级不崩溃
          sendEnvelope(
              s,
              Envelope(
                  from: 'core',
                  to: moduleId,
                  kind: EnvelopeKind.err,
                  id: 'ack',
                  method: 'module.hello',
                  params: errorBody(e)));
          await s.close();
          continue;
        }
        _sockets[moduleId] = s;
        // hello-ack：声明已登记，连接方可继续（消除注册竞态）
        sendEnvelope(
            s,
            Envelope(
                from: 'core',
                to: moduleId,
                kind: EnvelopeKind.ok,
                id: 'ack',
                method: 'module.hello',
                params: {'ack': true}));
        continue;
      }
      if (env.kind == EnvelopeKind.rpc) {
        // 元能力：注册表只读列举（事实，非决策）
        if (env.method == CoreCaps.modules) {
          sendEnvelope(
              s,
              Envelope(
                  from: 'core',
                  to: env.from,
                  kind: EnvelopeKind.ok,
                  id: env.id,
                  method: env.method,
                  params: {
                    'modules': [for (final d in _ex.declarations) d.toJson()]
                  }));
          continue;
        }
        // 定向调用：to 是模块 id 时直达该模块（id 全局唯一所以无歧义）
        if (env.to != 'core') {
          try {
            final result = await _ex.rpcTo(env.from, env.to, env.method, env.params,
                requestId: env.id);
            await _replyRpc(s, env, result);
          } catch (e) {
            sendEnvelope(
                s,
                Envelope(
                    from: 'core',
                    to: env.from,
                    kind: EnvelopeKind.err,
                    id: env.id,
                    method: env.method,
                    params: errorBody(e)));
          }
          continue;
        }
        // 消费方按声明接线路由；内建兜底则回 NoProvider 由其回退
        final provider = _ex.providerOfOrNull(env.from, env.method);
        if (provider == null || provider.isEmpty) {
          sendEnvelope(
              s,
              Envelope(
                  from: 'core',
                  to: env.from,
                  kind: EnvelopeKind.err,
                  id: env.id,
                  method: env.method,
                  params: {'code': 'NoProvider', 'method': env.method}));
          continue;
        }
        try {
          final result = await _ex.rpcTo(env.from, provider, env.method,
              env.params,
              requestId: env.id);
          await _replyRpc(s, env, result);
        } catch (e) {
          sendEnvelope(
              s,
              Envelope(
                  from: 'core',
                  to: env.from,
                  kind: EnvelopeKind.err,
                  id: env.id,
                  method: env.method,
                  params: errorBody(e)));
        }
        continue;
      }
      if (env.kind == EnvelopeKind.event) {
        // 模块流式块：转发给原始调用方（按在途 rpc 的元数据路由）
        final meta = _pendingMeta[env.id];
        final target = meta == null ? null : _sockets[meta.from];
        if (meta != null && target != null) {
          sendEnvelope(
              target,
              Envelope(
                  from: 'core',
                  to: meta.from,
                  kind: EnvelopeKind.event,
                  id: meta.id,
                  method: env.method,
                  params: env.params));
        }
        continue;
      }
      if (env.kind == EnvelopeKind.ok || env.kind == EnvelopeKind.err) {
        final done = _pending.remove(env.id);
        if (done == null) continue;
        final meta = _pendingMeta.remove(env.id);
        _pendingTarget.remove(env.id);
        if (env.kind == EnvelopeKind.ok) {
          done.complete(env.params);
        } else {
          try {
            wireError(env.params);
          } catch (e) {
            done.completeError(e);
          }
        }
      }
      }
    } on Object {
      // 连接异常：走 finally 清理
    } finally {
      if (moduleId.isNotEmpty) {
        // 模块离线：注销声明，目标为它的在途 rpc 立即报错（进程重连可重新注册）
        _ex.disconnect(moduleId);
        _sockets.remove(moduleId);
        final dead = <String>[];
        _pendingTarget.forEach((id, target) {
          if (target == moduleId) dead.add(id);
        });
        for (final id in dead) {
          _pending.remove(id)?.completeError(StateError('模块已断开: ' + moduleId));
          _pendingMeta.remove(id);
          _pendingTarget.remove(id);
        }
      }
    }
  }

  Future<Object?> _moduleRpc(Socket s, String moduleId, Envelope e) {
    final id = 'd' + (_seq++).toString();
    final done = Completer<Object?>();
    _pending[id] = done;
    _pendingMeta[id] = e; // 事件转发需要原始调用方
    _pendingTarget[id] = moduleId;
    sendEnvelope(
        s,
        Envelope(
            from: 'core',
            to: moduleId,
            kind: EnvelopeKind.rpc,
            id: id,
            method: e.method,
            params: e.params));
    return done.future;
  }
}

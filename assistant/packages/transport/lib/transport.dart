/// 线路层：模块侧。模块只依赖这里与协议，永不 import 核心。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';

Stream<Envelope> envelopeLines(Socket s) => s
    .cast<List<int>>()
    .transform(utf8.decoder)
    .transform(const LineSplitter())
    .map((l) => Envelope.fromJson(jsonDecode(l) as Map<String, Object?>));

void sendEnvelope(Socket s, Envelope e) => s.writeln(jsonEncode(e.toJson()));

/// 线路错误还原为异常对象：NoProvider 让消费方降级，Candidates 交回选择权
Object wireError(Object? params) {
  final m = params as Map;
  if (m['code'] == 'NoProvider') {
    return NoProviderException(m['method'] as String);
  }
  if (m['code'] == 'Candidates') {
    return CandidatesException(m['method'] as String,
        [for (final c in (m['candidates'] as List)) c as String]);
  }
  return StateError((m['message'] ?? m).toString());
}

Map<String, Object?> errorBody(Object e) {
  if (e is NoProviderException) {
    return {'code': 'NoProvider', 'method': e.method};
  }
  if (e is CandidatesException) {
    return {
      'code': 'Candidates',
      'method': e.method,
      'candidates': e.candidates,
    };
  }
  // StateError 取 message：toString 自带前缀，逐跳转发会层层叠加
  return {
    'code': 'Error',
    'message': e is StateError ? e.message : e.toString()
  };
}

/// 配对快照解码：{需求能力: [提供方...]}（core.wiring 推送的负载）
WiringSnapshot decodeWiringSnapshot(Object? params) => {
      for (final e in (params as Map<String, Object?>).entries)
        e.key: [for (final p in (e.value as List)) p as String],
    };

/// 一次流式调用的收流端
class _StreamReply {
  final controller = StreamController<Object?>();
  bool gotEvents = false;
}

/// 模块侧连接：hello 声明 -> 绑定服务 -> 可发起 rpc
class ModuleClient {
  final Socket socket;
  final String moduleId;
  final _pending = <String, Completer<Object?>>{};
  final _pendingStreams = <String, _StreamReply>{};
  final _wiring = StreamController<WiringSnapshot>.broadcast();
  WiringSnapshot? _lastWiring; // 最近一次快照：订阅晚于首推也不丢（消竞态）
  var _seq = 0;

  ModuleClient._(this.socket, this.moduleId);

  /// 配对快照流：先补发最近快照（若订阅前已到达），之后跟随成员变化
  Stream<WiringSnapshot> get wiring async* {
    final last = _lastWiring;
    if (last != null) yield last;
    yield* _wiring.stream;
  }

  static Future<ModuleClient> connect(
      InternetAddress host, int port, ModuleProgram program) async {
    final s = await Socket.connect(host, port);
    final c = ModuleClient._(s, program.declaration.id);
    sendEnvelope(
        s,
        Envelope(
            from: program.declaration.id,
            to: 'core',
            kind: EnvelopeKind.hello,
            method: 'module.hello',
            params: program.declaration.toJson()));
    final out = _ClientOutbound(c);
    final handler = program.bind(out);
    final helloAck = Completer<void>();
    envelopeLines(s).listen((env) async {
      if (env.kind == EnvelopeKind.ok &&
          env.method == 'module.hello') {
        if (!helloAck.isCompleted) helloAck.complete();
        return;
      }
      if (env.kind == EnvelopeKind.err && env.method == 'module.hello') {
        // 注册被拒（如重复 id）：连接失败由调用方处理
        if (!helloAck.isCompleted) {
          helloAck.completeError(wireError(env.params));
        }
        return;
      }
      if (env.kind == EnvelopeKind.event && env.method == CoreCaps.wiring) {
        // 配对快照推送：成员变化的事实（勿轮询）；留底供迟到订阅者补发
        final snap = decodeWiringSnapshot(env.params);
        c._lastWiring = snap;
        c._wiring.add(snap);
        return;
      }
      if (env.kind == EnvelopeKind.rpc) {
        try {
          final r = await handler(env);
          if (r is Streamed) {
            // 流式结果：逐块发同 id 的 event，ok 收全量
            final buf = StringBuffer();
            await for (final chunk in r.chunks) {
              buf.write(chunk);
              sendEnvelope(
                  s,
                  Envelope(
                      from: c.moduleId,
                      to: 'core',
                      kind: EnvelopeKind.event,
                      id: env.id,
                      method: env.method,
                      params: chunk));
            }
            sendEnvelope(
                s,
                Envelope(
                    from: c.moduleId,
                    to: 'core',
                    kind: EnvelopeKind.ok,
                    id: env.id,
                    method: env.method,
                    params: buf.toString()));
          } else {
            sendEnvelope(
                s,
                Envelope(
                    from: c.moduleId,
                    to: 'core',
                    kind: EnvelopeKind.ok,
                    id: env.id,
                    method: env.method,
                    params: r));
          }
        } catch (e) {
          sendEnvelope(
              s,
              Envelope(
                  from: c.moduleId,
                  to: 'core',
                  kind: EnvelopeKind.err,
                  id: env.id,
                  method: env.method,
                  params: errorBody(e)));
        }
      } else if (env.kind == EnvelopeKind.event) {
        // 流式 rpc 的增量块：按同 id 入流
        final stream = c._pendingStreams[env.id];
        if (stream != null) {
          stream.gotEvents = true;
          stream.controller.add(env.params);
        }
      } else if (env.kind == EnvelopeKind.ok || env.kind == EnvelopeKind.err) {
        // 普通与流式在途都要结算，缺一即漏关流（死锁）
        final done = c._pending.remove(env.id);
        if (done != null) {
          if (env.kind == EnvelopeKind.ok) {
            done.complete(env.params);
          } else {
            done.completeError(wireError(env.params));
          }
        }
        final stream = c._pendingStreams.remove(env.id);
        if (stream != null) {
          if (env.kind == EnvelopeKind.err) {
            stream.controller.addError(wireError(env.params));
          } else if (!stream.gotEvents && env.params != null) {
            // 提供方不流式：ok 全量文本回退为单块
            stream.controller.add(env.params);
          }
          stream.controller.close();
        }
      }
    });
    // 等 hello-ack：核心已登记声明，后续撮合不会漏掉本模块
    await helloAck.future.timeout(const Duration(seconds: 5));
    return c;
  }

  Future<Object?> rpc(String method, Object? params) =>
      _ClientOutbound(this).rpc(method, params);

  /// 流式调用：块经 event 封到达，ok 收尾
  Stream<Object?> rpcStream(String method, Object? params) =>
      _ClientOutbound(this).rpcStream(method, params);
}

final class _ClientOutbound implements Outbound {
  final ModuleClient _c;
  _ClientOutbound(this._c);

  @override
  Future<Object?> rpc(String method, Object? params) =>
      _send('core', method, params);

  @override
  Stream<Object?> rpcStream(String method, Object? params) {
    final id = 'c' + (_c._seq++).toString();
    final reply = _StreamReply();
    _c._pendingStreams[id] = reply;
    sendEnvelope(
        _c.socket,
        Envelope(
            from: _c.moduleId,
            to: 'core',
            kind: EnvelopeKind.rpc,
            id: id,
            method: method,
            params: params));
    return reply.controller.stream;
  }

  @override
  Future<Object?> call(String moduleId, String method, Object? params) =>
      _send(moduleId, method, params);

  @override
  Stream<WiringSnapshot> get wiring => _c.wiring;

  Future<Object?> _send(String to, String method, Object? params) {
    final id = 'c' + (_c._seq++).toString();
    final done = Completer<Object?>();
    _c._pending[id] = done;
    sendEnvelope(
        _c.socket,
        Envelope(
            from: _c.moduleId,
            to: to,
            kind: EnvelopeKind.rpc,
            id: id,
            method: method,
            params: params));
    return done.future;
  }
}

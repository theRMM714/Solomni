/// 动态配对端到端测试：单进程内起真 TCP 拓扑（BrokerDaemon + 多 ModuleClient）。
/// 证明：接入顺序无关、提供方离线即降级、回归即恢复、
/// 配对快照推送到模块侧（勿轮询）、多提供方候选直达调用方。
library;

import 'dart:async';
import 'dart:io';
import 'package:core/core.dart';
import 'package:protocol/protocol.dart';
import 'package:protocol/outbound.dart';
import 'package:transport/transport.dart';

int _passed = 0;

void check(String name, bool cond) {
  if (!cond) {
    throw StateError('FAILED: ' + name);
  }
  _passed++;
  print('PASS: ' + name);
}

/// TCP 事件（ack、快照推送、断开检测）异步到达：留时间冲刷
Future<void> settle() => Future<void>.delayed(const Duration(milliseconds: 300));

final class _App implements ModuleProgram {
  final String id;
  final List<Provide> provides;
  final List<Need> needs;
  Outbound? out; // bind 时由线路层注入，测试用它发起调用与订阅快照
  _App(this.id,
      {this.provides = const [], this.needs = const []});

  @override
  Declaration get declaration =>
      Declaration(id, provides: provides, needs: needs);

  @override
  ModuleHandler bind(Outbound outbound) {
    out = outbound;
    return (env) async => {'to': env.to}; // 回显路由目标，供断言
  }
}

Future<void> main() async {
  final daemon = await BrokerDaemon.bind(InternetAddress.loopbackIPv4, 0);
  await daemon.start();

  // 1. 消费方先接入（此刻无任何提供方）：声明到达即配对，集合空
  final consumer = _App('conversation',
      needs: [Need(Caps.llmChat, NeedVia.preferShared)]);
  await ModuleClient.connect(daemon.address, daemon.port, consumer);
  var degraded = false;
  try {
    await consumer.out!.rpc(Caps.llmChat, null);
  } on NoProviderException {
    degraded = true;
  }
  check('消费方先接入，无提供方 -> NoProvider（降级信号）', degraded);

  // 2. 提供方晚接入：接入顺序无关，消费方自动接上
  final gw = _App('llm_gateway', provides: [Provide(Caps.llmChat)]);
  final gwClient =
      await ModuleClient.connect(daemon.address, daemon.port, gw);
  final r = await consumer.out!.rpc(Caps.llmChat, null);
  check('提供方晚接入 -> 消费方自动接上（顺序无关）',
      (r as Map)['to'] == 'llm_gateway');

  // 3. 配对快照推送：订阅后触发成员变化，快照应到达模块侧
  final snapshots = <Map<String, List<String>>>[];
  consumer.out!.wiring.listen(snapshots.add);
  await ModuleClient.connect(
      daemon.address, daemon.port, _App('secrets'));
  await settle();
  check('成员变化 -> 消费方收到快照推送（勿轮询）', snapshots.isNotEmpty);
  check('快照内容：需求 -> 提供方集合',
      snapshots.any((s) => (s[Caps.llmChat] ?? const []).contains('llm_gateway')));

  // 4. 提供方离线：消费方集合收缩，调用立即降级
  await gwClient.socket.close();
  await settle();
  var degraded2 = false;
  try {
    await consumer.out!.rpc(Caps.llmChat, null);
  } on NoProviderException {
    degraded2 = true;
  }
  check('提供方离线 -> 消费方降级（NoProvider）', degraded2);
  check('离线后快照更新为空集',
      snapshots.isNotEmpty && (snapshots.last[Caps.llmChat] ?? const []).isEmpty);

  // 5. 提供方回归：配对自动恢复
  await ModuleClient.connect(daemon.address, daemon.port,
      _App('llm_gateway', provides: [Provide(Caps.llmChat)]));
  final r2 = await consumer.out!.rpc(Caps.llmChat, null);
  check('提供方回归 -> 自动恢复', (r2 as Map)['to'] == 'llm_gateway');

  // 6. 第二个提供方接入：诚实集合（不拒收）；无表态 -> Candidates 直达调用方
  await ModuleClient.connect(
      daemon.address, daemon.port, _App('llm_b', provides: [Provide(Caps.llmChat)]));
  var cand = const <String>[];
  try {
    await consumer.out!.rpc(Caps.llmChat, null);
  } on CandidatesException catch (e) {
    cand = e.candidates;
  }
  check('多提供方 -> CandidatesException 直达调用方（附清单）',
      cand.contains('llm_gateway') && cand.contains('llm_b') && cand.length == 2);

  print('');
  print('全部通过: ' + _passed.toString() + ' 项');
  exit(0);
}

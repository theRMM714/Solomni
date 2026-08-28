/// 出边词汇：模块发送侧 + 模块程序形状。属于字典：接口与异常，不含实现。
library;

import 'protocol.dart';

/// 配对快照：模块某需求 -> 当前全体提供方（事实集合，非决策）
typedef WiringSnapshot = Map<String, List<String>>;

/// 模块发消息出边。能力地址由当前配对路由（成员变化即重算）
abstract interface class Outbound {
  /// 业务调用：显式接线 > 声明首选 > 唯一提供方直达；
  /// 多提供方且无表态抛 Candidates（选择权交回你）；无提供方抛 NoProvider（按声明降级）
  Future<Object?> rpc(String method, Object? params);

  /// 流式业务调用：token 经 event 封到达；提供方不流式时回退单块
  Stream<Object?> rpcStream(String method, Object? params);

  /// 定向调用：按模块 id 直达（发现/事件路由/多提供方选择用；id 全局唯一所以无歧义）
  Future<Object?> call(String moduleId, String method, Object? params);

  /// 配对快照流：本模块各需求 -> 当前提供方集合；成员变化时核心推送（勿轮询）
  Stream<WiringSnapshot> get wiring;
}

/// 能力无共享提供方：消费方应按声明策略降级
/// （preferShared 回内建实现，sharedOnly 功能降级）
class NoProviderException implements Exception {
  final String method;
  NoProviderException(this.method);
  @override
  String toString() => 'NoProviderException: ' + method;
}

/// 能力有多个提供方且消费方未表态（无显式接线、无声明首选）：
/// 核心绝不代选，附候选清单把选择权交回调用方
class CandidatesException implements Exception {
  final String method;
  final List<String> candidates;
  CandidatesException(this.method, this.candidates);
  @override
  String toString() =>
      'CandidatesException: ' + method + ' -> ' + candidates.join(', ');
}

/// 模块处理器：收到封套，返回应答参数（纯数据）
typedef ModuleHandler = Future<Object?> Function(Envelope env);

/// 模块程序：一个模块作为自治程序对外的全部
abstract interface class ModuleProgram {
  Declaration get declaration;
  ModuleHandler bind(Outbound outbound);
}

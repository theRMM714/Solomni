/// 出边词汇：模块发送侧 + 模块程序形状。属于字典：接口与异常，不含实现。
library;

import 'protocol.dart';

/// 模块发消息出边。能力地址由装配环境解析；无共享提供方（内建兜底）时抛 NoProvider
abstract interface class Outbound {
  /// 业务调用：按声明接线路由，无提供方抛 NoProvider
  Future<Object?> rpc(String method, Object? params);

  /// 流式业务调用：token 经 event 封到达；提供方不流式时回退单块
  Stream<Object?> rpcStream(String method, Object? params);

  /// 定向调用：按模块 id 直达（发现/事件路由用；id 全局唯一所以无歧义）
  Future<Object?> call(String moduleId, String method, Object? params);
}

/// 能力无共享提供方：消费方应回退内建实现
class NoProviderException implements Exception {
  final String method;
  NoProviderException(this.method);
  @override
  String toString() => 'NoProviderException: ' + method;
}

/// 模块处理器：收到封套，返回应答参数（纯数据）
typedef ModuleHandler = Future<Object?> Function(Envelope env);

/// 模块程序：一个模块作为自治程序对外的全部
abstract interface class ModuleProgram {
  Declaration get declaration;
  ModuleHandler bind(Outbound outbound);
}

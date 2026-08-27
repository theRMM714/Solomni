/// 单机入口：打印网关配置。真调用需密钥与网络，属协作拓扑。
library;

import 'package:llm_gateway/llm_gateway.dart';

Future<void> main() async {
  final p = LlmGatewayProgram();
  print('[llm_gateway] baseUrl=' + p.baseUrl + ' model=' + p.model);
  print('单机模式无核心；真 AI 对话经协作拓扑（核心 + secrets 提供密钥）进行。');
}

/// 协作入口：模块守护进程，连到核心。
library;

import 'dart:async';
import 'dart:io';
import 'package:transport/transport.dart';
import 'package:llm_gateway/llm_gateway.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  await ModuleClient.connect(
      InternetAddress.loopbackIPv4, port, LlmGatewayProgram());
  print('[llm_gateway] 已连接核心 :' + port.toString() + '，等待调用');
  // 守护常驻：连接断开（核心退出）才会结束
  await Completer<void>().future;
}

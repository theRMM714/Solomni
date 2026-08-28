/// 协作入口：模块守护进程，连到核心。用法：dart bin/coordinated.dart --port 9100
library;

import 'dart:io';
import 'package:transport/transport.dart';
import 'package:conversation/conversation.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  await ModuleClient.connect(
      InternetAddress.loopbackIPv4, port, ConversationProgram());
  print('[conversation] 已连接核心 :' + port.toString() + '，等待调用');
}

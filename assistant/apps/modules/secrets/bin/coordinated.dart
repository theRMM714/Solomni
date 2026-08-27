/// 协作入口：模块守护进程，连到核心。
library;

import 'dart:io';
import 'package:transport/transport.dart';
import 'package:secrets/secrets.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, SecretsProgram());
  print('[secrets] 已连接核心 :' + port.toString() + '，等待调用');
}

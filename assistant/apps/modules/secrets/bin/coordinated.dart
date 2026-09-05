/// 协作入口：模块守护进程，连到核心。
library;

import 'dart:async';
import 'dart:io';
import 'package:transport/transport.dart';
import 'package:secrets/secrets.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  // 就绪由宿主同步播报，这里只在连接前报一次启动事实（避免异步输出打断宿主菜单）
  print('[secrets] 连接核心 :' + port.toString() + '…');
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, SecretsProgram());
  // 守护常驻：连接断开（核心退出）才会结束
  await Completer<void>().future;
}

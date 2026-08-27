/// 协作入口：Flutter UI 模块连到核心（--port=9100），自己一个窗口。
/// 运行：flutter run -t bin/coordinated.dart -d windows
library;

import 'dart:io';
import 'package:flutter/material.dart';
import 'package:transport/transport.dart';
import 'package:ui_flutter/ui_flutter.dart';
import 'package:ui_flutter/widgets/shell.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  for (final a in args) {
    if (a.startsWith('--port=')) port = int.parse(a.substring(7));
  }
  WidgetsFlutterBinding.ensureInitialized();
  final program = UiFlutterProgram();
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, program);
  runApp(UiShell(outbound: program.outbound));
}

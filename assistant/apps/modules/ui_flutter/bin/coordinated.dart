/// 协作入口：Flutter UI 模块连到核心（--port=9100），自己一个窗口。
/// 运行：flutter run -t bin/coordinated.dart -d windows
library;

import 'dart:io';
import 'package:flutter/material.dart';import 'package:transport/transport.dart';
import 'package:ui_flutter/ui_flutter.dart';
import 'package:ui_flutter/widgets/shell.dart';

Future<void> main(List<String> args) async {
  // 参数双形态：--port=N 与 --port N（Windows 的 cmd 批参数以 = 分隔，
  // dev.bat 侧会把 --port=N 拆成两个参数送进来）
  var port = 9100;
  for (var i = 0; i < args.length; i++) {
    if (args[i] == '--port' && i + 1 < args.length) {
      port = int.parse(args[i + 1]);
    } else if (args[i].startsWith('--port=')) {
      port = int.parse(args[i].substring(7));
    }
  }
  WidgetsFlutterBinding.ensureInitialized();
  final program = UiFlutterProgram();
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, program);
  runApp(UiShell(outbound: program.outbound));
}

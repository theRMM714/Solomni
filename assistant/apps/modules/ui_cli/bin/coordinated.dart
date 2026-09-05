/// 协作入口：终端 UI 模块守护进程，连到核心。
/// 双形态：
///   无 --repl：无头守护（提供 ui.render / ui.command，供脚本与验收驱动）
///   带 --repl：交互面（接管当前终端跑 REPL，exit 只结束本进程）
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:transport/transport.dart';
import 'package:ui_cli/ui_cli.dart';

Future<void> main(List<String> args) async {
  var port = 9100;
  var repl = false;
  // 参数双形态：--port=N 与 --port N（Windows 的 cmd 批参数以 = 分隔）
  for (var i = 0; i < args.length; i++) {
    if (args[i] == '--port' && i + 1 < args.length) {
      port = int.parse(args[i + 1]);
    } else if (args[i].startsWith('--port=')) {
      port = int.parse(args[i].substring(7));
    } else if (args[i] == '--repl') {
      repl = true;
    }
  }
  // 就绪由宿主同步播报，这里只在连接前报一次启动事实（避免异步输出打断宿主菜单）
  if (!repl) print('[ui_cli] 连接核心 :' + port.toString() + '…');
  final program = UiCliProgram();
  await ModuleClient.connect(InternetAddress.loopbackIPv4, port, program);

  if (!repl) {
    // 无头守护常驻：连接断开（核心退出）才会结束
    await Completer<void>().future;
    return;
  }

  // surface 模式：终端交互面归本模块（help 走 ui.render，其余整行交 ui.command）
  final out = program.outbound!;
  stdout.writeln('（终端对话模式；help 查看组件与命令，exit 返回）');
  final lines = stdin
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .map((l) => l.trim());
  await for (final line in lines) {
    if (line.isEmpty) continue;
    if (line == 'exit' || line == 'quit') break;
    try {
      if (line == 'help') {
        stdout.writeln(await program.render(out));
        continue;
      }
      final sp = line.indexOf(' ');
      final name = sp < 0 ? line : line.substring(0, sp);
      final rest = sp < 0
          ? <String>[]
          : line.substring(sp + 1).trim().split(RegExp(r'\s+'));
      stdout.writeln(
          await program.command(out, {'name': name, 'args': rest}));
    } catch (e) {
      // 模块层的拒绝与错误照实显示：降级展示，不崩溃（退出只归 exit/Ctrl+C）
      stdout.writeln('[错误] ' + e.toString());
    }
  }
  // surface 退出：仅结束本进程，核心与产品宿主继续
  exit(0);
}

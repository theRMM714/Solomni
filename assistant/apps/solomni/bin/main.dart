/// 产品入口（规范见 PRODUCT.md）。
/// 默认终端 REPL；-gui 额外拉起一个外部 UI 模块进程作为并行交互面；
/// 终端退出=产品消亡，GUI 关窗=模块离线（产品继续）。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:solomni/assembler.dart';

/// 拉起外部模块进程：守护入口 bin/coordinated.dart，端口由产品注入。
/// Flutter 模块经仓库内包装命令（开发态；PATH 的 flutter 兜底）；
/// 纯 Dart 模块走当前 VM。Windows 的 .bat 必须经 cmd.exe 执行。
Future<Process> _spawnModule(ModuleFolder f, int port) async {
  final pubspec = File(f.path + '/pubspec.yaml');
  final isFlutter = pubspec.existsSync() &&
      pubspec.readAsLinesSync().any((l) => l.startsWith('flutter:'));
  if (isFlutter) {
    final device = Platform.isLinux
        ? 'linux'
        : Platform.isWindows
            ? 'windows'
            : 'macos';
    final args = [
      'run', '-t', 'bin/coordinated.dart', '-d', device, '--', '--port=' + port.toString(),
    ];
    final sep = Platform.isWindows ? r'\' : '/';
    final wrapper = repoRoot() + sep + 'bin' + sep +
        (Platform.isWindows ? 'flutter.bat' : 'flutter');
    if (File(wrapper).existsSync()) {
      // 仓库包装命令：与产品同一套 SDK/缓存环境
      if (Platform.isWindows) {
        return Process.start('cmd.exe', ['/c', wrapper, ...args],
            workingDirectory: f.path,
            mode: ProcessStartMode.inheritStdio);
      }
      return Process.start(wrapper, args,
          workingDirectory: f.path, mode: ProcessStartMode.inheritStdio);
    }
    return Process.start('flutter', args,
        workingDirectory: f.path, mode: ProcessStartMode.inheritStdio);
  }
  return Process.start(Platform.resolvedExecutable, [
    'bin/coordinated.dart', '--port=' + port.toString(),
  ], workingDirectory: f.path, mode: ProcessStartMode.inheritStdio);
}

/// 终止外部模块进程：Windows 下 cmd 的子进程不随父死，须杀进程树
Future<void> _killProcess(Process p) async {
  if (Platform.isWindows) {
    await Process.run('taskkill', ['/PID', p.pid.toString(), '/T', '/F']);
  } else {
    p.kill();
  }
}

Future<void> main(List<String> args) async {
  final excluded = args
      .where((a) => a.startsWith('--without='))
      .map((a) => a.substring(10))
      .toSet();
  final serve = args.contains('--serve');
  final verbose = args.contains('--verbose');
  var port = 0;
  String? guiId;
  var guiRequested = false;
  for (final a in args) {
    if (a == '-gui' || a == '--gui') {
      guiRequested = true;
    } else if (a.startsWith('-gui=')) {
      guiRequested = true;
      guiId = a.substring(5);
    } else if (a.startsWith('--gui=')) {
      guiRequested = true;
      guiId = a.substring(6);
    } else if (a.startsWith('--port=')) {
      port = int.parse(a.substring(7));
    }
  }

  final asm = await assemble(excluded: excluded, port: port, verbose: verbose);
  final daemon = asm.daemon;
  final procs = <Process>[];

  // -gui：额外拉起外部 UI 模块（装配层也无静默选择）
  if (guiRequested) {
    final candidates = asm.spawnable;
    if (candidates.isEmpty) {
      stderr.writeln('[失败] 没有可拉起的外部模块（modules/ 下无未编译模块）');
      exit(2);
    }
    ModuleFolder? chosen;
    if (guiId == null) {
      if (candidates.length == 1) chosen = candidates.single;
    } else {
      for (final f in candidates) {
        if (f.id == guiId) chosen = f;
      }
    }
    if (chosen == null) {
      if (guiId == null) {
        stderr.writeln('[失败] 多个外部模块，请指定：');
        for (final f in candidates) {
          stderr.writeln('  -gui ' + f.id);
        }
      } else {
        stderr.writeln('[失败] 未找到外部模块: ' + guiId + '（候选: ' +
            candidates.map((f) => f.id).join(', ') + '）');
      }
      exit(2);
    }
    try {
      procs.add(await _spawnModule(chosen, daemon.port));
    } catch (e) {
      // 拉起失败（如 flutter 不在 PATH）：fail-fast 并指明原因
      stderr.writeln('[失败] 拉起 ' + chosen.id + ' 失败: ' + e.toString());
      exit(2);
    }
    print('[拉起] ' + chosen.id + ' -> :' + daemon.port.toString());
  }

  // Ctrl+C：产品进程消亡，外部模块进程随之终止
  unawaited(ProcessSignal.sigint.watch().first.then((_) async {
    for (final p in procs) {
      await _killProcess(p);
    }
    exit(130);
  }));

  if (serve) {
    print('[serve] 核心常驻 :' + daemon.port.toString() + '（Ctrl+C 退出；无 REPL）');
    await Completer<void>().future; // 常驻
  }

  await _repl(asm, procs);
}

/// 终端 REPL：产品宿主的交互面（兜底交互面，恒在）。
/// help 走 ui.render（模块能力）；exit 是产品生命周期；其余整行交 ui.command 路由。
Future<void> _repl(Assembled asm, List<Process> procs) async {
  final driver = asm.driver;
  stdout.writeln('（终端对话模式；help 查看组件与命令，exit 退出）');
  final lines = stdin
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .map((l) => l.trim());
  await for (final line in lines) {
    if (line.isEmpty) continue;
    if (line == 'exit' || line == 'quit') break;
    try {
      if (line == 'help') {
        stdout.writeln(await driver.rpc('ui.render', null));
        continue;
      }
      final sp = line.indexOf(' ');
      final name = sp < 0 ? line : line.substring(0, sp);
      final rest = sp < 0
          ? <String>[]
          : line.substring(sp + 1).trim().split(RegExp(r'\s+'));
      stdout.writeln(
          await driver.rpc('ui.command', {'name': name, 'args': rest}));
    } catch (e) {
      // 模块层的拒绝与错误照实显示：降级展示，不崩溃（退出只归 exit/Ctrl+C）
      stdout.writeln('[错误] ' + e.toString());
    }
  }
  for (final p in procs) {
    await _killProcess(p);
  }
  exit(0);
}

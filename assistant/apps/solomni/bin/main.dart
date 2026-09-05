/// 核心宿主入口：拉核心 + 拉起 service + 极简菜单（唤起 surface）。
/// 产品入口（bin/solomni）只剩薄壳；终端交互面是 ui_cli 模块，不是产品。
/// 菜单只允许唤起界面类模块；exit 是产品生命周期（杀全部子进程）。
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:protocol/protocol.dart';
import 'package:solomni/assembler.dart';

void _printSurfaces(Assembled asm) {
  final s = asm.surfaces;
  if (s.isEmpty) {
    stdout.writeln('（没有可唤起的界面模块）');
  } else {
    stdout.writeln('可唤起界面：');
    for (final f in s) {
      stdout.writeln('  ' + f.id);
    }
  }
}

/// 当前在线模块（核心注册表事实）：surface 退出重进后据此判断 service 是否还在
void _printOnline(Assembled asm) {
  stdout.writeln('[在线] ' +
      asm.daemon.declarations.map((d) => d.id).join(', '));
}

/// 唤起一个 surface：spawn -> 校验自声明 kind == surface -> 附着终端等它退出。
/// 返回 false 表示未能唤起（未知 id / 自声明不是界面）。
Future<bool> _launchSurface(
    Assembled asm, List<Process> procs, String? id) async {
  final candidates = asm.surfaces;
  ModuleFolder? chosen;
  if (id == null) {
    if (candidates.length == 1) chosen = candidates.single;
  } else {
    for (final f in candidates) {
      if (f.id == id) chosen = f;
    }
  }
  if (chosen == null) {
    if (id == null && candidates.isEmpty) {
      stderr.writeln('[失败] 没有可唤起的界面模块');
    } else if (id == null) {
      stderr.writeln('[提示] 多个候选，请指定：-gui <id>（' +
          candidates.map((f) => f.id).join(', ') + '）');
    } else {
      stderr.writeln('[失败] 未找到界面模块: ' + id +
          '（候选: ' + candidates.map((f) => f.id).join(', ') + '）');
    }
    return false;
  }
  final proc = await spawnSurface(chosen, asm.daemon.port);
  procs.add(proc);
  print('[唤起] ' + chosen.id + ' -> :' + asm.daemon.port.toString());
  // 自声明校验：等它的 hello 到达，kind 必须是 surface（模块自报身份，宿主照实校验）
  final decl = await asm.waitDeclaration(chosen.id,
      timeout: const Duration(seconds: 5));
  if (decl != null && decl.kind != ModuleKind.surface) {
    stderr.writeln(
        '[拒绝] ' + chosen.id + ' 自声明为 ' + decl.kind.name + '，不是界面模块');
    await killProcess(proc);
    return false;
  }
  // 附着：等它退出（关窗 / 终端 exit），宿主回到菜单
  await proc.exitCode;
  return true;
}

/// 单次订阅 stdin 的行读取器：surface 运行期间可 pause（把终端让给 surface）。
/// stdin 是单订阅流，只能 listen 一次，故这里只建一个订阅 + 手动队列。
class _LineReader {
  final _queue = <String>[];
  final _waiters = <Completer<String?>>[];
  bool _eof = false;
  late final StreamSubscription<String> _sub;

  _LineReader() {
    _sub = stdin
        .transform(utf8.decoder)
        .transform(const LineSplitter())
        .listen((l) => _deliver(l.trim()),
            onDone: () {
              _eof = true;
              _flush();
            },
            onError: (Object e) => _deliverError(e));
  }

  void _deliver(String l) {
    if (_waiters.isNotEmpty) {
      _waiters.removeAt(0).complete(l);
    } else {
      _queue.add(l);
    }
  }

  void _deliverError(Object e) {
    if (_waiters.isNotEmpty) _waiters.removeAt(0).completeError(e);
  }

  void _flush() {
    for (final w in _waiters) {
      w.complete(null);
    }
    _waiters.clear();
  }

  Future<String?> next() {
    if (_queue.isNotEmpty) return Future.value(_queue.removeAt(0));
    if (_eof) return Future.value(null);
    final c = Completer<String?>();
    _waiters.add(c);
    return c.future;
  }

  void pause() => _sub.pause();
  void resume() => _sub.resume();
  void cancel() => _sub.cancel();
}

Future<void> _menu(Assembled asm, List<Process> procs) async {
  final reader = _LineReader();
  stdout.writeln('（核心宿主菜单；输入界面 id 唤起，exit 退出产品）');
  try {
    while (true) {
      final line = await reader.next();
      if (line == null) break; // EOF
      if (line.isEmpty) continue;
      if (line == 'exit' || line == 'quit') break;
      if (line == 'help' || line == 'ls' || line == 'list') {
        _printSurfaces(asm);
        continue;
      }
      if (!asm.surfaces.any((f) => f.id == line)) {
        stdout.writeln('[未知] 没有界面模块 "' + line + '"，输入 help 看列表');
        continue;
      }
      reader.pause(); // 把终端让给 surface
      await _launchSurface(asm, procs, line);
      reader.resume();
      _printOnline(asm); // surface 退出后：在线模块事实（诊断 service 是否离线）
      _printSurfaces(asm);
    }
  } finally {
    reader.cancel();
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
  String? surfaceId;
  var surfaceRequested = false;
  for (var i = 0; i < args.length; i++) {
    final a = args[i];
    if (a == '-gui' || a == '--gui') {
      surfaceRequested = true;
      // 分离形式：-gui <id>（下一个非选项参数作为界面 id）
      if (i + 1 < args.length && !args[i + 1].startsWith('-')) {
        surfaceId = args[i + 1];
        i++;
      }
    } else if (a.startsWith('-gui=')) {
      surfaceRequested = true;
      surfaceId = a.substring(5);
    } else if (a.startsWith('--gui=')) {
      surfaceRequested = true;
      surfaceId = a.substring(6);
    } else if (a.startsWith('--port=')) {
      port = int.parse(a.substring(7));
    } else if (a.startsWith('--without=') || a == '--serve' || a == '--verbose') {
      // 已消费（excluded / serve / verbose）
    } else {
      stderr.writeln('[未知参数] ' + a + '（忽略）');
    }
  }

  final asm = await assemble(excluded: excluded, port: port, verbose: verbose);
  await asm.waitServices();
  // 就绪由宿主同步播报（子进程不再异步打印，避免打断菜单处的输入行）
  if (asm.serviceFolders.isNotEmpty) {
    print('[就绪] service ' + asm.serviceFolders.map((f) => f.id).join(', '));
  }
  final procs = <Process>[...asm.services];

  // service 进程退出必须大声可见（离线=降级，但悄悄死掉没法诊断）
  for (var i = 0; i < asm.services.length; i++) {
    final id = asm.serviceFolders[i].id;
    unawaited(asm.services[i].exitCode.then((code) {
      stdout.writeln('[离线] service ' + id + ' 已退出（exit ' +
          code.toString() + '）——消费方自动降级；重启产品可恢复');
    }));
  }

  // Ctrl+C：宿主消亡，所有子模块进程随之终止
  unawaited(ProcessSignal.sigint.watch().first.then((_) async {
    for (final p in procs) {
      await killProcess(p);
    }
    exit(130);
  }));

  if (serve) {
    print('[serve] 核心常驻 :' + asm.daemon.port.toString() +
        '（Ctrl+C 退出；无交互面）');
    await Completer<void>().future; // 常驻
  }

  _printSurfaces(asm);
  if (surfaceRequested) {
    await _launchSurface(asm, procs, surfaceId);
    _printOnline(asm);
    _printSurfaces(asm);
  }
  await _menu(asm, procs);

  for (final p in procs) {
    await killProcess(p);
  }
  exit(0);
}

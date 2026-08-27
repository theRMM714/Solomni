#!/usr/bin/env node
// bin/setup.mjs —— Solomni 依赖安装脚本（Node 18+，跨平台，clone 后运行一次）
//
// 做什么：
//   1. 若仓库根缺 SDK，则按 OS 下载并解压 Dart SDK（→ dart-sdk/）。
//      --flutter：同时下载 Flutter SDK（→ flutter/）。
//   2. 建好仓库内缓存目录 .pub-cache / .dart-home / .tmp。
//   3. 对 assistant/ 下每个含 pubspec.yaml 的包运行 dart pub get。
// 之后开发用 bin/dart、bin/flutter 包装命令，内部自动配好缓存，无需改环境。
//
// 环境变量覆盖：DART_VERSION、FLUTTER_VERSION、DART_BASE、FLUTTER_BASE
// 参数：--flutter（一并装 Flutter）、--force（存在也强制重装）、--help

import { mkdirSync, createWriteStream, existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const BIN = dirname(fileURLToPath(import.meta.url));
const ROOT = join(BIN, '..');

// ---------- 版本 / 镜像（默认国内 flutter-io 源） ----------
const DART_VER  = process.env.DART_VERSION   || '3.12.2';
const FLUT_VER  = process.env.FLUTTER_VERSION || '3.38.2';
const DART_BASE = process.env.DART_BASE  || 'https://storage.flutter-io.cn/dart-archive/channels/stable/release/';
const FLUT_BASE = process.env.FLUT_BASE  || 'https://storage.flutter-io.cn/flutter_infra_release/releases/stable/';

const WITH_FLUTTER = process.argv.includes('--flutter');
const FORCE        = process.argv.includes('--force');
const HELP         = process.argv.includes('--help');

const DART_DIR = join(ROOT, 'dart-sdk', 'dart-sdk');
const FLUT_DIR = join(ROOT, 'flutter', 'flutter');
const DART_BIN = join(DART_DIR, 'bin', process.platform === 'win32' ? 'dart.exe' : 'dart');
const FLUT_BIN = join(FLUT_DIR, 'bin', process.platform === 'win32' ? 'flutter.bat' : 'flutter');
const PUB_CACHE = join(ROOT, '.pub-cache');
const DART_HOME = join(ROOT, '.dart-home');
const TMP       = join(ROOT, '.tmp');

const dartPlatform = function () { return process.platform === 'win32' ? 'windows' : process.platform === 'darwin' ? 'macos' : 'linux'; };
const arch = function () { return process.arch === 'arm64' ? 'arm64' : 'x64'; };
const dartUrl = function () { return DART_BASE + DART_VER + '/sdk/dartsdk-' + DART_VER + '-' + dartPlatform() + '-' + arch() + '.zip'; };
const flutterExt = function () { return process.platform === 'linux' ? 'tar.xz' : 'zip'; };
const flutterUrl = function () { return FLUT_BASE + 'flutter_' + dartPlatform() + '_' + FLUT_VER + '-stable.' + flutterExt(); };

function log(msg) { console.log('[setup] ' + msg); }

function exec(cmd, args, opts) {
  opts = opts || {};
  const r = spawnSync(cmd, args, { stdio: 'inherit', env: Object.assign({}, process.env, opts.env || {}), cwd: opts.cwd });
  if (r.status !== 0) throw new Error('命令失败(exit ' + r.status + ')：' + cmd + ' ' + args.join(' '));
}

async function download(url, dest) {
  log('下载 ' + url);
  const res = await fetch(url, { redirect: 'follow' });
  if (!res.ok) throw new Error('HTTP ' + res.status + ' ' + res.statusText + ' —— ' + url);
  const total = Number(res.headers.get('content-length') || 0);
  const reader = res.body.getReader();
  const ws = createWriteStream(dest);
  let got = 0; const mb = 1048576;
  for (;;) {
    const step = await reader.read();
    if (step.done) break;
    ws.write(step.value); got += step.value.byteLength;
    if (total) process.stdout.write('\r  ' + (got / mb).toFixed(1) + 'MB / ' + (total / mb).toFixed(1) + 'MB');
  }
  await new Promise(function (ok, bad) { ws.end(function (e) { return e ? bad(e) : ok(); }); });
  process.stdout.write('\n');
}

function extract(archive, destDir, kind) {
  log('解压 ' + relative(ROOT, archive) + ' -> ' + relative(ROOT, destDir));
  mkdirSync(destDir, { recursive: true });
  if (kind === 'zip') {
    if (process.platform === 'win32') {
      exec('powershell', ['-NoProfile', '-Command', "Expand-Archive -LiteralPath '" + archive + "' -DestinationPath '" + destDir + "' -Force"]);
    } else if (process.platform === 'darwin') {
      exec('unzip', ['-q', '-o', archive, '-d', destDir]);
    } else {
      const u = spawnSync('unzip', ['-q', '-o', archive, '-d', destDir], { stdio: 'inherit' });
      if (u.status !== 0) exec('tar', ['-xf', archive, '-C', destDir]);
    }
  } else { // tar.xz（Linux Flutter）
    exec('tar', ['-xJf', archive, '-C', destDir]);
  }
}

async function ensureDart() {
  if (!FORCE && existsSync(DART_BIN)) { log('Dart SDK 已就绪：' + relative(ROOT, DART_BIN)); return; }
  const tmp = join(TMP, 'dartsdk.zip');
  mkdirSync(TMP, { recursive: true });
  await download(dartUrl(), tmp);
  extract(tmp, ROOT, 'zip');
  if (!existsSync(DART_BIN)) throw new Error('解压后未找到 ' + relative(ROOT, DART_BIN));
  log('Dart SDK 安装完成：v' + DART_VER);
}

async function ensureFlutter() {
  if (!WITH_FLUTTER) return;
  if (!FORCE && existsSync(FLUT_BIN)) { log('Flutter SDK 已就绪：' + relative(ROOT, FLUT_BIN)); return; }
  const ext = flutterExt();
  const tmp = join(TMP, 'flutter.' + ext);
  mkdirSync(TMP, { recursive: true });
  await download(flutterUrl(), tmp);
  extract(tmp, ROOT, ext);
  if (!existsSync(FLUT_BIN)) throw new Error('解压后未找到 ' + relative(ROOT, FLUT_BIN));
  log('Flutter SDK 安装完成：v' + FLUT_VER);
}

function collectPackageDirs(base) {
  const out = [];
  const walk = function (dir) {
    if (!existsSync(dir)) return;
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (['.dart_tool', 'build', '.git', '.pub-cache'].includes(e.name)) continue;
      const p = join(dir, e.name);
      if (e.isDirectory()) { if (existsSync(join(p, 'pubspec.yaml'))) out.push(p); walk(p); }
    }
  };
  walk(base);
  return out;
}

function needsFlutter(dir) {
  try { return /\bsdk:\s*flutter\b/.test(readFileSync(join(dir, 'pubspec.yaml'), 'utf8')); } catch (e) { return false; }
}

function pubEnv() { return { PUB_CACHE, TMP, TEMP: TMP, APPDATA: DART_HOME, LOCALAPPDATA: join(DART_HOME, 'local') }; }

async function main() {
  if (HELP) {
    console.log('用法：node bin/setup.mjs [--flutter] [--force]');
    console.log('  安装 Dart SDK(+可选 Flutter) 到本仓库并执行 pub get。');
    console.log('  环境变量：DART_VERSION / FLUTTER_VERSION / DART_BASE / FLUTTER_BASE');
    return;
  }
  mkdirSync(PUB_CACHE, { recursive: true });
  mkdirSync(DART_HOME, { recursive: true });
  mkdirSync(TMP, { recursive: true });

  await ensureDart();
  if (WITH_FLUTTER) await ensureFlutter();

  if (!existsSync(DART_BIN)) throw new Error('Dart SDK 不可用：' + relative(ROOT, DART_BIN) + ' —— 请先运行 setup');
  const pkgs = collectPackageDirs(join(ROOT, 'assistant'));
  const dartPkgs = pkgs.filter(function (p) { return !needsFlutter(p); });
  const flutterPkgs = pkgs.filter(function (p) { return needsFlutter(p); });
  log('共 ' + pkgs.length + ' 个 package（Dart ' + dartPkgs.length + '，Flutter ' + flutterPkgs.length + '）');
  for (const p of dartPkgs) {
    log('dart pub get: ' + relative(ROOT, p));
    exec(DART_BIN, ['pub', 'get'], { env: pubEnv(), cwd: p });
  }
  if (WITH_FLUTTER) {
    for (const p of flutterPkgs) {
      log('flutter pub get: ' + relative(ROOT, p));
      exec(FLUT_BIN, ['pub', 'get'], { env: pubEnv(), cwd: p });
    }
  } else if (flutterPkgs.length) {
    log('跳过 ' + flutterPkgs.length + ' 个 Flutter 依赖包（加 --flutter 处理）：' + flutterPkgs.map(function (p) { return relative(ROOT, p); }).join(', '));
  }

  console.log('');
  console.log('[setup] 完成。开发请用仓库内包装命令（自动配好环境，无需手改）：');
  console.log('  Windows:  bin\dart.bat  /  bin\flutter.bat');
  console.log('  WSL/Linux: ./bin/dart  /  ./bin/flutter');
  console.log('详见 SETUP.md。');
}

main().catch(function (e) { console.error('\n[setup] 失败：' + e.message); process.exit(1); });

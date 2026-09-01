#!/usr/bin/env node
// Solomni 产品启动器（bin/solomni 与 bin/solomni.bat 的共用实现）。
// 只做一件事：等价于 cd assistant/apps/solomni && dart bin/main.dart，
// 免去手敲长路径。参数原样透传、退出码透传。
// 不含任何业务：模块依赖与环境由各模块自己的 dev 脚本打理，
// 平台支持也由模块的 dev/dev.bat 存在性声明（装配器只机械检查）。

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const isWin = process.platform === 'win32';

// 拉起产品（Windows 的 .bat 须经 cmd.exe）
const r = isWin
    ? spawnSync('cmd.exe',
        ['/c', join(root, 'bin', 'dart.bat'), 'bin/main.dart', ...process.argv.slice(2)],
        { cwd: join(root, 'assistant', 'apps', 'solomni'), stdio: 'inherit' })
    : spawnSync(join(root, 'bin', 'dart'),
        ['bin/main.dart', ...process.argv.slice(2)],
        { cwd: join(root, 'assistant', 'apps', 'solomni'), stdio: 'inherit' });
process.exit(r.status ?? (r.signal ? 130 : 1));

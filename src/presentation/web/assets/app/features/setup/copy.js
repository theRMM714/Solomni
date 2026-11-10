/* 目的：依赖补齐弹层的文案：生效说明与结构化诊断的中文。
 * 不管：补齐动作、轮询与弹层（见同目录 setup.js）。 */

export const setupCopy = {
  applied: '已经生效：再打开配置 / 向导就按新状态判定，下一次启动虚拟机就用上，不需要重启。',
  missing: (d) => '模块 ' + d.module + ' 需要的运行能力 ' + d.capability +
    ' 在本档位没有：把它放进依赖文件夹后点「重新扫描」，或确认基础工具链 / 宿主 PATH。',
  ambiguous: (d) => '运行能力 ' + d.capability + ' 有多个版本（' + (d.versions || []).join('、') +
    '）：在下面「定版」里选一个版本，工具才跑得起来。',
  unknownPin: (d) => '运行能力 ' + d.capability + ' 定的版本 ' + d.version +
    ' 不在本档位可用的包里：改一个版本，或把该版本的运行包放进依赖文件夹后点「重新扫描」。',
  notForTarget: (d) => '模块 ' + d.module + ' 需要的运行能力 ' + d.capability +
    ' 在本档位用不了：包库里只有 ' + (d.packages || []).join('、') +
    '（目标：' + d.target + '）。换一个包，或改用虚拟机档。',
  noBin: (d) => '运行包 ' + d.package + ' 没有可执行的 bin/：本机档放不进 PATH。检查包内容或换一个包。',
  missingEntry: (d) => '模块 ' + d.module + ' 的工具 ' + d.tool +
    ' 入口不在场：' + d.command + '。先按模块 README 或 --build 编一次。',
};

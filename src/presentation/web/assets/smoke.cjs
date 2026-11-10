/* 前端冒烟跑手：递归发现本目录下所有 *.smoke.cjs / *.smoke.mjs，逐个用 node 跑，最后汇总。
 * 用法：node src/presentation/web/assets/smoke.cjs   （全通过输出 FRONTEND-SMOKE-OK；任一失败非零退出）
 * 为什么需要：新增前端冒烟只要放进本目录（含子目录）并以 .smoke.cjs / .smoke.mjs 结尾就自动纳入，不必改跑手、也不必改文档。
 *   .smoke.mjs 用于 ES Module：冒烟直接 import 被测模块。两种开发脚本都不进发行版（build.rs 跳过 .cjs / .mjs）。
 * 子进程用 stdio: 'inherit'：每个冒烟自己打印成功标记、自己把细节打到同一个输出流；
 * 跑手只汇总退出码，不抓取子进程输出（不经过管道，跨平台且不受受限环境污染）。
 */
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const dir = __dirname;
const isSmoke = (f) => f.endsWith(".smoke.cjs") || f.endsWith(".smoke.mjs");
const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p);
    else if (isSmoke(e.name)) files.push(path.relative(dir, p));
  }
})(dir);
files.sort();
if (!files.length) {
  console.log("前端冒烟：一个都没找到（期望本目录下 *.smoke.cjs / *.smoke.mjs）");
  console.log("FRONTEND-SMOKE-FAIL");
  process.exit(1);
}
const failed = [];
files.forEach((f, i) => {
  console.log("[" + (i + 1) + "/" + files.length + "] " + f);
  const r = spawnSync(process.execPath, [path.join(dir, f)], { stdio: "inherit" });
  if (r.error) console.log("  跑不起来：" + r.error.message);
  const code = r.status === null || r.status === undefined ? 1 : r.status;
  console.log("  → " + (code === 0 ? "通过" : "失败（退出码 " + code + "）"));
  if (code !== 0) failed.push(f);
});
if (failed.length) {
  console.log("前端冒烟失败：" + failed.join("、"));
  console.log("FRONTEND-SMOKE-FAIL");
  process.exit(1);
}
console.log("前端冒烟：" + files.length + " 个全部通过（" + files.join("、") + "）");
console.log("FRONTEND-SMOKE-OK");

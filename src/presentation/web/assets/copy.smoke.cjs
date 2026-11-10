/* 文案分层冒烟：功能文案集中在 copy.js，首层短、详情长，安全提示不折叠。
 * 用法：node src/presentation/web/assets/copy.smoke.cjs   （输出 COPY-SMOKE-OK / FAIL）
 * 为什么需要：长说明散回视图里，首层就又变回一堵墙；这条只能机器守。
 */
const fs = require("fs");
const path = require("path");
const { pathToFileURL } = require("url");

const APP = path.join(__dirname, "app");
const problems = [];
const LEAD_MAX = 40;
const DETAIL_MAX = 600;
const INLINE_MAX = 40;

function walkFiles(dir, out) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walkFiles(p, out);
    else if (e.name.endsWith(".js")) out.push(p);
  }
  return out;
}

function chinese(s) {
  return (String(s).match(/[\u4e00-\u9fff]/g) || []).length;
}

function checkValue(value, where) {
  if (typeof value === "function") return;
  if (Array.isArray(value)) { value.forEach((v, i) => checkValue(v, where + "[" + i + "]")); return; }
  if (value && typeof value === "object") {
    if (typeof value.lead === "string") {
      if (chinese(value.lead) > LEAD_MAX) problems.push(where + ".lead 超过 " + LEAD_MAX + " 字");
      if (value.detail && chinese(value.detail) > DETAIL_MAX) problems.push(where + ".detail 超过 " + DETAIL_MAX + " 字");
      if (value.safety && chinese(value.safety) > DETAIL_MAX) problems.push(where + ".safety 超过 " + DETAIL_MAX + " 字");
      return;
    }
    for (const k of Object.keys(value)) checkValue(value[k], where + "." + k);
    return;
  }
  if (typeof value === "string" && chinese(value) > DETAIL_MAX) problems.push(where + " 超过 " + DETAIL_MAX + " 字");
}

const files = walkFiles(APP, []);
const literals = /'([^'\\\n]|\\.)*'|"([^"\\\n]|\\.)*"|`([^`\\]|\\.)*`/g;
for (const file of files) {
  const src = fs.readFileSync(file, "utf8");
  if (!file.endsWith("copy.js")) {
    for (const m of src.matchAll(literals)) {
      const body = m[0].slice(1, -1);
      if (chinese(body) >= INLINE_MAX) {
        const line = src.slice(0, m.index).split("\n").length;
        problems.push(path.relative(APP, file) + ":" + line + " 内联中文 " + chinese(body) + " 字，应搬进 copy.js");
      }
    }
  }
}

(async () => {
  const copies = files.filter((f) => f.endsWith("copy.js"));
  for (const file of copies) {
    const mod = await import(pathToFileURL(file).href);
    for (const key of Object.keys(mod)) checkValue(mod[key], path.relative(APP, file).replace(/\.js$/, "") + "." + key);
  }
  if (problems.length) {
    console.log(problems.join("\n"));
    console.log("COPY-SMOKE-FAIL");
    process.exit(1);
  }
  console.log("文案检查：" + copies.length + " 份 copy.js，首层 ≤" + LEAD_MAX + " 字，无 ≥" + INLINE_MAX + " 字内联中文");
  console.log("COPY-SMOKE-OK");
})();

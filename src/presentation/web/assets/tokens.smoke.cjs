/* 令牌纪律冒烟：令牌是唯一数值来源，组件与页面只准引用 var(--…)。
 * 用法：node src/presentation/web/assets/tokens.smoke.cjs   （输出 TOKENS-SMOKE-OK / FAIL）
 * 为什么需要：字面量颜色与时长散落一次，后面每次改主题都会漏；这条只能机器守。
 */
const fs = require("fs");
const path = require("path");
const DIR = __dirname;
const problems = [];
const read = (f) => fs.readFileSync(path.join(DIR, f), "utf8");

function stripComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

function definedVars(css) {
  const out = new Set();
  for (const m of css.matchAll(/(--[a-z0-9-]+)\s*:/g)) out.add(m[1]);
  return out;
}

function usedVars(css) {
  const out = new Set();
  for (const m of css.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) out.add(m[1]);
  return out;
}

function checkSelectors(css) {
  for (const line of css.split("\n")) {
    const i = line.indexOf("{");
    if (i < 0) continue;
    const sel = line.slice(0, i).trim();
    if (!sel) continue;
    if (sel === ":root" || sel.startsWith("@media")) continue;
    problems.push("tokens.css 只许定义变量，出现了选择器：" + sel);
  }
}

const tokens = stripComments(read("tokens.css"));
const ui = stripComments(read("ui.css"));
const page = stripComments(read("style.css"));

if (!/:root\s*\{/.test(tokens)) problems.push("tokens.css 缺少 :root");
if (!/@media\s*\(prefers-reduced-motion:\s*reduce\)/.test(tokens)) {
  problems.push("tokens.css 缺少 prefers-reduced-motion 段");
}
checkSelectors(tokens);

const defined = definedVars(tokens);
for (const [name, css] of [["ui.css", ui], ["style.css", page]]) {
  for (const v of usedVars(css)) {
    if (!defined.has(v)) problems.push(name + " 用了未定义的令牌：" + v);
  }
  for (const m of css.matchAll(/#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(/g)) {
    problems.push(name + " 出现字面量颜色：" + m[0]);
  }
  for (const m of css.matchAll(/\b\d+(?:\.\d+)?m?s\b/g)) {
    problems.push(name + " 出现字面量时长：" + m[0]);
  }
}

if (problems.length) {
  const uniq = [...new Set(problems)];
  console.log(uniq.slice(0, 20).join("\n"));
  console.log("TOKENS-SMOKE-FAIL（" + uniq.length + " 条）");
  process.exit(1);
}
console.log("令牌检查：" + defined.size + " 个令牌，ui.css / style.css 无字面量颜色与时长");
console.log("TOKENS-SMOKE-OK");

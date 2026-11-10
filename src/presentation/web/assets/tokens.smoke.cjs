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

/* ---- 令牌层级：字号 5 档、间距 4 的倍数、颜色必须是不透明 hex ---- */
function declMap(css) {
  const out = new Map();
  for (const m of css.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) out.set(m[1], m[2].trim());
  return out;
}
const decls = declMap(tokens);

const FS = ["--fs-xs", "--fs-s", "--fs-base", "--fs-l", "--fs-xl"];
const fsNames = [...decls.keys()].filter((n) => n.startsWith("--fs-"));
if (fsNames.length !== FS.length || FS.some((n) => !decls.has(n))) {
  problems.push("字号必须恰好是 5 档：" + FS.join(" / ") + "；实际 " + fsNames.join(" / "));
} else {
  const vals = FS.map((n) => parseFloat(decls.get(n)));
  for (let i = 1; i < vals.length; i += 1) {
    if (!(vals[i] > vals[i - 1])) problems.push("字号阶梯不是严格递增：" + FS[i - 1] + " " + vals[i - 1] + " → " + FS[i] + " " + vals[i]);
  }
  if (vals[2] !== 16) problems.push("--fs-base 必须是 16px，实际 " + vals[2]);
}

for (const [n, v] of decls) {
  if (!n.startsWith("--sp-")) continue;
  const px = parseFloat(v);
  if (!/^\d+px$/.test(v) || px % 4 !== 0) problems.push("间距不是 4 的倍数：" + n + " = " + v);
}

/* 阴影 3 档：模糊半径必须递增。 */
const shadows = ["--shadow-1", "--shadow-2", "--shadow-3"];
if (!shadows.every((n) => decls.has(n))) problems.push("阴影必须恰好 3 档：" + shadows.join(" / "));
else {
  const blur = shadows.map((n) => {
    const p = decls.get(n).split(/\s+/);
    return parseFloat(p[2]);
  });
  for (let i = 1; i < blur.length; i += 1) {
    if (!(blur[i] > blur[i - 1])) problems.push("阴影层级模糊半径不递增：" + shadows[i - 1] + " → " + shadows[i]);
  }
}

/* 允许带透明的只有叠层、弱底与描边线；其余颜色一律 #rrggbb（阴影不是颜色令牌）。 */
const ALPHA_OK = /overlay|mask|weak|line/;
for (const [n, v] of decls) {
  if (!n.startsWith("--color-") && !n.startsWith("--c-")) continue;
  if (ALPHA_OK.test(n)) continue;
  if (!/^#[0-9a-f]{6}$/i.test(v) && !/^var\(--[a-z0-9-]+\)$/.test(v)) {
    problems.push("颜色令牌必须是不透明 hex： " + n + " = " + v);
  }
}

/* ---- 对比度（WCAG AA）：文字 4.5:1，控件边界与焦点 3:1 ---- */
function resolveColor(name) {
  let v = decls.get(name);
  for (let i = 0; i < 6 && v; i += 1) {
    const m = v.match(/^var\((--[a-z0-9-]+)\)$/);
    if (!m) break;
    v = decls.get(m[1]);
  }
  if (!v || !/^#[0-9a-f]{6}$/i.test(v)) return null;
  return [parseInt(v.slice(1, 3), 16), parseInt(v.slice(3, 5), 16), parseInt(v.slice(5, 7), 16)];
}
function luminance(rgb) {
  const f = (v) => { const x = v / 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); };
  return 0.2126 * f(rgb[0]) + 0.7152 * f(rgb[1]) + 0.0722 * f(rgb[2]);
}
function contrast(a, b) {
  const l1 = luminance(a);
  const l2 = luminance(b);
  const hi = Math.max(l1, l2);
  const lo = Math.min(l1, l2);
  return (hi + 0.05) / (lo + 0.05);
}
const PAIRS = [
  ["--color-text", "--color-bg", 4.5],
  ["--color-text", "--color-surface", 4.5],
  ["--color-text", "--color-surface-2", 4.5],
  ["--color-text-dim", "--color-bg", 4.5],
  ["--color-text-dim", "--color-surface", 4.5],
  ["--color-text-dim", "--color-surface-2", 4.5],
  ["--color-text", "--color-accent-weak", 4.5],
  ["--color-text-dim", "--color-accent-weak", 4.5],
  ["--color-accent", "--color-bg", 4.5],
  ["--color-accent", "--color-surface", 4.5],
  ["--color-accent-contrast", "--color-accent", 4.5],
  ["--color-text-inverse", "--color-err", 4.5],
  ["--color-ok", "--color-bg", 4.5],
  ["--color-warn", "--color-bg", 4.5],
  ["--color-err", "--color-bg", 4.5],
  ["--color-ok", "--color-surface-2", 4.5],
  ["--color-warn", "--color-surface-2", 4.5],
  ["--color-err", "--color-surface-2", 4.5],
  ["--color-border-strong", "--color-bg", 3],
  ["--color-border-strong", "--color-surface", 3],
  ["--color-border-strong", "--color-surface-2", 3],
  ["--color-focus", "--color-bg", 3],
  ["--color-focus", "--color-surface", 3],
];
let pairCount = 0;
for (const [fgName, bgName, need] of PAIRS) {
  const fg = resolveColor(fgName);
  const bg = resolveColor(bgName);
  if (!fg || !bg) { problems.push("对比度取样失败：" + fgName + " / " + bgName); continue; }
  const got = Math.round(contrast(fg, bg) * 100) / 100;
  pairCount += 1;
  if (got < need) problems.push(fgName + " / " + bgName + " 对比度 " + got + ":1 < " + need + ":1");
}

if (problems.length) {
  const uniq = [...new Set(problems)];
  console.log(uniq.slice(0, 20).join("\n"));
  console.log("TOKENS-SMOKE-FAIL（" + uniq.length + " 条）");
  process.exit(1);
}
console.log("令牌检查：" + defined.size + " 个令牌，5 档字号，对比度 " + pairCount + " 对达 AA，ui.css / style.css 无字面量颜色与时长");
console.log("TOKENS-SMOKE-OK");

/* 陈列页冒烟：只在 debug 嵌入，引用真实组件，不引外网、不写死颜色。
 * 用法：node src/presentation/web/assets/dev/gallery.smoke.cjs   （输出 GALLERY-SMOKE-OK / FAIL）
 * 为什么需要：陈列页是组件换代时唯一能一眼看全的地方；它悄悄坏掉比没有更糟。
 */
const fs = require("fs");
const path = require("path");
const DIR = __dirname;
const problems = [];

const html = fs.readFileSync(path.join(DIR, "gallery.html"), "utf8");
const js = fs.readFileSync(path.join(DIR, "gallery.js"), "utf8");
const build = fs.readFileSync(path.join(DIR, "..", "..", "..", "..", "..", "build.rs"), "utf8");

function check(ok, msg) { if (!ok) problems.push(msg); }

for (const href of ["tokens.css", "ui.css", "style.css"]) {
  check(html.indexOf('/assets/' + href) >= 0, "gallery.html 缺 /assets/" + href);
}
check(html.indexOf('/assets/dev/gallery.js') >= 0, "gallery.html 没引 gallery.js");
for (const id of ["modal-root", "notice-root", "toast-root", "menu-root"]) {
  check(html.indexOf('id="' + id + '"') >= 0, "gallery.html 缺挂载点 #" + id);
}
check(html.indexOf("http://") < 0 && html.indexOf("https://") < 0, "gallery.html 不应引外网资源");
check(js.indexOf("createDom") >= 0 && js.indexOf("createIcons") >= 0 &&
  js.indexOf("createButtons") >= 0 && js.indexOf("createForms") >= 0 &&
  js.indexOf("createOverlays") >= 0, "gallery.js 应引用真实组件工厂");
check(js.indexOf("ICONS") >= 0, "gallery.js 应陈列全部图标");
check(!/#[0-9a-fA-F]{3,8}\b/.test(html + js), "陈列页不许写死颜色，颜色只走令牌");
check(build.indexOf("DEV_ONLY_DIRS") >= 0 && build.indexOf("PROFILE") >= 0,
  "build.rs 应只在 debug 嵌入 dev/");

if (problems.length) {
  console.log(problems.join("\n"));
  console.log("GALLERY-SMOKE-FAIL");
  process.exit(1);
}
console.log("GALLERY-SMOKE-OK");

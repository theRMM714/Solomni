/* 侧栏抽屉冒烟：移动端左上角按钮必须能开也能合，aria-expanded 跟着状态走。
 * 用法：node src/presentation/web/assets/app/features/sidebar/sidebar.smoke.cjs （输出 SIDEBAR-SMOKE-OK / FAIL）
 * 为什么需要：窄屏下按钮只开会不合，是真实发生过的回归；桩 DOM 足以钉住这条判据。
 */
const path = require("path");
const { pathToFileURL } = require("url");

const failures = [];
function check(name, cond, extra) {
  if (!cond) failures.push(name + (extra === undefined ? "" : " :: " + extra));
}

function node() {
  const n = {
    attrs: {},
    classes: new Set(),
    classList: {
      add(c) { n.classes.add(c); },
      remove(c) { n.classes.delete(c); },
      contains(c) { return n.classes.has(c); },
    },
    setAttribute(k, v) { n.attrs[k] = String(v); },
  };
  return n;
}

(async () => {
  const sidebar = node();
  const mask = node();
  const drawerBtn = node();
  const nodes = new Map([["#sidebar", sidebar], ["#drawer-mask", mask], ["#btn-drawer", drawerBtn]]);

  const mod = await import(pathToFileURL(path.join(__dirname, "sidebar.js")).href);
  const root = {};
  mod.mount(root, {
    g: { matchMedia: () => ({ matches: true }), addEventListener() {} },
    doc: { querySelector: (s) => nodes.get(s) || node(), addEventListener() {} },
    state: {}, api: {}, ui: {}, app: {},
  });

  root.toggleSidebar();
  check("第一次点开抽屉", sidebar.classList.contains("open") && mask.classList.contains("show"));
  check("点开后 aria-expanded=true", drawerBtn.attrs["aria-expanded"] === "true", drawerBtn.attrs["aria-expanded"]);

  root.toggleSidebar();
  check("再点一次收起抽屉", !sidebar.classList.contains("open") && !mask.classList.contains("show"));
  check("收起后 aria-expanded=false", drawerBtn.attrs["aria-expanded"] === "false", drawerBtn.attrs["aria-expanded"]);

  root.openDrawer();
  check("遮罩/外部入口打开时也同步 aria", drawerBtn.attrs["aria-expanded"] === "true");
  root.closeDrawer();
  check("关闭入口清掉展开态", drawerBtn.attrs["aria-expanded"] === "false" && !sidebar.classList.contains("open"));

  if (failures.length) {
    console.error("侧栏抽屉冒烟失败：");
    for (const f of failures) console.error(" - " + f);
    console.error("SIDEBAR-SMOKE-FAIL");
    process.exit(1);
  }
  console.log("侧栏抽屉：开 / 合 / aria-expanded 均已验证");
  console.log("SIDEBAR-SMOKE-OK");
})();

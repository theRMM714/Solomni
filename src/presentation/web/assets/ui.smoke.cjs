/* 组件库冒烟：图标 / 图标按钮 / 折叠 / toast / 菜单 / 弹层关闭判据。
 * 用法：node src/presentation/web/assets/ui.smoke.cjs   （输出 UI-SMOKE-OK / FAIL）
 * 为什么需要：ui.js 是 no-build 原生 JS，组件结构错了只有真机才看得出来；这里用最小桩 DOM 钉住形状。
 */
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const failures = [];
function check(name, cond, extra) {
  if (!cond) failures.push(name + (extra === undefined ? "" : " :: " + extra));
}

function el(tag) {
  const node = {
    tagName: (tag || "div").toUpperCase(),
    type: "", className: "", textContent: "", innerHTML: "", title: "", ariaLabel: "",
    disabled: false, open: false, dataset: {}, style: {}, attrs: {}, children: [], parentNode: null, _h: {},
    appendChild(c) { if (!c) throw new Error("appendChild 收到空值"); c.parentNode = node; node.children.push(c); return c; },
    removeChild(c) { const i = node.children.indexOf(c); if (i >= 0) node.children.splice(i, 1); c.parentNode = null; return c; },
    remove() { if (node.parentNode) node.parentNode.removeChild(node); },
    setAttribute(k, v) { node.attrs[k] = String(v); if (k === "aria-label") node.ariaLabel = String(v); },
    addEventListener(t, f) { (node._h[t] = node._h[t] || []).push(f); },
    removeEventListener() {},
    contains(n) { return node.children.indexOf(n) >= 0; },
    getBoundingClientRect() { return { left: 10, top: 0, right: 120, bottom: 24 }; },
  };
  return node;
}

const toastRoot = el("div");
const menuRoot = el("div");
const body = el("body");
const doc = {
  createElement: (t) => el(t),
  querySelector: (s) => (s === "#toast-root" ? toastRoot : s === "#menu-root" ? menuRoot : null),
  body,
  addEventListener() {},
  removeEventListener() {},
};
const sandbox = {
  document: doc, setTimeout, clearTimeout, console, JSON, Object, Array, String, Number, Boolean, Math, Error, Map, Set,
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
try {
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "ui.js"), "utf8"), sandbox, { filename: "ui.js" });
} catch (e) {
  console.log("ui.js 载入失败：" + e.message);
  console.log("UI-SMOKE-FAIL");
  process.exit(1);
}
const ui = sandbox.ui;
check("ui 命名空间存在", ui && typeof ui.icon === "function");

const ico = ui.icon("undo");
check("图标是 .ui-icon", ico.className === "ui-icon", ico.className);
check("图标内联 SVG", String(ico.innerHTML).indexOf("<svg") === 0, String(ico.innerHTML).slice(0, 40));
check("图标对读屏隐藏", ico.attrs["aria-hidden"] === "true");

let clicked = 0;
const btn = ui.iconButton({ icon: "pencil", label: "会话操作", tip: "打开菜单", onClick: () => { clicked += 1; } });
check("图标按钮是 button", btn.tagName === "BUTTON");
check("图标按钮类名", btn.className.indexOf("icon-btn") === 0, btn.className);
check("图标按钮 aria-label", btn.attrs["aria-label"] === "会话操作", btn.attrs["aria-label"]);
check("图标按钮 data-tip", btn.dataset.tip === "打开菜单", btn.dataset.tip);
check("图标按钮带图标", (btn.children[0] || {}).className === "ui-icon");
btn.onclick();
check("图标按钮接线", clicked === 1);

const fold = ui.fold({ label: "思维链", count: 12, body: el("div") });
check("折叠是 details", fold.tagName === "DETAILS");
check("折叠有 summary", (fold.children[0] || {}).tagName === "SUMMARY");
const sum = fold.children[0] || { children: [] };
check("折叠标签", String(sum.children[0] && sum.children[0].textContent) === "思维链");
check("折叠计数", String(sum.children[1] && sum.children[1].textContent) === "12");
check("折叠体在 .fold-body", (fold.children[1] || {}).className === "fold-body");

const overlay = el("div");
const panel = el("div");
check("按在遮罩、松在遮罩才关", ui.dismisses(overlay, overlay, overlay) === true);
check("面板内按下不关", ui.dismisses(overlay, panel, overlay) === false);
check("松在面板内不关", ui.dismisses(overlay, overlay, panel) === false);
check("面板内按下 + 面板内松开不关", ui.dismisses(overlay, panel, panel) === false);

let closed = 0;
ui.bindDismiss(overlay, panel, () => { closed += 1; });
overlay._h.pointerdown[0]({ target: panel });
overlay._h.click[0]({ target: overlay });
check("拖选跨出弹层不关", closed === 0, String(closed));
overlay._h.pointerdown[0]({ target: overlay });
overlay._h.click[0]({ target: overlay });
check("点遮罩才关", closed === 1, String(closed));

const t1 = ui.toast("ok", "已保存");
check("toast 进根节点", toastRoot.children.length === 1 && t1.parentNode === toastRoot);
check("toast 带状态类", t1.className.indexOf("toast") === 0 && t1.className.indexOf("ok") > 0, t1.className);
ui.toast("warn", "注意");
ui.toast("err", "失败");
ui.toast("ok", "再来一条");
check("toast 最多三条", toastRoot.children.length === 3, String(toastRoot.children.length));

const anchor = el("button");
let opened = 0;
const m = ui.menu(anchor, [
  { label: "打开", icon: "folder", onClick: () => { opened += 1; } },
  { sep: true },
  { label: "删除", danger: true },
]);
check("菜单挂到 #menu-root", menuRoot.children.length === 1 && m.parentNode === menuRoot);
check("菜单项 + 分隔线", m.children.length === 3, String(m.children.length));
check("危险项带 danger", String(m.children[2].className).indexOf("danger") > 0);
m.children[0].onclick();
check("菜单项接线并收起", opened === 1 && menuRoot.children.length === 0);

const chip = ui.statusChip({ icon: "chart", text: "18,432 tok · 14.4%", label: "上下文用量", cls: "warn", onClick: () => { opened += 1; } });
check("状态 chip 是 button", chip.tagName === "BUTTON");
check("状态 chip 类名", chip.className.indexOf("status-chip") === 0, chip.className);
check("状态 chip 带图标与文字", (chip.children[0] || {}).className === "ui-icon" && String((chip.children[1] || {}).textContent).indexOf("18,432") === 0);
chip.onclick();
check("状态 chip 接线", opened === 2, String(opened));

const floating = el("div");
const pop = ui.popover(anchor, floating, { label: "上下文用量" });
check("小卡片挂到 #menu-root", menuRoot.children.length === 1 && pop.node.parentNode === menuRoot);
check("小卡片 role=dialog", pop.node.attrs.role === "dialog");
check("锚点 aria-expanded", anchor.attrs["aria-expanded"] === "true");
pop.close();
check("关闭后移除小卡片", menuRoot.children.length === 0 && anchor.attrs["aria-expanded"] === "false");

if (failures.length) {
  console.log(failures.join("\n"));
  console.log("UI-SMOKE-FAIL（" + failures.length + " 条）");
  process.exit(1);
}
console.log("UI-SMOKE-OK");
process.exit(0);

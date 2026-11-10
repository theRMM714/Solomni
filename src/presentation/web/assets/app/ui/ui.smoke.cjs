/* 组件库冒烟：图标 / 按钮 / 表单 / 折叠 / toast / 菜单 / 弹层关闭判据 / 小卡片。
 * 用法：node src/presentation/web/assets/app/ui/ui.smoke.cjs   （输出 UI-SMOKE-OK / FAIL）
 * 为什么需要：组件不经构建直接进浏览器，结构错了只有真机才看得出来；这里用最小桩 DOM 钉住形状。
 */
const { createUi } = require("./index.js");
const failures = [];
function check(name, cond, extra) {
  if (!cond) failures.push(name + (extra === undefined ? "" : " :: " + extra));
}

function el(tag) {
  const node = {
    tagName: (tag || "div").toUpperCase(),
    type: "", className: "", textContent: "", title: "", ariaLabel: "",
    disabled: false, open: false, dataset: {}, style: {}, attrs: {}, children: [], parentNode: null, _h: {},
    classes: new Set(),
    classList: { add(c) { node.classes.add(c); }, remove(c) { node.classes.delete(c); } },
    appendChild(c) { if (!c) throw new Error("appendChild 收到空值"); c.parentNode = node; node.children.push(c); return c; },
    removeChild(c) { const i = node.children.indexOf(c); if (i >= 0) node.children.splice(i, 1); c.parentNode = null; return c; },
    remove() { if (node.parentNode) node.parentNode.removeChild(node); },
    setAttribute(k, v) { node.attrs[k] = String(v); if (k === "aria-label") node.ariaLabel = String(v); },
    addEventListener(t, f) { (node._h[t] = node._h[t] || []).push(f); },
    removeEventListener() {},
    contains(n) { return node.children.indexOf(n) >= 0; },
    getBoundingClientRect() { return { left: 10, top: 0, right: 120, bottom: 24 }; },
  };
  // 与真实 DOM 一致：innerHTML = '' 清空子节点（下拉换选项靠它）。
  let html = "";
  Object.defineProperty(node, "innerHTML", { get: () => html, set: (v) => { html = v; if (v === "") node.children = []; } });
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
const ui = createUi(sandbox);

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
const sumParts = sum.children || [];
check("折叠图标", sumParts.some((c) => c.className === "ui-icon"));
check("折叠标签", String((sumParts.find((c) => c.className === "fold-label") || {}).textContent) === "思维链");
check("折叠计数", String((sumParts.find((c) => c.className === "fold-count") || {}).textContent) === "12");
check("折叠体在 .fold-body", (fold.children[1] || {}).className === "fold-body");

const det = ui.detail({ lead: "第一层", detail: "第二层", safety: "不可逆" });
check("分层说明：三段齐全", det.children.length === 3, String(det.children.length));
check("分层说明：详情收在折叠里", String(det.children[1].className).indexOf("fold") === 0);
check("分层说明：安全文案不折叠", det.children[2].className === "detail-safety" && det.children[2].textContent === "不可逆");

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

// 关闭回调：主动关与"被下一张顶掉"都要响，调用方才能清掉自己的计时器。
let closedCount = 0;
const pop2 = ui.popover(anchor, el("div"), { label: "关闭回调", onClose: () => { closedCount += 1; } });
pop2.close();
check("关闭回调响一次", closedCount === 1, String(closedCount));
let replaced = 0;
ui.popover(anchor, el("div"), { onClose: () => { replaced += 1; } });
const third = ui.popover(anchor, el("div"), {});
check("换一张卡片会关掉旧的（旧回调要响）", replaced === 1, String(replaced));
third.close();

// 小卡片夹在视口内：锚点贴底 + 卡片比"锚点下方"高时，底边不得越出视口；锚点被重建后沿用上次位置。
sandbox.innerHeight = 300;
const lowAnchor = el("button");
lowAnchor.getBoundingClientRect = () => ({ left: 10, top: 260, right: 120, bottom: 290 });
const clamped = ui.popover(lowAnchor, el("div"), { label: "夹住底部" });
clamped.node.getBoundingClientRect = () => ({ width: 400, height: 250 });
clamped.place();
const clampedTop = parseInt(clamped.node.style.top, 10);
check("小卡片底边不越出视口", clampedTop >= 8 && clampedTop + 250 <= 292, clamped.node.style.top);
lowAnchor.getBoundingClientRect = () => ({ left: 0, top: 0, right: 0, bottom: 0 });
clamped.place();
check("锚点被重建后沿用上次位置", parseInt(clamped.node.style.top, 10) === clampedTop, clamped.node.style.top);
clamped.close();
delete sandbox.innerHeight;

// 视口滚动监听：卡片自己滚（长回执）不能关，页面滚才关。
const winHandlers = {};
sandbox.addEventListener = (t, f) => { (winHandlers[t] = winHandlers[t] || []).push(f); };
sandbox.removeEventListener = (t, f) => {
  const list = winHandlers[t] || [];
  const i = list.indexOf(f);
  if (i >= 0) list.splice(i, 1);
};
const scrollAnchor = el("button");
const scrollCard = ui.popover(scrollAnchor, el("div"), { label: "滚动" });
const fireScroll = (target) => (winHandlers.scroll || []).slice().forEach((f) => f({ target }));
fireScroll(scrollCard.node);
check("卡片内部滚动不关闭卡片", scrollCard.node.parentNode === menuRoot, String(scrollCard.node.parentNode && scrollCard.node.parentNode.tagName));
fireScroll(el("div"));
check("页面滚动关闭卡片", menuRoot.children.length === 0);

// 按钮与表单：形状、类名与返回值（numberInput 返回 {input, wrap}，wrap 才挂进表单）。
const b = ui.btn("保存", "btn btn-primary");
check("文字按钮", b.tagName === "BUTTON" && b.type === "button" && b.className === "btn btn-primary" && b.textContent === "保存");
check("文字按钮默认类", ui.btn("x").className === "btn");
const reset = ui.busyButton(b, "保存中…");
check("忙态：禁用 + 转圈 + 换文案", b.disabled === true && b.classes.has("busy") && b.textContent === "保存中…");
reset();
check("忙态复位", b.disabled === false && !b.classes.has("busy") && b.textContent === "保存");
const ti = ui.textInput("名字", "password");
check("文本输入", ti.tagName === "INPUT" && ti.className === "field-input" && ti.placeholder === "名字" && ti.type === "password" && ti.autocomplete === "off");
const fd = ui.field("名字", ti);
check("字段 = 标签 + 输入", fd.className === "wf-field" && fd.children[0].className === "wf-label" && fd.children[0].textContent === "名字" && fd.children[1] === ti);
const num = ui.numberInput("超时（秒）", 300, 10, 3600);
check("数字输入返回 {input, wrap}", num.input.type === "number" && num.input.min === "10" && num.input.max === "3600" && num.input.value === "300" && num.wrap.className === "wf-field");
check("数字输入空值", ui.numberInput("x", null, 0, 1).input.value === "");
const ta = ui.areaInput("需求");
check("多行输入", ta.tagName === "TEXTAREA" && ta.rows === 3 && ta.placeholder === "需求");
const sel = ui.selectInput([{ value: "a", label: "甲" }, { value: "b", label: "乙" }], "b");
check("下拉", sel.tagName === "SELECT" && sel.children.length === 2 && sel.children[1].textContent === "乙" && sel.value === "b");
check("下拉换选项保留选中", ui.syncSelect(sel, [{ value: "c", label: "丙" }, { value: "b", label: "乙" }]) === "b" && sel.children.length === 2);
check("下拉选中值消失退回第一个", ui.syncSelect(sel, [{ value: "d", label: "丁" }], "zz") === "d");
check("下拉空选项", ui.syncSelect(sel, [], "x") === "");
const cb = ui.checkbox("流式", true);
check("勾选框", cb.wrap.tagName === "LABEL" && cb.wrap.className === "chk" && cb.box.type === "checkbox" && cb.box.checked === true && cb.wrap.children[1].textContent === "流式");
const rd = ui.radio("虚拟机档", false, "tier", true);
check("单选", rd.box.type === "radio" && rd.box.name === "tier" && rd.box.checked === false && rd.box.disabled === true);
const eh = ui.emptyHint("还没有模型");
check("空态提示", eh.className === "reg-empty" && eh.textContent === "还没有模型");
const dv = ui.div("cfg-sec-title", "服务");
check("说明块", dv.className === "cfg-sec-title" && dv.textContent === "服务" && dv.children.length === 0);
const nt = ui.hint("读了后端字段", "warn");
check("旁注带状态类", nt.className === "cfg-hint warn" && nt.textContent === "读了后端字段");
check("旁注无状态类", ui.hint("纯说明").className === "cfg-hint");

if (failures.length) {
  console.log(failures.join("\n"));
  console.log("UI-SMOKE-FAIL（" + failures.length + " 条）");
  process.exit(1);
}
console.log("UI-SMOKE-OK");
process.exit(0);

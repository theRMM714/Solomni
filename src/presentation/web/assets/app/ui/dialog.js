/* 目的：屏幕居中的一次性提示（自研，顶替原生 alert/confirm）：notice 只报一件事，confirmBox 返回 Promise<boolean>。
 * 管：三种配色（info / warn / err）的提示面板、确认面板的按钮语义与关闭方式（按钮、Esc、点遮罩）。
 * 不管：主区弹层（见 ui/modal.js，那个在主区里、可叠层）；业务文案与跳转。
 * 联动：由 app/main.js 装配；关闭判据用 ui/overlays.js 的 bindDismiss，按钮用 ui/buttons.js。 */

let doc, ui;
let btn;
const $ = (sel) => doc.querySelector(sel);

function notice(title, text, kind) {
  const root = $("#notice-root");
  root.innerHTML = "";
  root.className = "notice-root open";
  const overlay = doc.createElement("div"); overlay.className = "notice-overlay";
  const panel = doc.createElement("div"); panel.className = "notice-panel notice-" + (kind || "info");
  const head = doc.createElement("div"); head.className = "notice-title"; head.textContent = title || "";
  const body = doc.createElement("div"); body.className = "notice-text"; body.textContent = text || "";
  const row = doc.createElement("div"); row.className = "notice-actions";
  const ok = btn("知道了", "btn btn-primary");
  let unbind = null;
  const close = () => {
    if (unbind) { unbind(); unbind = null; }
    root.innerHTML = "";
    root.className = "notice-root";
  };
  ok.onclick = close;
  row.appendChild(ok);
  panel.appendChild(head); panel.appendChild(body); panel.appendChild(row);
  overlay.appendChild(panel);
  root.appendChild(overlay);
  unbind = ui.bindDismiss(overlay, panel, close);
  return close;
}

/// 需要用户二选一的确认（顶替原生 confirm）：返回 Promise<boolean>，只认"确认"或"取消"。
function confirmBox(title, text, confirmLabel, danger) {
  return new Promise((resolve) => {
    const root = $("#notice-root");
    root.innerHTML = "";
    root.className = "notice-root open";
    const overlay = doc.createElement("div"); overlay.className = "notice-overlay";
    const panel = doc.createElement("div"); panel.className = "notice-panel notice-warn";
    const head = doc.createElement("div"); head.className = "notice-title"; head.textContent = title || "";
    const body = doc.createElement("div"); body.className = "notice-text" + (danger ? " notice-danger" : ""); body.textContent = text || "";
    const row = doc.createElement("div"); row.className = "notice-actions";
    let unbind = null;
    const done = (v) => {
      if (unbind) { unbind(); unbind = null; }
      root.innerHTML = "";
      root.className = "notice-root";
      resolve(v);
    };
    const no = btn("取消", "btn"); no.onclick = () => done(false);
    const yes = btn(confirmLabel || "确认", "btn btn-primary"); yes.onclick = () => done(true);
    row.appendChild(no); row.appendChild(yes);
    panel.appendChild(head); panel.appendChild(body); panel.appendChild(row);
    overlay.appendChild(panel);
    root.appendChild(overlay);
    unbind = ui.bindDismiss(overlay, panel, () => done(false));
  });
}

export function mount(root, deps) {
  ({ doc, ui } = deps);
  btn = ui.btn;
  Object.assign(root, { notice, confirmBox });
}

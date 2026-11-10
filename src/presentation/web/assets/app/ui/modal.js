/* 目的：主区弹层，可叠层（向导 → agent 编辑 → 模块选择器）；closeModal 保持「全关再开一层」的旧语义。
 * 管：层的堆叠顺序、Esc / 点遮罩只关最上面一层、关层时统一退订、共享上下文的 setMsg / close / onRegistryChange / mutate。
 * 不管：各弹层里放什么；登记处写操作的具体 URL 与刷新（经 app.actionUrl / app.refreshState 晚绑定）。
 * 联动：由 app/main.js 装配；关闭判据用 ui/overlays.js 的 bindDismiss，按钮用 ui/buttons.js。 */

let doc, api, ui, app;
let btn;
const actionUrl = (...a) => app.actionUrl(...a);
const onRegistryChange = (...a) => app.onRegistryChange(...a);
const refreshState = (...a) => app.refreshState(...a);
const $ = (sel) => doc.querySelector(sel);

let modalLayers = [];

function detachNode(n) {
  if (!n) return;
  if (n.parentNode && n.parentNode.removeChild) n.parentNode.removeChild(n);
  else if (typeof n.remove === 'function') n.remove();
}

/** 关掉这一层及其上面的所有层；closeModal 就是从最底层开始关。 */
function closeModalLayer(layer) {
  const at = modalLayers.indexOf(layer);
  if (at < 0) return;
  while (modalLayers.length > at) {
    const top = modalLayers.pop();
    if (top.unbind) top.unbind();
    for (const off of top.disposers.splice(0)) {
      try { off(); } catch (e) { console.warn("退订失败：", e); }
    }
    detachNode(top.overlay);
  }
  const root = $('#modal-root');
  if (!modalLayers.length) {
    root.innerHTML = '';
    root.className = 'modal-root';
  }
}

/** 关掉所有弹层（原有 closeModal 的语义）。 */
function closeModal() {
  if (!modalLayers.length) {
    const root = $('#modal-root');
    if (root) { root.innerHTML = ''; root.className = 'modal-root'; }
    return;
  }
  closeModalLayer(modalLayers[0]);
}

/** 叠一层（没有下层时就是普通弹层）。返回的 ctx.close 只关自己这一层。 */
function openModalLayer(title, build, wide) {
  const root = $('#modal-root');
  root.className = 'modal-root open';
  const overlay = doc.createElement('div'); overlay.className = 'modal-overlay';
  const panel = doc.createElement('div'); panel.className = 'modal-panel' + (wide ? ' wide' : '');
  const head = doc.createElement('div'); head.className = 'modal-head';
  const h = doc.createElement('div'); h.className = 'modal-title'; h.textContent = title;
  const close = btn('✕', 'modal-close');
  const layer = { overlay, disposers: [], unbind: null };
  const closeThis = () => closeModalLayer(layer);
  close.onclick = closeThis;
  head.appendChild(h); head.appendChild(close);
  const body = doc.createElement('div'); body.className = 'modal-body';
  const msg = doc.createElement('div'); msg.className = 'modal-msg';
  panel.appendChild(head); panel.appendChild(body); panel.appendChild(msg);
  overlay.appendChild(panel);
  root.appendChild(overlay);
  layer.unbind = ui.bindDismiss(overlay, panel, closeThis);
  modalLayers.push(layer);
  const ctx = {
    body,
    setMsg: (t, isErr) => { msg.textContent = t || ''; msg.className = isErr ? 'modal-msg err' : 'modal-msg'; },
    close: closeThis,
    // 订阅登记处变化；关层时统一退订。
    onRegistryChange: (fn) => {
      const off = onRegistryChange(fn);
      layer.disposers.push(off);
      return off;
    },
    // 登记处写操作只有这一条：动作 → 重拉状态（指纹变了会通知所有已打开的分页）。
    mutate: async (action, body) => {
      const out = await api('POST', actionUrl(action), body);
      await refreshState();
      return out;
    },
  };
  build(ctx);
  return ctx;
}

/** 开一层新的主弹层（保留原有替换语义）。 */
function openModal(title, build, wide) {
  closeModal();
  return openModalLayer(title, build, wide);
}

export function mount(root, deps) {
  ({ doc, api, ui, app } = deps);
  btn = ui.btn;
  Object.defineProperty(root, "modalLayers", { get: () => modalLayers, configurable: true, enumerable: true });
  Object.assign(root, { detachNode, closeModalLayer, closeModal, openModalLayer, openModal });
}

/* 目的：DOM 构建的最小原语（绑定宿主全局的 `el` / `attr` / `h`）。
 * 管：建元素、设属性（宿主没有 setAttribute 时退回 ariaLabel）、按 props 建节点树。
 * 不管：组件形状（`app/ui/`）、业务状态；不读任何 API。
 * 联动：宿主全局由调用方传入（浏览器是 window，冒烟是桩沙箱）；只用冒烟桩支持的 DOM API。 */

/**
 * @param {any} g 宿主全局（含 document）
 */
export function createDom(g) {
  const doc = g.document;

  function el(tag, cls, text) {
    const n = doc.createElement(tag || 'div');
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  function attr(n, name, value) {
    if (typeof n.setAttribute === 'function') n.setAttribute(name, value);
    else if (name === 'aria-label') n.ariaLabel = value;
  }

  /**
   * 按 props 建一个元素：`class` / `text` / `attrs` / `dataset` / `on`（事件表）是保留键，其余直接赋属性。
   * 子节点只收元素（null / false 跳过），文字走 `text`。
   */
  function h(tag, props, ...children) {
    const p = props || {};
    const n = el(tag, p.class, p.text);
    for (const k of Object.keys(p)) {
      if (k === 'class' || k === 'text') continue;
      if (k === 'attrs') { for (const a of Object.keys(p.attrs)) attr(n, a, p.attrs[a]); continue; }
      if (k === 'dataset') { Object.assign(n.dataset, p.dataset); continue; }
      if (k === 'on') { for (const t of Object.keys(p.on)) n.addEventListener(t, p.on[t]); continue; }
      n[k] = p[k];
    }
    for (const c of children.flat()) if (c) n.appendChild(c);
    return n;
  }

  return { g, doc, el, attr, h };
}

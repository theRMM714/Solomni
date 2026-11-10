/* 目的：DOM 构建的最小原语（绑定宿主全局的 `el` / `attr`）。
 * 管：建元素、设属性（宿主没有 setAttribute 时退回 ariaLabel）。
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

  return { g, doc, el, attr };
}

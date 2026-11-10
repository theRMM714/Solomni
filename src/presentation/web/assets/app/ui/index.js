/* 目的：组件库的组装入口（`createUi(g)` 返回全部组件）。
 * 管：用宿主全局建 DOM 原语，再装配图标 / 按钮 / 表单 / 浮层。
 * 不管：业务状态与 API：组件只收参数、发回调。
 * 联动：`app/main.js` 建它并挂到 `app.ui`；单个组件见同目录各文件；冒烟见 `app/ui/ui.smoke.cjs`。 */

import { createDom } from '../core/dom.js';
import { createIcons } from './icons.js';
import { createButtons } from './buttons.js';
import { createForms } from './forms.js';
import { createOverlays } from './overlays.js';

export function createUi(g) {
  const dom = createDom(g);
  const icons = createIcons(dom);
  const overlays = createOverlays(dom, icons);
  return {
    el: dom.el,
    ...icons,
    ...createButtons(dom, icons),
    ...createForms(dom, overlays),
    ...overlays,
  };
}

/* 目的：把 ES Module 交给经典脚本 `app.js`（经典脚本不能 import，故挂到全局 `solomni` 与 `ui` 上）。
 * 管：用宿主的 `fetch` / `setTimeout` / `document` 装配 `api`、状态、事件流工厂与组件库；浏览器与冒烟沙箱走同一段装配。
 * 不管：任何业务或渲染；不读 DOM。
 * 联动：浏览器由 `app/main.js` 调用；冒烟（`*.smoke.cjs`）在载入 `app.js` 前对沙箱调用。 */

import { createApi } from './core/http.js';
import { createEventStream } from './core/events.js';
import * as stateStore from './stores/state.js';
import { createUi } from './ui/index.js';

export function installBridge(g) {
  const fetchImpl = (url, init) => g.fetch(url, init);
  const state = stateStore.createState();
  g.ui = createUi(g);
  g.solomni = {
    api: createApi(fetchImpl),
    state,
    stores: stateStore,
    createEventStream: (hooks) => createEventStream({
      fetch: fetchImpl,
      setTimeout: (fn, ms) => g.setTimeout(fn, ms),
      ...hooks,
    }),
  };
  return g.solomni;
}

/* 目的：迁移期把新模块交给仍是经典脚本的 `app.js`（挂到全局 `solomni` 上）。
 * 管：用宿主的 `fetch` / `setTimeout` 装配 `api`、状态与事件流工厂；浏览器与冒烟沙箱走同一段装配。
 * 不管：任何业务或渲染；不读 DOM。
 * 联动：浏览器由 `app/main.js` 调用；冒烟（`*.smoke.cjs`）在载入 `app.js` 前对沙箱调用；
 *   `app.js` 全部迁入模块后连同本文件一起删除。 */

import { createApi } from './core/http.js';
import { createEventStream } from './core/events.js';
import * as stateStore from './stores/state.js';

export function installBridge(g) {
  const fetchImpl = (url, init) => g.fetch(url, init);
  const state = stateStore.createState();
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

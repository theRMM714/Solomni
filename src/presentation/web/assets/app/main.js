/* 目的：浏览器端的模块入口。
 * 管：在经典脚本 `app.js` 执行前装好迁移桥（模块脚本与 defer 脚本按文档顺序执行）。
 * 不管：业务、渲染、启动顺序之外的任何事。
 * 联动：`index.html` 先引本文件（type="module"），再引 defer 的 `md.js` / `ui.js` / `app.js`；桥见 `app/bridge.js`。 */

import { installBridge } from './bridge.js';

installBridge(globalThis);

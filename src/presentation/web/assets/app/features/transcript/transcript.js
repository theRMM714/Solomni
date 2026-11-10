/* 目的：转录功能的聚合装配点——事件吸收、行部件、定稿气泡、流式区、裁决卡与整页重画。
 * 管：按依赖顺序把 ./events.js ./parts.js ./turn.js ./stream.js ./cards.js ./page.js 挂到同一个根对象上，
 *   并把无依赖的纯映射 ./model.js 一起并入。
 * 不管：不实现任何吸收或渲染逻辑；模块之间只经依赖注入的 app 晚绑定互调（见各文件头）。
 * 联动：由 app/main.js 的唯一组装点调用；挂上去的入口供其它功能（向导、输入区、长轮询）经 app.* 使用。 */

import * as model from './model.js';
import { mount as mountEvents } from './events.js';
import { mount as mountParts } from './parts.js';
import { mount as mountTurn } from './turn.js';
import { mount as mountStream } from './stream.js';
import { mount as mountCards } from './cards.js';
import { mount as mountPage } from './page.js';

/// 挂载：纯映射直接并入，其余按依赖方向（部件 → 气泡 → 流式 → 整帧）依次装配。
export function mount(root, deps) {
  Object.assign(root, model);
  mountEvents(root, deps);
  mountParts(root, deps);
  mountTurn(root, deps);
  mountStream(root, deps);
  mountCards(root, deps);
  mountPage(root, deps);
}

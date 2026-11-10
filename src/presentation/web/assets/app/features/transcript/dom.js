/* 目的：转录流式容器的归属状态——定稿容器、流式容器、打字指示、流式渲染节拍与已渲染计数。
 * 管：唯一的可变持有者（boxes）：谁创建、重置、切换会话都改这里；跨模块共享只此一处。
 * 不管：不创建节点、不渲染、不吸收事件；建节点在 ./turn.js 与 ./stream.js。
 * 联动：turn.js（整帧重建、只追加）与 stream.js（流式增量）读写同一份状态，避免两模块互相 import。 */

/// 容器当前属于哪个会话、已渲染到第几轮、打字指示是否挂上、合并渲染节拍是否已排。
export const boxes = {
  owner: null,
  done: null,
  live: null,
  typing: null,
  tickScheduled: false,
  doneState: { owner: null, count: 0, show: null, note: '', last: null },
};

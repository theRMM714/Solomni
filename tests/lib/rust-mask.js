/**
 * Rust 源码掩码：把注释与字符串抹成空白（保留长度与换行），供门禁判定"生产代码里是否出现某个记号"。
 * 目的：注释或字符串里的 ureq / std::fs 既不是依赖也不是 IO 调用；同时不许把生命周期 'a 当成字符字面量，
 *   否则会把后面的代码一起吞掉，门禁就会出现静默漏报。
 * 覆盖：行注释、可嵌套块注释、普通与 byte 字符串（含转义）、raw string（r / br / cr 前缀与 # 定界）、
 *   字符与 byte 字符字面量。只做词法遮蔽，不求值宏与 cfg；无法确定是字面量的 ' 按生命周期处理。
 */
"use strict";

const IDENT = /[A-Za-z0-9_]/;

function isIdent(ch) {
  return ch !== undefined && IDENT.test(ch);
}

/** 保留换行（\n、\r），其余位置写成空格；长度与原串一致。 */
function blank(out, text, from, to) {
  for (let k = from; k < to && k < text.length; k++) {
    const ch = text[k];
    if (ch !== "\n" && ch !== "\r") out[k] = " ";
  }
}

/**
 * 以 text[q] 的 '\'' 开头的字符字面量的结束位置（开区间，即下一个字符的下标）；不是字符字面量给 -1。
 * 字面量恰好一个码点（可带转义）；后面没有紧随的 ' 就是生命周期 'a，绝不能按字面量吞代码。
 */
function charLiteralEnd(text, q) {
  const n = text.length;
  let j = q + 1;
  if (j >= n) return -1;
  if (text[j] === "\\") {
    j++;
    const e = text[j];
    if (e === undefined) return -1;
    if (e === "x") {
      j += 3; // 跳过 'x' 与两位十六进制；越界时下面的引号判断兜底
    } else if (e === "u") {
      j++;
      if (text[j] !== "{") return -1;
      while (j < n && text[j] !== "}") j++;
      if (j >= n) return -1;
      j++;
    } else {
      j++;
    }
  } else {
    const cp = text.codePointAt(j);
    if (cp === undefined || cp === 0x0a || cp === 0x0d || cp === 0x27) return -1;
    j += cp > 0xffff ? 2 : 1;
  }
  return text[j] === "'" ? j + 1 : -1;
}

/** 抹掉注释与字面量；返回与入参等长、换行位置不变的串。 */
function mask(text) {
  const out = text.split("");
  const n = text.length;
  let i = 0;
  while (i < n) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "/") {
      let j = i;
      while (j < n && text[j] !== "\n") j++;
      blank(out, text, i, j);
      i = j;
      continue;
    }
    if (c === "/" && text[i + 1] === "*") {
      let depth = 0;
      let j = i;
      while (j < n) {
        if (text[j] === "/" && text[j + 1] === "*") {
          depth++;
          j += 2;
          continue;
        }
        if (text[j] === "*" && text[j + 1] === "/") {
          depth--;
          j += 2;
          if (depth === 0) break;
          continue;
        }
        j++;
      }
      blank(out, text, i, j);
      i = j;
      continue;
    }
    // raw string：r" / r#" / r##"…"##，以及 byte / C 前缀 br、cr；前缀必须是记号起点（不能是标识符的一部分）。
    if ((c === "r" || ((c === "b" || c === "c") && text[i + 1] === "r")) && !isIdent(text[i - 1])) {
      let p = c === "r" ? i + 1 : i + 2;
      let hashes = 0;
      while (text[p] === "#") {
        hashes++;
        p++;
      }
      if (text[p] === '"') {
        const close = '"' + "#".repeat(hashes);
        const found = text.indexOf(close, p + 1);
        const stop = found === -1 ? n : found + close.length;
        blank(out, text, i, stop);
        i = stop;
        continue;
      }
    }
    // 普通字符串与 byte string：b"…"；转义字符整体跳过，裸换行按未闭合收尾。
    if (c === '"' || (c === "b" && text[i + 1] === '"' && !isIdent(text[i - 1]))) {
      const open = c === '"' ? i : i + 1;
      let j = open + 1;
      while (j < n) {
        const d = text[j];
        if (d === "\\") {
          j += 2;
          continue;
        }
        if (d === '"' || d === "\n") {
          if (d === '"') j++;
          break;
        }
        j++;
      }
      blank(out, text, i, j);
      i = j;
      continue;
    }
    // 字符 / byte 字符字面量；判不出来就是生命周期，只抹引号本身。
    if (c === "'" || (c === "b" && text[i + 1] === "'" && !isIdent(text[i - 1]))) {
      const q = c === "'" ? i : i + 1;
      const end = charLiteralEnd(text, q);
      if (end !== -1) {
        blank(out, text, i, end);
        i = end;
        continue;
      }
      i++;
      continue;
    }
    i++;
  }
  return out.join("");
}

module.exports = { mask };

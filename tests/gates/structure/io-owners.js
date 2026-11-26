/**
 * IO 面门禁：一类 IO 只有一份实现，业务单元经 `kernel::api` 取，不在自己家里再装一套。
 * 判据一：每个类别的实现文件必须真实存在（改名后旧表会让门禁空转）。
 * 判据二：命中类别特征的调用点只许在实现文件里，或表里逐条写明的例外里（例外要写用途）。
 * 判据三：命中即失败、零豁免；先剥注释与内联测试模块再匹配，别把文档示例当实现。
 * 不判：一次性工具进程（`Command::new` 走 ProcessRunner 端口那套，见 docs/tools/README.md）、
 *   环境读取（`structure/env-surface` 判）、盘面访问（`structure/disk-owners` 判）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { maskComments } = require("../../lib/rust-mask.js");

const NET = "src/kernel/detail/io/net";

/** 类别表：特征 → 唯一实现 → 逐条写明的例外。 */
const RULES = [
  {
    name: "入站监听",
    pattern: /TcpListener::bind|tiny_http::Server/g,
    impl: [NET + "/loopback.rs", NET + "/tcp.rs"],
    allow: {
      "src/kernel/detail/container/windows/relay.rs":
        "容器内回环中继绑 127.0.0.2（同一条回环上的另一个地址），不是产品端口",
    },
  },
  {
    name: "回环客户端",
    pattern: /TcpStream::connect/g,
    impl: [NET + "/tcp.rs"],
    allow: {},
  },
  {
    name: "出站装配",
    pattern: /ureq::Agent::config_builder|ureq::AgentBuilder|reqwest::Client::builder/g,
    impl: [NET + "/outbound.rs", NET + "/backend.rs"],
    allow: {
      "src/units/business/llm/detail/http_stream.rs":
        "流式栈必须 async（reqwest 自建 client），TLS 仍取 kernel::api::net::configure_reqwest",
    },
  },
];

/** 测试自己的夹具树，不按产品实现判。 */
function isTestFile(f) {
  if (f === "src/tests.rs" || f.startsWith("src/tests/")) return true;
  const segs = f.split("/");
  return segs.includes("tests") || segs[segs.length - 1].endsWith("_tests.rs");
}

function walk(root) {
  const files = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".rs")) files.push(rel(root, p));
    }
  };
  walkDir(path.join(root, "src"));
  return files;
}

/** 目的：扫一遍实现与调用点，返回问题清单与一行说明。 */
function ioOwners(root) {
  const problems = [];
  for (const rule of RULES) {
    for (const f of rule.impl) {
      if (!fs.existsSync(path.join(root, f))) {
        problems.push("IO 面：「" + rule.name + "」的实现 " + f + " 不存在——本门禁的类别表过期了");
      }
    }
  }
  let scanned = 0;
  for (const f of walk(root)) {
    if (isTestFile(f)) continue;
    scanned++;
    const prod = maskComments(stripTestModules(fs.readFileSync(path.join(root, f), "utf8")));
    prod.split("\n").forEach((line, i) => {
      for (const rule of RULES) {
        if (rule.impl.includes(f) || Object.prototype.hasOwnProperty.call(rule.allow, f)) continue;
        for (const m of line.matchAll(rule.pattern)) {
          problems.push(
            "IO 面：" +
              f +
              ":" +
              (i + 1) +
              " 自己实现了「" +
              rule.name +
              "」（" +
              m[0] +
              "）——只有 " +
              rule.impl.join(" / ") +
              " 这一份实现，业务经 kernel::api::net 取",
          );
        }
      }
    });
  }
  return {
    problems,
    notes: ["IO 面：扫了 " + scanned + " 个源文件，判 " + RULES.length + " 类原语（类别表读自本文件）"],
  };
}

module.exports = { ioOwners };

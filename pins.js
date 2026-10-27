#!/usr/bin/env node
/**
 * 目的：维护 links.yaml 与 dependencies.yaml 里的校验值，把"手抄哈希"变成"按上游校验源生成"。
 * 管：update（刷新现有 pin）、check（只读比对）、bump（smbd 的 URL + 哈希一起换）。
 * 不管：下载依赖、安装依赖、产品运行期校验（那些在 provision / env.js / setup-toolchains.js）。
 * 联动：links.yaml、dependencies.yaml、.github/workflows/pins-check.yml；规则见 docs/provision/README.md。
 */
"use strict";
const fs = require("fs");
const path = require("path");

const ROOT = __dirname;
const LINKS = path.join(ROOT, "links.yaml");
const DEPS = path.join(ROOT, "dependencies.yaml");
const SMBD_REPO = "theRMM714/rust-smb-server";
const ZIG_INDEX = "https://ziglang.org/download/index.json";
const USER_AGENT = "solomni-pins/1";

function readText(file) {
  return fs.readFileSync(file, "utf8");
}

/** 目的：读扁平的"键: 值"台账，保留行号供原地重写；注释与空行跳过。 */
function parseFlatText(text) {
  const lines = text.split("\n");
  const entries = new Map();
  lines.forEach((line, i) => {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) return;
    const m = line.match(/^([A-Za-z0-9_.-]+):\s*(\S+)\s*$/);
    if (!m) throw new Error("不是扁平的键: 值（第 " + (i + 1) + " 行）：" + line);
    if (entries.has(m[1])) throw new Error("重复键：" + m[1]);
    entries.set(m[1], { value: m[2], line: i });
  });
  return { lines, entries };
}

/** 目的：读 dependencies.yaml 的条目块，只取 id / source / sha256 三个字段。 */
function parseDepsText(text) {
  const lines = text.split("\n");
  const blocks = [];
  let current = null;
  lines.forEach((line, i) => {
    const start = line.match(/^\s*-\s+id:\s*(\S+)\s*$/);
    if (start) {
      current = { id: start[1], start: i, fields: {} };
      blocks.push(current);
      return;
    }
    if (!current) return;
    const m = line.match(/^\s+(source|sha256):\s*(\S+)\s*$/);
    if (m) current.fields[m[1]] = { value: m[2], line: i };
  });
  for (const b of blocks) {
    b.source = b.fields.source;
    b.sha256 = b.fields.sha256;
  }
  return { lines, blocks };
}

function basenameOfUrl(url) {
  const last = new URL(url).pathname.split("/").filter(Boolean).pop() || "";
  return decodeURIComponent(last);
}

/** 目的：从 GitHub release 资产 URL 拆出 owner / repo / tag / 资产名。 */
function githubRelease(url) {
  const m = url.match(/^https:\/\/github\.com\/([^/]+)\/([^/]+)\/releases\/download\/([^/]+)\/(.+)$/);
  if (!m) return null;
  return {
    owner: m[1],
    repo: m[2],
    tag: decodeURIComponent(m[3]),
    asset: decodeURIComponent(m[4]),
  };
}

/** 目的：把一条链接键映射到它的上游校验源。 */
function linkTarget(key, url) {
  if (key === "winlibs.upstream.asset" || key === "smbd.windows") {
    return { kind: "sidecar", label: key, url: url + ".sha256", github: githubRelease(url) };
  }
  if (key.startsWith("toolchain.node.")) {
    return {
      kind: "shasums",
      label: key,
      url: new URL("SHASUMS256.txt", new URL(".", url)).href,
      name: basenameOfUrl(url),
    };
  }
  if (key.startsWith("toolchain.zig.")) {
    return { kind: "zig", label: key, url: ZIG_INDEX, tarball: url };
  }
  if (key.startsWith("toolchain.python.")) {
    const gh = githubRelease(url);
    if (!gh) throw new Error(key + " 不是 GitHub release 资产：" + url);
    return { kind: "github", label: key, owner: gh.owner, repo: gh.repo, tag: gh.tag, asset: gh.asset };
  }
  return { kind: "sidecar", label: key, url: url + ".sha256", github: githubRelease(url) };
}

/** 目的：把 release:<tag>/<资产> 解析成下载 URL 与 GitHub release 坐标。 */
function releaseTarget(base, source) {
  if (!source.startsWith("release:")) throw new Error("不是自托管 release 源：" + source);
  const rest = source.slice("release:".length);
  const url = base.replace(/\/+$/, "") + "/" + rest;
  const gh = githubRelease(url);
  if (!gh) throw new Error("自托管源不是 GitHub release 资产：" + url);
  return { kind: "release", label: source, owner: gh.owner, repo: gh.repo, tag: gh.tag, asset: gh.asset, url };
}

function firstSha(text) {
  const m = text.match(/\b[0-9a-f]{64}\b/i);
  return m ? m[0].toLowerCase() : null;
}

/** 目的：解析 SHASUMS256.txt / SHA256SUMS 的"哈希  文件名"行。 */
function parseShasums(text) {
  const map = new Map();
  for (const raw of text.split(/\r?\n/)) {
    const m = raw.match(/^\s*([0-9a-f]{64})\s+\*?(.+?)\s*$/i);
    if (m) map.set(path.basename(m[2]), m[1].toLowerCase());
  }
  return map;
}

function shaFromShasums(text, name) {
  const map = parseShasums(text);
  return map.get(name) || null;
}

/** 目的：按 tarball URL 在 Zig 的 index.json 里找 shasum（不靠平台名拼接）。 */
function findZigShasum(index, tarball) {
  for (const versions of Object.values(index)) {
    if (!versions || typeof versions !== "object") continue;
    for (const info of Object.values(versions)) {
      if (info && info.tarball === tarball && info.shasum) return String(info.shasum).toLowerCase();
    }
  }
  return null;
}

function replaceTrailingValue(line, value) {
  const m = line.match(/^(.*?:\s*)\S+(\s*)$/);
  if (!m) throw new Error("这一行没有可替换的值：" + line);
  return m[1] + value + m[2];
}

async function fetchText(url, optional) {
  const headers = { "User-Agent": USER_AGENT };
  if (url.startsWith("https://api.github.com/")) {
    headers.Accept = "application/vnd.github+json";
    if (process.env.GITHUB_TOKEN) headers.Authorization = "Bearer " + process.env.GITHUB_TOKEN;
  }
  const res = await fetch(url, {
    headers,
    redirect: "follow",
    signal: AbortSignal.timeout(30000),
  });
  if (optional && res.status === 404) return null;
  if (!res.ok) throw new Error(url + " 返回 HTTP " + res.status);
  return await res.text();
}

const releaseCache = new Map();

/** 目的：一个 release 的 SHA256SUMS（没有就给 null）；同一个 tag 只取一次。 */
function releaseSums(owner, repo, tag) {
  const key = owner + "/" + repo + "@" + tag + "#sums";
  if (!releaseCache.has(key)) {
    releaseCache.set(
      key,
      (async () => {
        const url =
          "https://github.com/" + owner + "/" + repo + "/releases/download/" + encodeURIComponent(tag) + "/SHA256SUMS";
        const text = await fetchText(url, true);
        return text === null ? null : parseShasums(text);
      })(),
    );
  }
  return releaseCache.get(key);
}

/** 目的：一个 release 的资产 digest 表；SHA256SUMS 缺失时回退 GitHub API。 */
function releaseAssets(owner, repo, tag) {
  const key = owner + "/" + repo + "@" + tag + "#assets";
  if (!releaseCache.has(key)) {
    releaseCache.set(
      key,
      (async () => {
        const url =
          "https://api.github.com/repos/" + owner + "/" + repo + "/releases/tags/" + encodeURIComponent(tag);
        const json = JSON.parse(await fetchText(url));
        const map = new Map();
        for (const a of json.assets || []) {
          const digest = (a.digest || "").replace(/^sha256:/, "").toLowerCase();
          if (digest) map.set(a.name, digest);
        }
        return map;
      })(),
    );
  }
  return releaseCache.get(key);
}

async function releaseDigest(owner, repo, tag, asset) {
  const sums = await releaseSums(owner, repo, tag);
  if (sums) {
    const hit = sums.get(asset);
    if (hit) return hit;
    throw new Error(owner + "/" + repo + "@" + tag + " 的 SHA256SUMS 没有 " + asset);
  }
  const assets = await releaseAssets(owner, repo, tag);
  const hit = assets.get(asset);
  if (!hit) throw new Error(owner + "/" + repo + "@" + tag + " 没有可用的 " + asset + " digest");
  return hit;
}

function collectTargets() {
  const links = parseFlatText(readText(LINKS));
  const deps = parseDepsText(readText(DEPS));
  const base = links.entries.get("release.base");
  if (!base) throw new Error("links.yaml 缺 release.base");
  const targets = [];
  for (const [key, entry] of links.entries) {
    if (!key.endsWith(".sha256")) continue;
    const baseKey = key.slice(0, -".sha256".length);
    const url = links.entries.get(baseKey);
    if (!url) throw new Error("links.yaml 的校验值没有对应的链接键：" + key);
    targets.push({
      ...linkTarget(baseKey, url.value),
      label: baseKey,
      file: LINKS,
      line: entry.line,
      old: entry.value,
    });
  }
  for (const block of deps.blocks) {
    if (!block.source || !block.source.value.startsWith("release:")) continue;
    if (!block.sha256) continue;
    targets.push({
      ...releaseTarget(base.value, block.source.value),
      label: block.id,
      file: DEPS,
      line: block.sha256.line,
      old: block.sha256.value,
    });
  }
  return { links, deps, targets };
}

async function resolveTarget(t) {
  switch (t.kind) {
    case "sidecar": {
      const text = await fetchText(t.url, true);
      const hash = text ? firstSha(text) : null;
      if (hash) return hash;
      // 有的发布者只给 GitHub API digest，没有单独的 .sha256 资产（旧 smbd 仓就是这样）。
      if (t.github) return await releaseDigest(t.github.owner, t.github.repo, t.github.tag, t.github.asset);
      throw new Error(t.url + " 里没有 sha256");
    }
    case "shasums": {
      const hash = shaFromShasums(await fetchText(t.url), t.name);
      if (!hash) throw new Error(t.url + " 里没有 " + t.name);
      return hash;
    }
    case "zig": {
      const hash = findZigShasum(JSON.parse(await fetchText(t.url)), t.tarball);
      if (!hash) throw new Error(t.url + " 里没有 " + t.tarball);
      return hash;
    }
    case "github":
    case "release":
      return await releaseDigest(t.owner, t.repo, t.tag, t.asset);
    default:
      throw new Error("未知的校验源类型：" + t.kind);
  }
}

async function resolveAll(targets) {
  const results = [];
  for (const t of targets) {
    try {
      const hash = await resolveTarget(t);
      results.push({ ...t, hash });
      console.log("ok   " + t.label.padEnd(56) + " " + hash.slice(0, 12) + "…");
    } catch (e) {
      results.push({ ...t, error: e.message });
      console.error("FAIL " + t.label + "：" + e.message);
    }
  }
  return results;
}

function applyResults(results, write) {
  const failed = results.filter((r) => r.error);
  const changed = results.filter((r) => r.hash && r.hash !== r.old.toLowerCase());
  if (failed.length) {
    console.error("上游校验值取不到（" + failed.length + " 条），不写文件。");
    return 2;
  }
  if (!write) {
    if (!changed.length) {
      console.log("PINS-CHECK-OK");
      return 0;
    }
    for (const r of changed) {
      console.error("不符 " + r.label + "：台账 " + r.old + "，上游 " + r.hash);
    }
    return 1;
  }
  if (!changed.length) {
    console.log("PINS-UPDATE-OK（无变化）");
    return 0;
  }
  const byFile = new Map();
  for (const r of changed) {
    if (!byFile.has(r.file)) byFile.set(r.file, readText(r.file).split("\n"));
    const lines = byFile.get(r.file);
    lines[r.line] = replaceTrailingValue(lines[r.line], r.hash);
  }
  for (const [file, lines] of byFile) fs.writeFileSync(file, lines.join("\n"));
  console.log("PINS-UPDATE-OK（" + changed.length + " 条）");
  return 0;
}

async function update(write) {
  const { targets } = collectTargets();
  return applyResults(await resolveAll(targets), write);
}

async function bumpSmbd(tag) {
  if (!/^smbd-v[0-9A-Za-z._-]+$/.test(tag)) throw new Error("tag 形如 smbd-v0.1.1：" + tag);
  const links = parseFlatText(readText(LINKS));
  const url = links.entries.get("smbd.windows");
  const sha = links.entries.get("smbd.windows.sha256");
  if (!url || !sha) throw new Error("links.yaml 缺 smbd.windows / smbd.windows.sha256");
  const asset = basenameOfUrl(url.value);
  const nextUrl = "https://github.com/" + SMBD_REPO + "/releases/download/" + tag + "/" + asset;
  const hash = firstSha(await fetchText(nextUrl + ".sha256"));
  if (!hash) throw new Error(nextUrl + ".sha256 里没有 sha256");
  const lines = links.lines.slice();
  lines[url.line] = replaceTrailingValue(lines[url.line], nextUrl);
  lines[sha.line] = replaceTrailingValue(lines[sha.line], hash);
  fs.writeFileSync(LINKS, lines.join("\n"));
  console.log("PINS-BUMP-OK " + tag);
  console.log("  URL     " + nextUrl);
  console.log("  sha256  " + hash);
  return 0;
}

function usage() {
  return [
    "用法：node pins.js <子命令>",
    "",
    "子命令：",
    "  update                 按上游校验源刷新 links.yaml / dependencies.yaml 的哈希",
    "  check                  只读比对，当前 pin 与上游不一致时退出 1；网络错误退出 2",
    "  bump smbd <tag>        同时换 smbd 的 URL 与哈希（如 smbd-v0.1.1）",
    "",
    "校验源：winlibs / smbd 用 <url>.sha256；Node 用 SHASUMS256.txt；Zig 用 index.json；",
    "Python 与自托管 release 用 SHA256SUMS，缺失时回退 GitHub API 的资产 digest。",
    "GitHub API 受限时设置 GITHUB_TOKEN。",
  ].join("\n");
}

async function main(argv) {
  const args = argv || process.argv.slice(2);
  const cmd = args[0];
  if (!cmd || cmd === "--help" || cmd === "-h") {
    console.log(usage());
    return cmd ? 0 : 2;
  }
  if (cmd === "update") return await update(true);
  if (cmd === "check") return await update(false);
  if (cmd === "bump") {
    if (args[1] !== "smbd" || !args[2] || args.length !== 3) {
      console.error("用法：node pins.js bump smbd <tag>");
      return 2;
    }
    return await bumpSmbd(args[2]);
  }
  console.error("没有这个子命令：" + cmd);
  console.error(usage());
  return 2;
}

module.exports = {
  parseFlatText,
  parseDepsText,
  githubRelease,
  linkTarget,
  releaseTarget,
  firstSha,
  parseShasums,
  shaFromShasums,
  findZigShasum,
  replaceTrailingValue,
  main,
};

if (require.main === module) {
  main()
    .then((code) => process.exit(code))
    .catch((e) => {
      console.error("pins.js 失败：" + e.message);
      process.exit(2);
    });
}

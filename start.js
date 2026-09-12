#!/usr/bin/env node
/**
 * Solomni launcher: platform check -> toolchain/build readiness -> run.
 * Default CLI; -webUI starts the Web UI. No business logic here.
 * Cross-platform: node start.js [-webUI] [--release] [--root <dir>] [--web-port <port>]
 * Rust missing? Ask, then install via rustup (GNU toolchain on Windows: no MSVC needed).
 * Windows GNU gap (rust-lang/rust#140704): windows-sys needs a WORKING dlltool.exe
 * (one that can spawn its assembler). Preflight probes FIXED paths only (no disk scan)
 * AND trial-runs dlltool; if missing or broken, ask consent, then install portable
 * winlibs MinGW (complete binutils) into .tools/mingw64.
 */
"use strict";
const { spawnSync, spawn } = require("child_process");
const readline = require("readline");
const path = require("path");
const fs = require("fs");

const ROOT = __dirname;
const IS_WIN = process.platform === "win32";
const EXE = IS_WIN ? "solomni.exe" : "solomni";
const RELEASE = process.argv.includes("--release");
const PROFILE = RELEASE ? "release" : "debug";
const BIN = path.join(ROOT, "target", PROFILE, EXE);
const BUNDLED_CARGO = path.join(ROOT, "platform", "linux", "cargo", "bin", IS_WIN ? "cargo.exe" : "cargo");
const TOOLS = path.join(ROOT, ".tools");
const WINLIBS_BIN = path.join(TOOLS, "mingw64", "bin");
// Project-local rustup homes (same layout as the bundled linux toolchain):
const P_RUSTUP = path.join(ROOT, "platform", "windows", "rustup");
const P_CARGO = path.join(ROOT, "platform", "windows", "cargo");
// Release asset base for third-party redistributions (winlibs zip).
// Placeholder repo: fill in once the release is published.
const REL_BASE = "https://github.com/theRMM714/Solomni/releases/download/dependencies/";

const log = (m) => console.log("[start] " + m);
const die = (m) => { console.error("[start] " + m); process.exit(1); };
let EXTRA_PATH = []; // dlltool location found by preflight; consumed by cargoEnv.

function locateDlltool(cargo) {
  // Fixed-path probes only - never a disk scan. Sources, in order:
  // 1. toolchain self-contained dirs (older toolchains shipped dlltool there)
  // 2. conventional MSYS2 install dirs
  // 3. project-local winlibs install (.tools/mingw64)
  // 4. any directory that "where dlltool" already reports
  // Returns the first directory that actually contains dlltool.exe, or null.
  const dirs = [];
  const add = (p) => { if (p && dirs.indexOf(p) < 0) dirs.push(p); };
  const rustc = path.join(path.dirname(cargo), IS_WIN ? "rustc.exe" : "rustc");
  const v = spawnSync(rustc, ["--print", "sysroot"], { encoding: "utf8" });
  const sysroot = (v.stdout || "").trim();
  if (sysroot && fs.existsSync(sysroot)) {
    const gnu = path.join(sysroot, "lib", "rustlib", "x86_64-pc-windows-gnu", "bin");
    add(path.join(gnu, "self-contained"));
    add(path.join(gnu, "gdb.debug"));
    add(gnu);
  }
  add("C:/msys64/mingw64/bin");
  add("C:/msys2/mingw64/bin");
  add(WINLIBS_BIN);
  const w = spawnSync("where", ["dlltool"], { encoding: "utf8" });
  for (const line of (w.stdout || "").split(/\r?\n/)) {
    const t = line.trim();
    if (t && fs.existsSync(t)) add(path.dirname(t));
  }
  for (const d of dirs) {
    if (fs.existsSync(path.join(d, "dlltool.exe"))) return d;
  }
  return null;
}

function dlltoolWorks(dir) {
  // Decisive probe: actually generate a tiny import library. Catches the
  // "dlltool present but its assembler is missing" case (CreateProcess error).
  const os = require("os");
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "solo-dt-"));
  const def = path.join(tmp, "probe.def");
  const lib = path.join(tmp, "probe.lib");
  fs.writeFileSync(def, "LIBRARY kernel32.dll\nEXPORTS\n  GetLastError\n");
  const env = Object.assign({}, process.env);
  env.PATH = [dir, env.PATH || ""].filter(Boolean).join(path.delimiter);
  const r = spawnSync(path.join(dir, "dlltool.exe"),
    ["-d", def, "-D", "kernel32.dll", "-l", lib, "-m", "i386:x86-64", "-f", "--64", "--no-leading-underscore"],
    { encoding: "utf8", env, cwd: tmp });
  try { fs.rmSync(tmp, { recursive: true, force: true }); } catch (e) { /* best effort */ }
  return r.status === 0 && fs.existsSync(lib);
}

function cargoEnv(cargo) {
  // project-local toolchains get explicit HOMEs; system cargo used as-is.
  const env = Object.assign({}, process.env);
  const c = path.resolve(cargo);
  if (c === path.resolve(BUNDLED_CARGO)) {
    env.RUSTUP_HOME = path.join(ROOT, "platform", "linux", "rustup");
    env.CARGO_HOME = path.join(ROOT, "platform", "linux", "cargo");
  } else if (c === path.resolve(path.join(P_CARGO, "bin", IS_WIN ? "cargo.exe" : "cargo"))) {
    env.RUSTUP_HOME = P_RUSTUP;
    env.CARGO_HOME = P_CARGO;
  }
  env.PATH = [path.dirname(cargo)].concat(EXTRA_PATH, env.PATH || "").filter(Boolean).join(path.delimiter);
  return env;
}

function findCargo() {
  // Order: project-local (windows) -> bundled linux -> PATH -> ~/.cargo/bin.
  if (IS_WIN) {
    const pc = path.join(P_CARGO, "bin", "cargo.exe");
    if (fs.existsSync(pc)) return pc;
  }
  if (fs.existsSync(BUNDLED_CARGO)) return BUNDLED_CARGO;
  const exe = IS_WIN ? "cargo.exe" : "cargo";
  for (const d of (process.env.PATH || "").split(path.delimiter)) {
    const c = path.join(d, exe);
    if (fs.existsSync(c)) return c;
  }
  const home = process.env.USERPROFILE || process.env.HOME || "";
  const ru = path.join(home, ".cargo", "bin", exe);
  if (fs.existsSync(ru)) return ru;
  return null;
}

function ask(question) {
  return new Promise((resolve) => {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question(question, (a) => { rl.close(); resolve(a); });
  });
}

function runPS(cmd) {
  return spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", cmd], { stdio: "inherit" });
}

function download(url, dest) {
  // curl.exe ships with Windows (10 1803+); far faster than Invoke-WebRequest and
  // renders a progress bar. -C - resumes a previous partial download instead of
  // restarting from zero. PowerShell fallback for ancient systems.
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (fs.existsSync(curl)) {
    const r = spawnSync(curl, ["-L", "--fail", "--retry", "3", "-C", "-", "--connect-timeout", "30", "-o", dest, url],
      { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(dest) && fs.statSync(dest).size > 1048576) return true;
    log("curl download failed (exit " + r.status + "); keeping partial file for resume; trying PowerShell fallback...");
  }
  runPS("Invoke-WebRequest -UseBasicParsing '" + url + "' -OutFile '" + dest + "'");
  return fs.existsSync(dest) && fs.statSync(dest).size > 1048576;
}

function upstreamWinlibsUrl() {
  // Upstream asset names carry versions (winlibs-x86_64-posix-seh-gcc-...zip), so
  // latest/download/winlibs.zip is a guaranteed 404. Resolve the real name via the API.
  const q = [
    "$ErrorActionPreference = 'Stop'",
    "$r = Invoke-RestMethod 'https://api.github.com/repos/brechtsanders/winlibs_mingw/releases/latest'",
    "$a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match 'seh' -and $_.name -match '[.]zip$' -and $_.name -notmatch 'llvm' } | Select-Object -First 1",
    "if (-not $a) { $a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match '[.]zip$' } | Select-Object -First 1 }",
    "Write-Output $a.browser_download_url"
  ].join("; ");
  const got = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", q], { encoding: "utf8" });
  const url = (got.stdout || "").trim();
  return /^https:/.test(url) ? url : null;
}

function probeSpeed(url) {
  // Sustained-throughput probe: ranged GET of 1MB, curl reports its own average
  // speed (bytes/s) via speed_download. max-time caps slow sources - the partial
  // average is still the honest sustained rate. 0 = unreachable.
  return new Promise((resolve) => {
    const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
    const cmd = IS_WIN ? curl : "curl";
    if (!fs.existsSync(cmd)) { resolve(0); return; }
    const p = spawn(cmd,
      ["-sL", "-r", "0-1048575", "-o", require("os").devNull, "-w", "%{http_code} %{speed_download}",
       "--connect-timeout", "4", "--max-time", "6", url],
      { encoding: "utf8" });
    let out = "";
    p.stdout.on("data", (d) => { out += d; });
    p.on("error", () => resolve(0));
    p.on("close", () => {
      const m = out.trim().match(/^(\d{3}) (\d+(?:\.\d+)?)$/);
      if (m && /^2/.test(m[1])) resolve(parseFloat(m[2]) || 0);
      else resolve(0);
    });
  });
}

async function pickFastest(urls) {
  // Speed test every candidate (sequential: parallel streams would share bandwidth
  // and skew each other's numbers on a thin pipe). Fastest wins; listed order is
  // only the tiebreak. Costs a few MB one-time vs a 261MB download.
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (IS_WIN && !fs.existsSync(curl)) return urls[0];
  log("speed-testing " + urls.length + " source(s), 1MB probe each...");
  const speeds = [];
  for (const u of urls) {
    const s = await probeSpeed(u);
    speeds.push(s);
    log("  " + Math.round(s / 1024) + " KB/s  " + u);
  }
  let best = 0;
  for (let i = 1; i < urls.length; i++) {
    if (speeds[i] > speeds[best]) best = i;
  }
  if (speeds[best] <= 0) {
    log("no source answered the probe; trying preferred order anyway.");
    return urls[0];
  }
  log("fastest: " + urls[best]);
  return urls[best];
}

function zipLooksValid(zipPath) {
  // Three-state check: true = complete archive with dlltool inside; "partial" =
  // looks like an unfinished download, keep it for resume; false = corrupt content,
  // delete. Structural listing via the system tar (Win10 1803+ ships one).
  const st = fs.statSync(zipPath);
  if (st.size < 10485760) return "partial"; // real winlibs zip is far bigger; resumable fragment
  const t = spawnSync("tar", ["-tf", zipPath], { encoding: "utf8" });
  if (t.error && t.error.code === "ENOENT") return true; // no tar available: size-only, do not delete a possibly-good file
  if (t.status !== 0) return false; // big but unreadable: wrong content, not a resume point
  return /bin[\\/]dlltool\.exe/i.test(t.stdout || "");
}

async function installWinlibs() {
  // Portable MinGW-w64 (binutils provides dlltool.exe). Project-local: .tools/mingw64.
  // No admin, no system PATH change; delete the directory to remove.
  // Source: our GitHub Release mirror of the unmodified upstream zip (GPL: plain
  // redistribution with attribution is permitted), falling back to the upstream URL.
  fs.mkdirSync(TOOLS, { recursive: true });
  const zip = path.join(TOOLS, "winlibs.zip");
  if (fs.existsSync(zip)) {
    const v = zipLooksValid(zip);
    if (v === true) {
      log("using existing " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB)");
    } else if (v === "partial") {
      log("found partial " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB) - will resume.");
    } else {
      log("existing " + zip + " is corrupt (wrong content) - deleting and re-downloading.");
      try { fs.rmSync(zip, { force: true }); } catch (e) { /* re-attempt below anyway */ }
    }
  }
  if (!fs.existsSync(zip)) {
    const rel = REL_BASE.replace(/\/+$/, ""); // tolerate trailing slash in REL_BASE / mirror vars
    const candidates = [rel + "/winlibs.zip"];
    const mirror = process.env.SOLOMNI_GH_MIRROR || "";
    if (mirror) candidates.push(mirror.replace(/\/+$/, "") + "/brechtsanders/winlibs_mingw/releases/latest/download/winlibs.zip");
    const upstream = await upstreamWinlibsUrl();
    if (upstream) candidates.push(upstream);
    const url = await pickFastest(candidates);
    log("downloading " + url);
    log("(curl with progress; ~200 MB. Slow? set SOLOMNI_GH_MIRROR, or download the zip");
    log(" manually in a browser and save it as .tools/winlibs.zip - the launcher will use it)");
    const ok = download(url, zip) ? zipLooksValid(zip) : false;
    if (ok === false) {
      log("download still incomplete or corrupt - re-run to resume, or download manually:");
      die("  get a winlibs x86_64 seh zip from https://github.com/brechtsanders/winlibs_mingw/releases");
    }
  }
  log("extracting (takes a minute)...");
  const tar = spawnSync("tar", ["-xf", zip, "-C", TOOLS], { stdio: "ignore" });
  if (tar.status !== 0) runPS("Expand-Archive -Force '" + zip + "' -DestinationPath '" + TOOLS + "'");
  fs.rmSync(zip, { force: true });
  if (!fs.existsSync(path.join(WINLIBS_BIN, "dlltool.exe"))) die("extracted, but dlltool.exe is not in .tools/mingw64/bin - inspect the .tools directory.");
  log("portable MinGW ready: " + WINLIBS_BIN);
}

async function installRust() {
  log("Rust toolchain not found (required to build Solomni).");
  const ans = await ask("Download and install Rust now via rustup? [y/N] ");
  if (ans.trim().toLowerCase() !== "y") {
    die("aborted. Install Rust manually: https://rustup.rs then re-run.");
  }
  log("installing Rust (project-local, a few hundred MB, once; nothing written outside the project)...");
  if (IS_WIN) {
    // GNU toolchain: self-contained linker, no Visual Studio Build Tools needed.
    // Project-local: RUSTUP_HOME/CARGO_HOME under platform/windows, --no-modify-path
    // so the user's system PATH stays untouched.
    fs.mkdirSync(path.join(ROOT, "platform", "windows"), { recursive: true });
    const initExe = path.join(TOOLS, "rustup-init.exe");
    if (!download("https://win.rustup.rs/x86_64", initExe)) {
      die("rustup-init download failed. Check network / proxy.");
    }
    const init = spawnSync(initExe,
      ["-y", "--default-toolchain", "stable-x86_64-pc-windows-gnu", "--no-modify-path"],
      { stdio: "inherit", env: Object.assign({}, process.env, { RUSTUP_HOME: P_RUSTUP, CARGO_HOME: P_CARGO }) });
    if (init.status !== 0) die("rustup install failed.");
  } else {
    const r = spawnSync("sh", ["-c", "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"],
      { stdio: "inherit" });
    if (r.status !== 0) die("rustup install failed. Check network / curl availability.");
  }
  const cargo = findCargo();
  if (!cargo) die("installed, but cargo still not visible. Reopen the terminal and re-run.");
  log("Rust installed: " + cargo);
  return cargo;
}

function run(cargo) {
  const pass = process.argv.slice(2).filter((a) => a !== "--release");
  const argv = [BIN, "."].concat(pass);
  log(process.argv.includes("-webUI")
    ? "starting Web UI (default 127.0.0.1:3081, open http://127.0.0.1:3081)"
    : "starting CLI (use -webUI for the Web UI)");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: ROOT, stdio: "inherit", env: cargoEnv(cargo) });
  if (r.error) die("run failed: " + r.error.message);
  process.exitCode = r.status || 0;
}

(async () => {
  let cargo = findCargo();
  if (!cargo) cargo = await installRust();
  log("platform " + process.platform + " " + process.arch);

  if (IS_WIN) {
    // Preflight BEFORE building: windows-sys fails at compile time without a WORKING
    // dlltool (it must be able to spawn "as.exe"; the toolchain's self-contained copy
    // often lacks one - CreateProcess failure at import-lib generation, see #140704).
    const dd = locateDlltool(cargo);
    if (dd && dlltoolWorks(dd)) {
      EXTRA_PATH.push(dd);
      log("dlltool working: " + dd);
    } else {
      if (dd) log("dlltool found but not usable (cannot run its assembler); falling back to portable MinGW.");
      else {
        console.error("[start] dlltool.exe NOT found at any fixed location (rust-lang/rust#140704:");
        console.error("[start] windows-sys raw-dylib needs dlltool; rust-mingw on this toolchain lacks it).");
      }
      const ans = await ask("Install portable MinGW now? winlibs ~200MB into project .tools, no admin, no system changes [y/N] ");
      if (ans.trim().toLowerCase() === "y") {
        await installWinlibs();
        EXTRA_PATH.push(WINLIBS_BIN);
      } else {
        console.error("[start] manual alternatives:");
        console.error("  1) MSYS2: winget install MSYS2.MSYS2 ; pacman -S mingw-w64-x86_64-binutils ; add C:/msys64/mingw64/bin to PATH");
        console.error("  2) MSVC: install VS 2022 Build Tools (VC workload) ; rustup default stable-x86_64-pc-windows-msvc");
        console.error("  3) Browser: download winlibs zip manually, save as .tools/winlibs.zip, re-run");
        console.error("     (set SOLOMNI_GH_MIRROR to a GitHub mirror prefix to speed up auto-download)");
      }
    }
  }

  const v = spawnSync(cargo, ["--version"], { cwd: ROOT, env: cargoEnv(cargo), encoding: "utf8" });
  if (v.error || v.status !== 0) die("cargo not runnable: " + (v.error && v.error.message));
  log(v.stdout.trim());
  if (!fs.existsSync(BIN)) {
    log("binary not found, building (first run is slow)...");
    const args = RELEASE ? ["build", "--release"] : ["build"];
    const b = spawnSync(cargo, args, { cwd: ROOT, env: cargoEnv(cargo), stdio: "inherit" });
    if (b.status !== 0) die("build failed");
  } else {
    log("using binary: " + path.relative(ROOT, BIN));
  }
  run(cargo);
})();

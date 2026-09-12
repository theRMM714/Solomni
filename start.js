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
const { spawnSync } = require("child_process");
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
  // bundled toolchain needs explicit HOME; system cargo used as-is.
  const env = Object.assign({}, process.env);
  if (path.resolve(cargo) === path.resolve(BUNDLED_CARGO)) {
    env.RUSTUP_HOME = path.join(ROOT, "platform", "linux", "rustup");
    env.CARGO_HOME = path.join(ROOT, "platform", "linux", "cargo");
  }
  env.PATH = [path.dirname(cargo)].concat(EXTRA_PATH, env.PATH || "").filter(Boolean).join(path.delimiter);
  return env;
}

function findCargo() {
  // bundled toolchain first, then PATH; also probe ~/.cargo/bin (PATH not refreshed).
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
  // renders a progress bar. PowerShell fallback for ancient systems.
  const curl = path.join(process.env.SystemRoot || "C:/Windows", "System32", "curl.exe");
  if (fs.existsSync(curl)) {
    const r = spawnSync(curl, ["-L", "--fail", "--retry", "3", "--connect-timeout", "30", "-o", dest, url],
      { stdio: "inherit" });
    if (r.status === 0 && fs.existsSync(dest) && fs.statSync(dest).size > 1048576) return true;
    try { fs.rmSync(dest, { force: true }); } catch (e) { /* drop partial file */ }
    log("curl download failed (exit " + r.status + "); trying PowerShell fallback...");
  }
  runPS("Invoke-WebRequest -UseBasicParsing '" + url + "' -OutFile '" + dest + "'");
  return fs.existsSync(dest) && fs.statSync(dest).size > 1048576;
}

async function installWinlibs() {
  // Portable MinGW-w64 (binutils provides dlltool.exe). Project-local: .tools/mingw64.
  // No admin, no system PATH change; delete the directory to remove.
  log("resolving latest winlibs release (github api)...");
  const q = [
    "$ErrorActionPreference = 'Stop'",
    "$r = Invoke-RestMethod 'https://api.github.com/repos/brechtsanders/winlibs_mingw/releases/latest'",
    "$a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match 'seh' -and $_.name -match '[.]zip$' -and $_.name -notmatch 'llvm' } | Select-Object -First 1",
    "if (-not $a) { $a = $r.assets | Where-Object { $_.name -match 'x86_64' -and $_.name -match '[.]zip$' } | Select-Object -First 1 }",
    "Write-Output $a.browser_download_url"
  ].join("; ");
  const got = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", q], { encoding: "utf8" });
  let url = (got.stdout || "").trim();
  if (!/^https:/.test(url)) die("cannot resolve winlibs download url (network?). Use the manual options below.");
  fs.mkdirSync(TOOLS, { recursive: true });
  const zip = path.join(TOOLS, "winlibs.zip");
  if (fs.existsSync(zip) && fs.statSync(zip).size > 10485760) {
    log("using existing " + zip + " (" + Math.round(fs.statSync(zip).size / 1048576) + " MB)");
  } else {
    const mirror = process.env.SOLOMNI_GH_MIRROR || "";
    if (mirror) { log("using GitHub mirror prefix: " + mirror); url = mirror + url; }
    log("downloading " + url);
    log("(curl with progress; ~200 MB. Slow? set SOLOMNI_GH_MIRROR, or download the zip");
    log(" manually in a browser and save it as .tools/winlibs.zip - the launcher will use it)");
    if (!download(url, zip)) {
      die("download failed. Manual: download the file above in a browser, save as .tools/winlibs.zip, re-run.");
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
  log("installing rustup (downloads a few hundred MB, once)...");
  if (IS_WIN) {
    // GNU toolchain: self-contained linker, no Visual Studio Build Tools needed.
    const dl = spawnSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
      "Invoke-WebRequest -UseBasicParsing https://win.rustup.rs/x86_64 -OutFile $env:TEMP\\rustup-init.exe"],
      { stdio: "inherit" });
    if (dl.status !== 0) die("rustup download failed. Check network / proxy.");
    const init = spawnSync(path.join(process.env.TEMP || "", "rustup-init.exe"),
      ["-y", "--default-toolchain", "stable-x86_64-pc-windows-gnu"],
      { stdio: "inherit" });
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
      console.error("[start] dlltool.exe NOT found at any fixed location (rust-lang/rust#140704:");
      console.error("[start] windows-sys raw-dylib needs dlltool; rust-mingw on this toolchain lacks it).");
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

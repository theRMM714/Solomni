@echo off
rem ui_flutter dev launcher (Windows). Spawned by the product assembler.
rem Module-owned responsibilities (product knows none of these):
rem   1) dependency self-heal: .dart_tool SDK deps are absolute paths written
rem      by the toolchain (platform-specific); re-resolve when foreign/stale
rem   2) device is windows (this script only runs on Windows)
rem   3) start daemon entry bin/coordinated.dart; product injects --port=N
setlocal
set "SELF_DIR=%~dp0"
set "ROOT=%SELF_DIR%..\..\..\..\"
set "FLUTTER=%ROOT%bin\flutter.bat"

rem self-heal: foreign = linux-style absolute paths (file:///mnt/ ...)
set "NEED_GET=0"
if not exist "%SELF_DIR%.dart_tool\package_config.json" set "NEED_GET=1"
if "%NEED_GET%"=="0" findstr /C:"file:///mnt/" /C:"file:///home/" "%SELF_DIR%.dart_tool\package_config.json" >nul 2>&1 && set "NEED_GET=1"
if "%NEED_GET%"=="1" (
  echo [ui_flutter] dependency config foreign or missing, re-resolving...
  call "%FLUTTER%" pub get
  if errorlevel 1 exit /b 1
)

call "%FLUTTER%" run -t bin\coordinated.dart -d windows -- %*
exit /b %ERRORLEVEL%

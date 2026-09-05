@echo off
rem ui_flutter dev launcher (Windows). Spawned by the host.
rem Module-owned responsibilities (product knows none of these):
rem   1) device is windows (this script only runs on Windows); dependency
rem      self-heal is centralized in bin\flutter.bat (platform config swap)
rem   2) start daemon entry bin\coordinated.dart; product injects --port=N
setlocal
set "SELF_DIR=%~dp0"
set "ROOT=%SELF_DIR%..\..\..\..\"
set "FLUTTER=%ROOT%bin\flutter.bat"

rem arg forwarding: this SDK version of flutter run DROPS args after "--";
rem each incoming arg must become --dart-entrypoint-args=<arg> to reach
rem main(List<String> args)
set "ENTRY_ARGS="
:argloop
if "%~1"=="" goto :runargs
set ENTRY_ARGS=%ENTRY_ARGS% "--dart-entrypoint-args=%~1"
shift
goto :argloop
:runargs
call "%FLUTTER%" run -t bin\coordinated.dart -d windows %ENTRY_ARGS%
exit /b %ERRORLEVEL%

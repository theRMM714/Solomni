@echo off
rem ui_cli surface launcher (Windows). Spawned by the host.
rem Terminal surface, no special deps: run the Dart daemon entry in REPL mode
rem and forward the injected --port=N untouched.
setlocal
set "SELF_DIR=%~dp0"
set "ROOT=%SELF_DIR%..\..\..\..\"
set "DART=%ROOT%bin\dart.bat"

call "%DART%" bin\coordinated.dart --repl %*
exit /b %ERRORLEVEL%

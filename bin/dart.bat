@echo off
rem bin\dart.bat -- Windows wrapper: set repo cache then call local Dart SDK
setlocal
set "ROOT=%~dp0.."
set "PUB_CACHE=%ROOT%\.pub-cache"
set "TMP=%ROOT%\.tmp"
set "TEMP=%TMP%"
if exist "%ROOT%\dart-sdk\dart-sdk\bin\dart.exe" (
  "%ROOT%\dart-sdk\dart-sdk\bin\dart.exe" %*
  exit /b %ERRORLEVEL%
)
echo Dart SDK not installed. Run: node bin\setup.mjs 1>&2
exit /b 1

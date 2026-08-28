@echo off
rem bin\dart.bat -- Windows wrapper: platform-suffixed SDK dirs + repo cache
setlocal
set "ROOT=%~dp0.."
set "PUB_CACHE=%ROOT%\.pub-cache-windows"
set "TMP=%ROOT%\.tmp-windows"
set "TEMP=%TMP%"
if exist "%ROOT%\dart-sdk-windows\dart-sdk\bin\dart.exe" (
  "%ROOT%\dart-sdk-windows\dart-sdk\bin\dart.exe" %*
  exit /b %ERRORLEVEL%
)
echo Dart SDK not installed. Run: node bin\setup.mjs 1>&2
exit /b 1

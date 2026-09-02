@echo off
rem bin\flutter.bat -- Windows wrapper: platform-suffixed SDK dirs + repo cache
setlocal
set "ROOT=%~dp0.."
set "PUB_CACHE=%ROOT%\.pub-cache-windows"
set "TMP=%ROOT%\.tmp-windows"
set "TEMP=%TMP%"
if exist "%ROOT%\flutter-windows\flutter\bin\flutter.bat" (
  call "%ROOT%\flutter-windows\flutter\bin\flutter.bat" %*
  exit /b %ERRORLEVEL%
)
echo Flutter SDK not installed. Run: node bin\setup.mjs --flutter 1>&2
exit /b 1

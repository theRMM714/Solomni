@echo off
rem bin\flutter.bat -- Windows wrapper: set repo cache then call local Flutter SDK
setlocal
set "ROOT=%~dp0.."
set "PUB_CACHE=%ROOT%\.pub-cache"
set "TMP=%ROOT%\.tmp"
set "TEMP=%TMP%"
if exist "%ROOT%\flutter\flutter\bin\flutter.bat" (
  "%ROOT%\flutter\flutter\bin\flutter.bat" %*
  exit /b %ERRORLEVEL%
)
echo Flutter SDK not installed. Run: node bin\setup.mjs --flutter 1>&2
exit /b 1

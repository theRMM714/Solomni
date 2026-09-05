@echo off
rem bin\dart.bat -- Windows wrapper: platform dir + repo cache
setlocal
set "ROOT=%~dp0.."
set "PUB_CACHE=%ROOT%\platform\windows\pub-cache"
set "TMP=%ROOT%\platform\windows\tmp"
set "TEMP=%TMP%"
set "DART=%ROOT%\platform\windows\dart-sdk\bin\dart.exe"
if exist "%DART%" (
  "%DART%" %*
  exit /b %ERRORLEVEL%
)
echo Dart SDK not installed. Run: node bin\setup.mjs 1>&2
exit /b 1

@echo off
rem bin\solomni.bat -- product launcher thin shell (Windows); logic in solomni.mjs
node "%~dp0solomni.mjs" %*
exit /b %ERRORLEVEL%

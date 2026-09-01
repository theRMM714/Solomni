@echo off
rem bin\solomni.bat -- 产品启动器薄壳（Windows）；实现在 solomni.mjs
node "%~dp0solomni.mjs" %*
exit /b %ERRORLEVEL%

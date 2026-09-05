@echo off
rem solomni.bat -- root portal: forward to bin\solomni.bat (product launcher)
rem Default (no args) shows the surface menu; nothing is auto-launched.
call "%~dp0bin\solomni.bat" %*
exit /b %ERRORLEVEL%

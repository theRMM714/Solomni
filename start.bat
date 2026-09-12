@echo off
rem Solomni launcher (Windows). Default CLI; add -webUI for the Web UI.
where node >nul 2>nul
if errorlevel 1 (
  echo [start] Node.js not found. Install from https://nodejs.org
  pause
  exit /b 1
)
node "%~dp0start.js" %*
pause

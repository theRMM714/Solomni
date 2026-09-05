@echo off
rem bin\flutter.bat -- Windows wrapper: platform dir + repo cache + config heal
setlocal
set "ROOT=%~dp0.."
set "PUB_CACHE=%ROOT%\platform\windows\pub-cache"
set "TMP=%ROOT%\platform\windows\tmp"
set "TEMP=%TMP%"
set "FLUTTER=%ROOT%\platform\windows\flutter\bin\flutter.bat"

rem ---- platform config heal (flutter packages only; pure Dart never triggers) ----
rem The toolchain only reads <pkg>\.dart_tool\package_config.json at this fixed
rem location; the launcher owns the content: swap in this platform's snapshot
rem when fresh (pubspec.lock unchanged), otherwise really resolve and snapshot.
if not exist "%CD%\pubspec.yaml" goto :run
set "CFG=%CD%\.dart_tool\package_config.json"
for %%I in ("%CD%") do set "PKG=%%~nxI"
set "SNAPDIR=%ROOT%\platform\windows\dart_tool\%PKG%"
set "SNAP=%SNAPDIR%\package_config.json"
set "STAMP=%SNAPDIR%\pubspec.lock"
set "FOREIGN=0"
if not exist "%CFG%" set "FOREIGN=1"
if %FOREIGN%==0 findstr /C:"file:///mnt/" /C:"file:///home/" "%CFG%" >nul 2>&1 && set "FOREIGN=1"
if %FOREIGN%==0 goto :run
if exist "%SNAP%" (
  fc /b "%CD%\pubspec.lock" "%STAMP%" >nul 2>&1
  if not errorlevel 1 (
    if not exist "%CD%\.dart_tool" mkdir "%CD%\.dart_tool"
    copy /Y "%SNAP%" "%CFG%" >nul
    goto :run
  )
)
call "%FLUTTER%" pub get
if errorlevel 1 exit /b 1
if not exist "%SNAPDIR%" mkdir "%SNAPDIR%"
copy /Y "%CFG%" "%SNAP%" >nul
copy /Y "%CD%\pubspec.lock" "%STAMP%" >nul

:run
if exist "%FLUTTER%" (
  call "%FLUTTER%" %*
  exit /b %ERRORLEVEL%
)
echo Flutter SDK not installed. Run: node bin\setup.mjs --flutter 1>&2
exit /b 1

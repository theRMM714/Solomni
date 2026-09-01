@echo off
rem ui_flutter 的开发态启动脚本（Windows 侧）。产品装配器经它拉起本模块。
rem 职责（模块自决，产品不参与）：
rem   1) 依赖自愈：.dart_tool 里 SDK 依赖是工具链写死的绝对路径（平台相关），
rem      检测异侧/缺失配置则先 pub get（纯 path 依赖的包两侧通用，不会命中）
rem   2) 设备固定 windows（本脚本只在 Windows 侧被调用）
rem   3) 起守护入口 bin/coordinated.dart，端口由产品注入（--port=N 透传）
setlocal
set "SELF_DIR=%~dp0"
set "ROOT=%SELF_DIR%..\..\..\..\"
set "FLUTTER=%ROOT%bin\flutter.bat"

rem 依赖自愈：异侧 = 出现 Linux 风格绝对路径（file:///mnt/… 等）；缺失同理
set "NEED_GET=0"
if not exist "%SELF_DIR%.dart_tool\package_config.json" set "NEED_GET=1"
if "%NEED_GET%"=="0" findstr /C:"file:///mnt/" /C:"file:///home/" "%SELF_DIR%.dart_tool\package_config.json" >nul 2>&1 && set "NEED_GET=1"
if "%NEED_GET%"=="1" (
  echo [ui_flutter] 依赖配置异侧/缺失，重新解析…
  call "%FLUTTER%" pub get
  if errorlevel 1 exit /b 1
)

call "%FLUTTER%" run -t bin\coordinated.dart -d windows -- %*
exit /b %ERRORLEVEL%

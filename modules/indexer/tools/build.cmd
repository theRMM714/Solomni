@echo off
rem 目的：把 indexer 编出来（Windows）。产品用 `solomni --build=indexer` 调它（cwd = 模块根）。
rem 与无扩展名的 tools/build 并排：cmd 按 PATHEXT 找到本文件（同 npm 的 shim 命名）。
rem 约束：g++ 要在宿主 PATH 上（模块声明的构建期能力 `gcc` 也按 PATH 判）。
if not exist build mkdir build
g++ -O2 -std=c++17 -Wall -Wextra -static -static-libgcc -static-libstdc++ tools/src/indexer.cpp -o build/indexer.exe
if errorlevel 1 exit /b 1

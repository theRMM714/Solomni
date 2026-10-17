@echo off
rem Build indexer on Windows (run by `solomni --build=indexer`; cwd = module root).
rem Sits next to the extension-less tools/build: cmd finds this file through PATHEXT.
rem ASCII only: cmd parses batch bytes with the console code page, so a non-ASCII comment
rem can swallow the next line (see MODULE_SPEC, the cross-platform note under build).
rem g++ must be on the host PATH (the module declares gcc as its build-time capability).
if not exist build mkdir build
g++ -O2 -std=c++17 -Wall -Wextra -static -static-libgcc -static-libstdc++ tools/src/indexer.cpp -o build/indexer.exe
if errorlevel 1 exit /b 1

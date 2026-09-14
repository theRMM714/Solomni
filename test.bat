@echo off
rem 薄包装：跑全部测试（见 TESTING.md）。等价于 node start.js -test；受限环境请直接 node run-tests.js。
node "%~dp0start.js" -test

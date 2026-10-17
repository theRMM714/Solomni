@echo off
rem Thin wrapper: run the whole test battery (see TESTING.md).
rem Same as `node start.js -test`; restricted shells should run `node run-tests.js` directly.
rem ASCII only: cmd parses batch bytes with the console code page, so a non-ASCII comment
rem can swallow the next line.
node "%~dp0start.js" -test

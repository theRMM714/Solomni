#!/bin/sh
# Unix 一键启动：默认 CLI；带 -webUI 进 Web 转录中心。
exec node "$(dirname "$0")/start.js" "$@"

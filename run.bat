@echo off
chcp 65001 >nul
setlocal

set WIN_DIR=/mnt/e/TUAN2/LANGUAGE/lume
set WSL_DIR=~/projects/lume
set DISTRO=Ubuntu

REM 测试由 scripts/run_tests.sh 驱动（WSL 里跑）：
REM   - 正向：main.lume + tests_*.lume，输出逐字节比对 tests/golden/
REM   - 负向：tests/fail/*.lume，必须按预期报错
REM 输出有意变了（比如改了期望值）之后，用 UPDATE=1 重写基线：
REM   wsl -d Ubuntu bash -lc "cd ~/projects/lume && UPDATE=1 bash scripts/run_tests.sh"

echo [1/3] Syncing %WIN_DIR% to WSL:%WSL_DIR%

REM .workbuddy 是本地工作区（memory + scratch 临时文件），不同步
wsl -d %DISTRO% bash -lc "mkdir -p %WSL_DIR% && rsync -a --delete --exclude target --exclude .git --exclude .lume_cache --exclude .workbuddy %WIN_DIR%/ %WSL_DIR%/"

if errorlevel 1 (
    echo [ERROR] Sync failed
    exit /b 1
)

echo [2/3] Running tests
wsl -d %DISTRO% bash -lc "cd %WSL_DIR% && bash scripts/run_tests.sh"
if errorlevel 1 (
    echo [ERROR] tests failed
    exit /b 1
)

echo [3/3] Done
endlocal

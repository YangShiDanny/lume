@echo off
chcp 65001 >nul
setlocal

set WIN_DIR=/mnt/e/TUAN2/LANGUAGE/lume
set WSL_DIR=~/projects/lume
set DISTRO=Ubuntu

REM 要跑的测试。每个都会 lume run <file>，一路串行跑完。
REM 想临时只测一个，把 TESTS 改成只留那一项。
set TESTS=main.lume tests_optional.lume

REM ==================

echo [1/3] Syncing %WIN_DIR% to WSL:%WSL_DIR%

wsl -d %DISTRO% bash -lc "mkdir -p %WSL_DIR% && rsync -a --delete --exclude target --exclude .git --exclude .lume_cache %WIN_DIR%/ %WSL_DIR%/"

if errorlevel 1 (
    echo [ERROR] Sync failed
    exit /b 1
)

echo [2/3] Running tests
for %%F in (%TESTS%) do (
    echo.
    echo ---------------- %%F ----------------
    wsl -d %DISTRO% bash -lc "cd %WSL_DIR% && cargo run --quiet -- run %%F"
    if errorlevel 1 (
        echo [ERROR] %%F failed
        exit /b 1
    )
)

echo [3/3] Done
endlocal
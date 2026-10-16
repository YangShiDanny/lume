@echo off
chcp 65001 >nul
setlocal

set WIN_DIR=/mnt/e/TUAN2/LANGUAGE/lume
set WSL_DIR=~/projects/lume
set DISTRO=Ubuntu
set CMD=cargo run -- run main.lume
REM ==================

echo [1/3] Syncing %WIN_DIR% to WSL:%WSL_DIR%

wsl -d %DISTRO% bash -lc "mkdir -p %WSL_DIR% && rsync -a --delete --exclude target --exclude .git --exclude .lume_cache %WIN_DIR%/ %WSL_DIR%/"

if errorlevel 1 (
    echo [ERROR] Sync failed
    exit /b 1
)

echo [2/3] Running: %CMD%
wsl -d %DISTRO% bash -lc "cd %WSL_DIR% && %CMD%"

if errorlevel 1 (
    echo [ERROR] Command failed
    exit /b 1
)

echo [3/3] Done
endlocal
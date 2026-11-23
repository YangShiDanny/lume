@echo off
setlocal
rem Lume CLI 入口（Windows CMD）。逻辑统一在 lume_launcher.py。
set "ROOT=%~dp0"

set "PYTHON="
if defined LUME_LAUNCHER_PYTHON set "PYTHON=%LUME_LAUNCHER_PYTHON%"
if not defined PYTHON (
  where python3 >nul 2>&1
  if not errorlevel 1 (
    for /f "delims=" %%P in ('where python3') do set "PYTHON=%%P" & goto :found_python
  )
  where python >nul 2>&1
  if not errorlevel 1 (
    for /f "delims=" %%P in ('where python') do set "PYTHON=%%P" & goto :found_python
  )
)
:found_python
if not defined PYTHON (
  echo [lume] Python 3 not found; install Python 3.10+ or set LUME_LAUNCHER_PYTHON. 1>&2
  exit /b 127
)

"%PYTHON%" "%ROOT%lume_launcher.py" %*
exit /b %ERRORLEVEL%

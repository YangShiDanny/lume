#!/usr/bin/env pwsh
# Lume CLI 入口（Windows PowerShell / PowerShell Core）。逻辑统一在 lume_launcher.py。
$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Python = $env:LUME_LAUNCHER_PYTHON

if (-not $Python) {
  foreach ($name in @('python3', 'python')) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd) {
      $Python = $cmd.Source
      break
    }
  }
}

if (-not $Python) {
  Write-Error '[lume] Python 3 not found; install Python 3.10+ or set LUME_LAUNCHER_PYTHON.'
  exit 127
}

& $Python (Join-Path $Root 'lume_launcher.py') @args
exit $LASTEXITCODE

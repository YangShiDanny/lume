# install.ps1
# Add lume.exe's directory to the user PATH

$ErrorActionPreference = "Stop"

$exeDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$exePath = Join-Path $exeDir "lume.exe"

if (-not (Test-Path $exePath)) {
    Write-Host "Error: lume.exe not found in $exeDir" -ForegroundColor Red
    exit 1
}

$userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($null -eq $userPath) { $userPath = "" }

if ($userPath -split ';' -contains $exeDir) {
    Write-Host "Already installed: $exeDir is in PATH" -ForegroundColor Green
} else {
    $newPath = if ($userPath) { "$userPath;$exeDir" } else { $exeDir }
    [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
    Write-Host "Added $exeDir to user PATH" -ForegroundColor Green
}

$clang = Get-Command clang -ErrorAction SilentlyContinue
if (-not $clang) {
    Write-Host ""
    Write-Host "Warning: clang not found. lume needs it to compile C." -ForegroundColor Yellow
    Write-Host "Install: winget install LLVM.LLVM" -ForegroundColor Yellow
} else {
    Write-Host "clang found: $($clang.Source)" -ForegroundColor Green
}

Write-Host ""
Write-Host "Done. Close all terminals, reopen, then run:" -ForegroundColor Cyan
Write-Host "  lume run hello.lume" -ForegroundColor Cyan
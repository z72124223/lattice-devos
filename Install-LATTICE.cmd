@echo off
setlocal
set "ROOT=%~dp0"
where py >nul 2>nul
if errorlevel 1 (
  echo LATTICE needs Python 3.12 or newer for the bootstrap step.
  echo Install Python from https://www.python.org/downloads/windows/ and run this file again.
  exit /b 2
)
set "SOURCE=%~1"
if "%SOURCE%"=="" set "SOURCE=%CD%"
set "WSL=%~2"
if "%WSL%"=="" set "WSL=%SystemRoot%\System32\wsl.exe"
set "MODE=%~3"
py -3 "%ROOT%scripts\lattice-one-click-install.py" --graph-source "%SOURCE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
if errorlevel 2 exit /b %errorlevel%
py -3 "%ROOT%scripts\lattice-codex-profile.py" --output "%LOCALAPPDATA%\LATTICE\codex-portable-profile.json"
exit /b %errorlevel%

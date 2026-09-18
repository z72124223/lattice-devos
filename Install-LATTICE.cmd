@echo off
setlocal
set "ROOT=%~dp0"
where py >nul 2>nul
if errorlevel 1 (
  echo LATTICE needs Python 3.12 or newer for the bootstrap step.
  echo Install Python from https://www.python.org/downloads/windows/ and run this file again.
  exit /b 2
)
if "%~2"=="" (
  echo Usage: Install-LATTICE.cmd ^<Graphify-source-folder^> ^<WSL-launcher^> [--install]
  exit /b 2
)
py -3 "%ROOT%scripts\lattice-one-click-install.py" --graph-source "%~1" --wsl "%~2" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" %3
exit /b %errorlevel%

@echo off
setlocal
set "ROOT=%~dp0"
set "PYTHON=%ROOT%bundle\python\python.exe"
if not exist "%PYTHON%" (
  echo The verified LATTICE bundle is incomplete: bundled Python is missing.
  exit /b 2
)
set "SOURCE=%~1"
if "%SOURCE%"=="" set "SOURCE=%CD%"
set "WSL=%~2"
if "%WSL%"=="" set "WSL=%SystemRoot%\System32\wsl.exe"
set "MODE=%~3"
"%PYTHON%" "%ROOT%scripts\lattice-one-click-install.py" --bundle "%ROOT%bundle" --graph-source "%SOURCE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
if errorlevel 2 exit /b %errorlevel%
"%PYTHON%" "%ROOT%scripts\lattice-codex-profile.py" --output "%LOCALAPPDATA%\LATTICE\codex-portable-profile.json"
"%PYTHON%" "%ROOT%scripts\lattice-environment-manifest.py" collect --bundle "%ROOT%bundle" --manifest "%LOCALAPPDATA%\LATTICE\required-environment.json"
exit /b %errorlevel%

@echo off
setlocal
set "ROOT=%~dp0"
set "BUNDLE=%ROOT%bundle"
if not exist "%BUNDLE%" set "BUNDLE=%ROOT%bundle-v5"
set "PYTHON=%BUNDLE%\python\python.exe"
if not exist "%PYTHON%" (
  echo The verified LATTICE bundle is incomplete: bundled Python is missing.
  exit /b 2
)
set "SOURCE=%~1"
if "%SOURCE%"=="" set "SOURCE=%CD%"
set "WSL=%~2"
if "%WSL%"=="" set "WSL=%SystemRoot%\System32\wsl.exe"
set "MODE=%~3"
"%PYTHON%" -B "%ROOT%scripts\lattice-one-click-install.py" --bundle "%BUNDLE%" --graph-source "%SOURCE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
if errorlevel 2 exit /b %errorlevel%
"%PYTHON%" -B "%ROOT%scripts\lattice-codex-profile.py" --output "%LOCALAPPDATA%\LATTICE\codex-portable-profile.json"
"%PYTHON%" -B "%ROOT%scripts\lattice-environment-manifest.py" collect --bundle "%BUNDLE%" --manifest "%LOCALAPPDATA%\LATTICE\required-environment.json"
exit /b %errorlevel%

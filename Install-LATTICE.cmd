@echo off
setlocal
chcp 65001 >nul
set "ROOT=%~dp0"
set "BUNDLE=%ROOT%bundle"
if not exist "%BUNDLE%" set "BUNDLE=%ROOT%bundle-v5"
if not exist "%BUNDLE%" set "BUNDLE=%ROOT%rebuild-bundle-v6"
set "PYTHON=%BUNDLE%\python\python.exe"
if not exist "%PYTHON%" (
  echo The verified LATTICE bundle is incomplete: bundled Python is missing.
  exit /b 2
)
set "SOURCE=%~1"
set "WSL=%~2"
if "%WSL%"=="" set "WSL=%SystemRoot%\System32\wsl.exe"
set "MODE=%~3"
set "INTERACTIVE=--interactive"
if defined LATTICE_INSTALL_UNATTENDED set "INTERACTIVE="
echo Installing LATTICE and checking all three cores. Please keep this window open.
if "%SOURCE%"=="" (
  "%PYTHON%" -I -B -S "%ROOT%scripts\lattice-one-click-install.py" --install --install-global-hook %INTERACTIVE% --bundle "%BUNDLE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
) else (
  "%PYTHON%" -I -B -S "%ROOT%scripts\lattice-one-click-install.py" --install --install-global-hook %INTERACTIVE% --bundle "%BUNDLE%" --graph-source "%SOURCE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
)
if errorlevel 1 goto failed
if exist "%ROOT%codex-preferences.json" (
  "%PYTHON%" -I -B -S "%ROOT%scripts\lattice-codex-profile.py" setup --profile "%ROOT%codex-preferences.json" %INTERACTIVE% --output "%LOCALAPPDATA%\LATTICE\preferences-report.json"
  if errorlevel 1 goto failed
)
"%PYTHON%" -I -B -S "%ROOT%scripts\lattice-codex-profile.py" --output "%LOCALAPPDATA%\LATTICE\codex-portable-profile.json"
if errorlevel 1 goto failed
"%PYTHON%" -I -B -S "%ROOT%scripts\lattice-environment-manifest.py" collect --bundle "%BUNDLE%" --manifest "%LOCALAPPDATA%\LATTICE\required-environment.json"
if errorlevel 1 goto failed
echo LATTICE installation and checks passed. Restart Codex to load the connection.
if not defined LATTICE_INSTALL_UNATTENDED pause
exit /b 0

:failed
set "INSTALL_EXIT=%errorlevel%"
echo LATTICE is not fully installed. Existing files have been preserved.
if "%INSTALL_EXIT%"=="3" echo Save your work, restart Windows, then double-click this installer again.
echo Open %%LOCALAPPDATA%%\LATTICE\one-click-report.json for the failed check.
if not defined LATTICE_INSTALL_UNATTENDED pause
exit /b %INSTALL_EXIT%

@echo off
setlocal
set "LATTICE_DASHBOARD_REPO=%~dp0."
set "LATTICE_DASHBOARD_OPEN=--open"
set "LATTICE_DASHBOARD_NETWORK="
if /I "%LATTICE_DASHBOARD_NO_OPEN%"=="1" set "LATTICE_DASHBOARD_OPEN="
if /I "%LATTICE_DASHBOARD_OFFLINE%"=="1" set "LATTICE_DASHBOARD_NETWORK=--offline"

node "%LATTICE_DASHBOARD_REPO%\scripts\export-lattice-engineering-status.mjs" --repository "%LATTICE_DASHBOARD_REPO%" %LATTICE_DASHBOARD_NETWORK% %LATTICE_DASHBOARD_OPEN%
if errorlevel 1 (
  echo.
  echo LATTICE dashboard refresh failed. The previous snapshot was not treated as new evidence.
  echo Ask Codex to inspect the failure.
  pause
  exit /b 1
)

endlocal

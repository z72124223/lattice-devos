@echo off
setlocal
chcp 65001 >nul
set "ROOT=%~dp0"
set "BUNDLE=%ROOT%bundle"
if not exist "%BUNDLE%" set "BUNDLE=%ROOT%bundle-v5"
if not exist "%BUNDLE%" set "BUNDLE=%ROOT%rebuild-bundle-v6"
set "PYTHON=%BUNDLE%\python\python.exe"
if not exist "%PYTHON%" (
  echo 安裝包不完整，缺少內附 Python。請重新下載完整安裝包。
  exit /b 2
)
set "SOURCE=%~1"
set "WSL=%~2"
if "%WSL%"=="" set "WSL=%SystemRoot%\System32\wsl.exe"
set "MODE=%~3"
set "INTERACTIVE=--interactive"
if defined LATTICE_INSTALL_UNATTENDED set "INTERACTIVE="
set "FAILED_STEP=三核心安裝與驗證"
set "FAILED_REPORT=%LOCALAPPDATA%\LATTICE\one-click-report.json"
echo 正在安裝 LATTICE 並檢查三核心，請保持此視窗開啟。
if "%SOURCE%"=="" (
  "%PYTHON%" -I -B -S "%ROOT%scripts\lattice-one-click-install.py" --install --install-global-hook %INTERACTIVE% --bundle "%BUNDLE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
) else (
  "%PYTHON%" -I -B -S "%ROOT%scripts\lattice-one-click-install.py" --install --install-global-hook %INTERACTIVE% --bundle "%BUNDLE%" --graph-source "%SOURCE%" --wsl "%WSL%" --report "%LOCALAPPDATA%\LATTICE\one-click-report.json" --overlap-report "%LOCALAPPDATA%\LATTICE\overlap-audit.json" %MODE%
)
if errorlevel 1 goto failed
if exist "%ROOT%codex-preferences.json" (
  set "FAILED_STEP=Codex 偏好設定"
  set "FAILED_REPORT=%LOCALAPPDATA%\LATTICE\preferences-report.json"
  "%PYTHON%" -I -B -S "%ROOT%scripts\lattice-codex-profile.py" setup --profile "%ROOT%codex-preferences.json" %INTERACTIVE% --output "%LOCALAPPDATA%\LATTICE\preferences-report.json"
  if errorlevel 1 goto failed
)
set "FAILED_STEP=儲存 Codex 可攜偏好"
set "FAILED_REPORT=%LOCALAPPDATA%\LATTICE\codex-portable-profile.json"
"%PYTHON%" -I -B -S "%ROOT%scripts\lattice-codex-profile.py" --output "%LOCALAPPDATA%\LATTICE\codex-portable-profile.json"
if errorlevel 1 goto failed
set "FAILED_STEP=必要環境檢查"
set "FAILED_REPORT=%LOCALAPPDATA%\LATTICE\required-environment.json"
"%PYTHON%" -I -B -S "%ROOT%scripts\lattice-environment-manifest.py" collect --bundle "%BUNDLE%" --manifest "%LOCALAPPDATA%\LATTICE\required-environment.json"
if errorlevel 1 goto failed
echo LATTICE 安裝與檢查已通過。請關閉並重新開啟 Codex，載入新連線。
if not defined LATTICE_INSTALL_UNATTENDED pause
exit /b 0

:failed
set "INSTALL_EXIT=%errorlevel%"
echo 安裝尚未全部完成，原有檔案已保留。
echo 未完成的步驟：%FAILED_STEP%
if "%INSTALL_EXIT%"=="3" echo 請先儲存工作並重新啟動 Windows，之後再雙擊同一個安裝檔。
echo 可將此視窗與報告交給 Codex 協助處理：%FAILED_REPORT%
if not defined LATTICE_INSTALL_UNATTENDED pause
exit /b %INSTALL_EXIT%

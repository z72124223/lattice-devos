# Windows 自解壓安裝包建置

建置入口是 `scripts/build-lattice-windows-installer.py`。朋友收到的 `.exe`
會自行解壓，再執行既有 `Install-LATTICE.cmd`；朋友不需要另裝 7-Zip。
此工具只包裝已完成驗收的候選資料夾，不會把候選版自動判定為正式版。

## 固定來源

使用 [7-Zip 官方 LZMA SDK 26.03](https://www.7-zip.org/sdk.html) 的
`bin/7zSD.sfx`，不使用第三方 SFX 分支。SDK 及模組均為 public domain；
完整授權在 SDK 的 `DOC/lzma-sdk.txt`，修改模組資源的許可見 `DOC/installer.txt`。

- SDK：[官方發行檔](https://github.com/ip7z/7zip/releases/download/26.03/lzma2603.7z)
- SDK SHA-256：`86c213f752520ab5325c310f50bef63ec344b56dd1c80b0246d06dc6cec953b2`
- 原始 `7zSD.sfx` SHA-256：`92194891840ce85cc528f44fd066a4acd708c75723e0721c10cb9850679c9ab9`
- SDK 摘要已與 [官方 GitHub release API](https://api.github.com/repos/ip7z/7zip/releases/tags/26.03) 核對。

原始 SDK 保持不變。建置器只在模組副本加入 Windows `asInvoker` manifest，
讓安裝主流程以目前使用者身分執行。WSL 啟用仍使用既有的明確確認和 UAC 流程，
不變更 Windows 安全設定。

## 建置命令

在 repository 根目錄執行。先將 `$Candidate` 設成已驗收的完整候選資料夾，
將 `$Output` 設成已存在輸出目錄中的新 `.exe` 路徑：

```powershell
$Python = Join-Path $Candidate 'bundle/python/python.exe'
$Sfx = Join-Path $env:LOCALAPPDATA 'LATTICE/installer-sfx-toolchain/lzma2603/official/bin/7zSD.sfx'
& $Python -I -B -S scripts/build-lattice-windows-installer.py `
  --package $Candidate --output $Output `
  --sevenzip 'C:/Program Files/7-Zip/7z.exe' --sfx-module $Sfx
```

建置器拒絕覆寫既有輸出，檢查候選檔案在壓縮前後未改變，並對 `.7z` 和最終
`.exe` 執行 `7z t`。旁邊的 `.build.json` 保存完整摘要、來源及驗證範圍。
產物尚未簽章；`BUILT_ARCHIVE_TESTED` 只表示包裝與壓縮完整性通過。

## 安裝結果與重新開機

官方 SFX 會等待安裝子程序，但**不傳遞子程序退出碼**。因此外層 `.exe`
退出 `0` 不能當作安裝成功。實際 `Install-LATTICE.cmd` 的整體退出碼保存在
`%LOCALAPPDATA%/LATTICE/sfx-last-result.json`；需確認時間屬於本次執行。
這包括三核心、Codex 偏好及環境步驟，不只前半段的 `one-click-report.json`。

若整體退出碼為 `3`，結果會標示 `reboot_requested=true`。使用者須先儲存工作、
自行重新開機，再雙擊同一個 `.exe`。封裝器不會自動重開機。

小型真實驗證已確認自解壓、等待子程序及保留退出碼 `7`；單元測試另涵蓋退出碼
`0`、`3`、`7`。這些驗證不等於完整安裝包已在乾淨電腦完成驗收。

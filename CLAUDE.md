# LATTICE — Claude 專案導讀

本檔是 Claude Code 的專案指引，也是 Claude 網頁讀者可直接閱讀的入口。
用途是理解、評估與維護此倉庫；產品目前的執行入口仍為 Codex App。
新增本檔不代表已完成 Claude 執行環境整合。

## 先讀這六點

以下為 2026-09-22 核對的產品範圍；版本與驗收細節見同一提交的 [README](README.md)。

1. **用途**：在 Codex App 背後保存正式專案／任務、決策與可查證結果，並查詢程式碼關係。
2. **三核心**：LATTICE 控制、PostgreSQL、Graphify；Hermes 已永久退役。
3. **現行實作**：Rust Runtime 與 PostgreSQL 儲存層仍在使用。舊全鏈路文件僅供追溯。
4. **模型**：全產品並非固定 `gpt-5.6-terra`；原生工作與歷史受管流程的限制不同。
   模型名稱描述 LATTICE 的執行路徑；閱讀此倉庫不需要切換 Claude 本身的模型。
5. **交付**：已有 Windows／WSL 完整安裝候選包，一般使用者不需自行編譯。
   Windows Server 2025 已有安裝證據；Windows 10／11 首次 WSL、UAC／重開機及 Codex 登入仍待真機驗收。
6. **授權與聯絡**：尚未選定 LICENSE；作者 [z72124223](https://github.com/z72124223)，
   信箱 [z72124223@gmail.com](mailto:z72124223@gmail.com)。

## 共用治理

產品與工程規則共用以下原始文件，不在本檔另抄一套。Claude Code 使用下列 import；
透過網頁閱讀時，請開啟 [AGENTS.md](AGENTS.md) 與 [工程協定](docs/contracts/ENGINEERING_PROTOCOL_V1.md)。

@AGENTS.md
@docs/contracts/ENGINEERING_PROTOCOL_V1.md

回覆以繁體中文為主。摘要與原始規則有差異時，先核對同一提交的原始文件與實作，明確說明差異。
評估產品能力時，分開標示文件聲明、原始碼實作、本次執行結果與既有驗收證據。
`PREPARED`、`COMPLETED`、安裝觀察收據與發布成功各有不同範圍。

## 確認正在讀哪一版

- GitHub 預設分支目前為 `product/lattice-control-mvp`；名稱中的 `mvp` 是歷史命名。
- 本機先核對 `git status --short`、`git branch --show-current` 與 `git rev-parse HEAD`。
- 網頁讀者核對[提交紀錄](https://github.com/z72124223/lattice-devos/commits/product/lattice-control-mvp)
  或 [GitHub 分支提交 API](https://api.github.com/repos/z72124223/lattice-devos/commits/product%2Flattice-control-mvp)的倉庫 SHA。
  GitHub 頁面的 `meta name="release"` 不是倉庫提交碼。
- 若抓取內容仍是四元件或「固定 Terra 的網頁 MVP」，先核對來源日期，再讀[README 原文](https://raw.githubusercontent.com/z72124223/lattice-devos/product/lattice-control-mvp/README.md)。
  需要精確引用時，使用該提交 SHA 的固定檔案連結，並在回覆註明 SHA；無法核對時明說版本未確認。

## 依任務讀取

| 目的 | 入口 |
|---|---|
| 了解產品、模型限制與授權 | [README.md](README.md) |
| 安裝或確認平台支援 | [INSTALL_WITH_CODEX.md](INSTALL_WITH_CODEX.md) |
| 啟用、更新、備份或還原 | [docs/customer-runtime.md](docs/customer-runtime.md) |
| Graphify 行為與證據 | [docs/graphify-execution-evidence.md](docs/graphify-execution-evidence.md) |
| 修改或驗證程式 | [package.json](package.json)、[Rust Runtime](apps/lattice-runtime/)、[PostgreSQL 儲存層](crates/lattice-postgres-store/) |

變更驗證依共用工程協定選最小必要範圍。倉庫結構檢查為 `npm.cmd run check`；
Control 測試為 `npm.cmd run control:test`，Node 測試為 `npm.cmd test`。
Rust 使用可用且與倉庫要求相容的工具鏈；不要將未執行的測試寫成通過。

Claude Code 的載入與 import 行為見 [官方文件](https://code.claude.com/docs/en/memory)。
一般 Claude 網頁對話是否已讀取本檔，須以實際讀取結果確認。

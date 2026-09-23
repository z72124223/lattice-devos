# Life-Harness 離線先導基準

本目錄是獨立實驗，不會改正式 DB、ActivityKind、PROGRESS 或執行恢復流程。
重播不執行案例命令；受控採集只使用下述三條隔離 fixture 命令。
不建立正式任務、不傳送案例至外部。前三增量無套件安裝；第四增量僅在工作樹隔離目錄進行授權的 CPU 依賴安裝。LATTICE MCP 本次不可用；
沒有正式任務身分。Control／PostgreSQL／Graphify 三核心及 Codex 原生執行迴圈不變。

## 執行

在 repository 根目錄，以 Node 24 執行（只有內建模組，無需安裝）：

```powershell
node experiments/life-harness/prepare.mjs
node --test experiments/life-harness/capture.test.mjs experiments/life-harness/resolver-capture.test.mjs experiments/life-harness/harness.test.mjs apps/lattice-control/test/execution-recovery.test.mjs apps/lattice-control/test/codex-runtime-resolution.test.mjs
node experiments/life-harness/harness.mjs
node experiments/life-harness/harness.mjs --controlled
node experiments/life-harness/harness.mjs --resolvers
```

`prepare` 重建固定案例，`harness` 驗證案例、來源、標註，重播 A/B，寫入
`results/advisories.jsonl` 與 `results/summary.json`。重播只處理資料，不執行 command 字串。
修改輸入即改變資料雜湊；已有標註將失效，必須重標，不能把開發先導集當保留測試集。

## 來源與適用範圍

來源基準為 `b2422e82ada81307159233e23fffa88189bcc015`。只盤點本 LATTICE
repository 的兩份 2026-08-26 lifecycle receipt 及已知同專案 acceptance 目錄。
兩份 tracked receipt 沒有 native commandExecution failed；已知 2026-09-01
receipt 有 Start-Sleep exitCode 124，屬逾時且無輸出，不能推論工具啟動／依賴解析問題。
因此第一增量的原 20 案 **合格原生失敗為零**，不以合成題補足真實樣本。
該排除紀錄位於相鄰 LATTICE 工作樹
`lattice-control-product-deploy/.lattice/acceptance/codex-lifecycle-2026-09-01T18-12-47.702Z-f6cbdd4e/canonical-evidence.json`
的 `/notifications/202/message/params/item`；原始檔 SHA-256
`4ea596288cec16f20ef43e075b3b724a033748fef7ebea941d2b521b339c015a`。
此檔只用於來源盤點、未匯入語料，不是重播依賴。

`cases.jsonl` 保留一筆原生受控成功反例，其餘為明確標示的合成介面測例。
受控驗收原生執行不等於真實生產案例。只複製固定欄位；省略路徑、帳號、對話、
thread/turn ID、憑證、環境設定。正式 project/task 僅來源有正式引用時保留，否則 null。
歷史事件現在已完成；不據此觸發任何新動作。source.revision（及 advisory 的
source_revision）只指此次 repository **基準版本**，不是所有來源檔已存在的版本，
也不是當時執行程式的版本。合成來源 prepare.mjs 為本次新檔，由 source.sha256
精確綁定內容；原 receipt 記錄 execution HEAD a61c039 且有 dirty diff。
驗證器以固定匯入器重建預期投影，核對基準、selector、來源及完整內容；v1 不接受
任意新來源路徑。匯入更多來源須明確修改匯入器及重新標註。

## 介面與標註規則（凍結 v1）

完整型別由 `validateCase`／`validateAdvisory` 強制檢查；拒絕未知欄位與過長輸入。
case_id 是 `offline-` 開頭的穩定字串，與 LATTICE work ID 無關。
source.kind 僅 real／controlled_replay／synthetic；source.native_observed 為布林。
source.sha256 是來源 UTF-8 文字 CRLF→LF 後的 SHA-256；evidence_sha256 是
`JSON.stringify({event,context})` 的 SHA-256。hash 證明內容一致，不授予權限。
formal_refs.project/task 為字串或 null；context 明記專案匹配、授權、生命週期、
時效、熔斷、類別及平台。缺少可判斷的身分不得推定為適用。

只讓 failed commandExecution、匹配專案、明確授權、active/current、未熔斷且
tool_launch/dependency_resolution 進入診斷。拒絕由既有 isExecutionDenied 優先辨識；
declined、成功、取消、完成、重開、過期、身分不明及錯專案全部排除。
合成 native-shaped item 只用於測試，永遠不計入 real 成績。日誌中的指令不執行。

兩名 AI 子代理獨立標註，不見彼此答案、不見 A/B 輸出；主代理為第三角色仲裁。
這不是人類專家標註，也不主張模型來源獨立。標註只用當下 event/context 與凍結的
`procedures.json` 全集，不看未來修復結果。每列保留 case_id、dataset_sha256、
procedures_sha256、annotator、label、valid_procedures、reason；仲裁額外保留兩份原答案與分歧說明。
候選程序檔內容一旦改動，即使 ID 相同也拒絕沿用舊標註；資料及程序雙重綁定。

- `excluded`：先適用上述排除條件，不推斷診斷；valid_procedures 必須空陣列。
- `valid`：明確符合至少一個程序前置條件；列出所有有效答案，順序不代表偏好。
- `none_applicable`：資訊足夠辨識問題，但程序全集沒有適用者，不能勉強選最近的。
- `insufficient_information`：當下缺少診斷必要證據；不得由最高分強迫選擇。

候選召回只在 valid 案例衡量至少一個有效程序有否進候選。排序錯誤只在已召回
有效程序卻選錯時記錄；漏召回、棄答、none、資訊不足、越界建議分開計數。
分母為零輸出 null，不能稱 100%。按 real／controlled_replay／synthetic 分開報告。

## A/B 的確切含義

本次未取得可直接套用此介面的現有產品診斷推薦基準，因此 A 是本次凍結的**離線確定性基準**，
不是完整 Codex 原生任務完成率：以固定關鍵字檢索兩個已有程序，按明確錯誤特徵
選擇第一個有效候選或棄答，重用現有拒絕辨識。程序提示只是資料，不是操作授權。
B 與 A 相同；原 20 案沒有合格原生失敗，兩個新增受控集合也未觀察到 A 的診斷錯誤，
因此不強造改善。summary 明示 no_change，
不能據此宣稱產品退化、改善、模型品質、端到端效率或正式任務完成率。
這些都是開發先導案例，不提供泛化、校準或統計採用證據。後續若在已授權且可確認
當下適用性的原生切片發現可重現的診斷錯誤或有效程序漏召回，才選定改善並重跑 A/B。

## 已執行結果與限制

`results/verification.txt`：原 16 項、Node 採集 7 項及解析器集合 3 項，共 26 項聚焦測試通過。封存保護回歸在隔離目錄驗證：
已有凍結投影但無本機原文時，採集在寫入前拒絕，既有檔案逐位元組不變且不建立 raw/index。
兩個既有診斷函式另有受控本機
失敗重播，未將函式錯誤冒充 commandExecution。`results/summary.json` 分開保存
19 合成／1 原生受控成功／0 真實案例；同一 20 案各跑 A/B，共 40 筆離線建議紀錄，
不是 40 個獨立樣本。
`results/replay-verification.txt` 記錄舊兩組重播與已驗收基準一致，並核對新四案重播（程式雜湊與實際耗時另列）；
耗時只屬這個離線評估器，不能當作任務效率。候選有效4例、皆不適用2例、
資訊不足1例、排除13例；兩名 AI 盲標無分歧，主代理第三角色逐筆核對。
獨立程式審查已確認兩項溯源修正，最終驗收仍由原協調任務負責。

`results/laya-readiness.json` 僅記錄當時硬體／可用資源與 Python 套件盤點：
當時未下載或載入模型，encoder/tokenizer 與精確依賴相容性仍未知；第四增量的新盤點與失敗另存，不覆寫這份歷史紀錄。
沒有修改正式 DB／恢復流程、外部推論、推送、合併、發布或清理編譯快取。

## 第二增量：三個受控原生切片（2026-09-23）

只使用本 Codex 任務自己的明確 rollout，沒有掃別的任務。原始事件實際為
`event_msg → item_completed → CommandExecution`，命令為陣列，cwd 為 file URL；
status 與 exit_code 分開保存。[官方 App Server 契約](https://learn.chatgpt.com/docs/app-server)
也分列狀態與退出碼，但本次判定依據是實際 rollout 原文，不由文件反推執行結果。

同一 `native-fixtures/` 目錄的三個情境，以目前 Codex exec_command 介面執行：

```powershell
node ./intentionally-absent-entry.mjs
node ./dependency-failure.mjs
node ./success.mjs
```

第一案 Node 已啟動但工具入口檔缺失，第二案匯入的本機相依模組缺失，第三案載入
現有小型依賴成功。前兩案的**原生** status 是 failed、exit_code 是 1；成功案為
completed／0。沒有更改 PATH、全域環境、安裝或既有專案依賴；檔案皆限實驗目錄。
這是 Node 啟動入口／依賴解析範圍，未收 Git 或 Codex 可執行檔故障。

`capture.mjs` 只讀明示的單一 rollout 與 thread/turn ID，核對該回合原始
task_started／turn_context、唯一命令事件、精確 fixture cwd 及事件前未終止。
首次封存介面為 `node experiments/life-harness/capture.mjs <own-rollout> <own-task-id> <turn-id>`；
任何封存輸出（投影、原文、source-index 或案例）已存在，就在建立目錄或寫檔前拒絕；
寫檔也只允許新建。本輪已封存，日常重播使用 `--controlled`，無需重採集。
後續新採集須另保存來源並明示更新已審查投影 digest，不能覆寫這個凍結集合。

原始三事件及當時回合範圍保留在本工作樹被 Git 忽略的
`.lattice/life-harness/native-node-20260923/raw-capture.jsonl`；相鄰 source-index.json
保留原檔與行號。可攜去識別原生形狀在 `native-fixtures/captured-events.jsonl`，
其固定雜湊為 `6fa9f76741d45983f7df534eedfcf150cac7f194ba427a098c4ad373d41c1a3a`。
匯入器核對該 digest 與 fixture 內容；有本機原文時再核對原文／context／capture
雜湊及完整去識別結果，缺席則明記 `native_original=not_available_locally`。
隔離 checkout 可重播投影；不宣稱它重新看到了本機原文。

`controlled-cases.jsonl` 是獨立開發切片，kind 永遠 controlled_replay；將型別大小寫
及 snake_case 欄位映射至既有介面，保留 status 原值。若平台回傳 completed／非零，
仍按原條件排除，不將它改成 failed（另有合成解析測試）。context 是事件當時的
已授權離線採集範圍，不是現行正式任務權限；正式 project/task 仍為 null。

事件、候選及兩名 AI 盲標先凍結，主代理逐筆仲裁後先跑 A。候選庫維持原 Git／Codex
兩程序，兩個 Node 故障資訊足夠但皆不適用，成功案排除；不是漏召回。
A 與標註一致，無分歧，B 維持 no_change；來源校驗的修正不充作 B 的診斷改善。
`results/controlled-summary.json`／`controlled-advisories.jsonl` 保存 3 案各跑 A/B
的 6 筆紀錄，原 20 案仍是先導回歸資料，不能當保留集。
本輪證明受控原生來源可以誠實取得；沒有證明診斷改善、產品無改善空間、
真實效益或模型品質，也未設定真實生產樣本數門檻。

## 第三增量：Git／Codex 解析器受控切片

從 `13c5478cc044e0e81efe4e68645c40f5b08f1e50` 開始，舊 20 案、Node 3 案、
標註、程序及原始封存保持凍結。新集合為 `resolver-cases.jsonl`，原生去識別投影為
`resolver-fixtures/captured-events.jsonl`，結果與標註使用 `resolver-` 前綴。

只在 `resolver-fixtures/` 執行下列四條命令，每條都是獨立原生命令事件：

```powershell
node ./resolve.mjs git missing
node ./resolve.mjs codex missing
node ./resolve.mjs git success
node ./resolve.mjs codex success
```

fixture 直接呼叫既有 `resolveGitExecutable`／`resolveWindowsCodexRuntime`，
不攔截或重寫例外。Git 失敗案僅傳 `pathValue: ''`，Codex 失敗案僅傳 `env: {}`；
原生輸出保留這些參數。這證明受限搜尋範圍內解析失敗，不表示電腦未安裝工具。
沒有更改 process.env、使用者或機器 PATH，沒有安裝、帳號、憑證或 DB 變動。
兩筆失敗的實際原生狀態均為 failed／1，兩筆成功為 completed／0；來源是本任務
同一回合、唯一命令與 cwd 的原始 `CommandExecution`，不從退出碼推定 failed。

成功案使用既有環境，只做解析。Git 回傳 `git.exe`；Codex 回傳 `node.exe`，
表示既有 npm fallback 找到 script。Codex 解析器可能做內建 `--version` 探測，
fixture 不執行回傳命令或 `app-server --stdio`，所以成功不代表 App Server 已啟動或可用。

本機原文位於被 Git 忽略的 `.lattice/life-harness/native-resolvers-20260923/`
`raw-capture.jsonl`，相鄰 `source-index.json` 記錄來源 rollout、回合及行號。
原文包含四個事件、原始 task_started 和 turn_context；只讀本任務自己的明確來源。
新投影另綁定 fixture 及兩個實際解析器原始碼的雜湊；原文與投影會逐筆核對。
既有封存保護同樣適用新集合，`--resolvers` 重播不執行 fixture 命令。

投影 SHA-256 為 `b33f147c7e3eef1b484b215c2bab14cec8c7fd89f48108f88894537e1642471f`，
案例 SHA-256 為 `1f1b8b54b623a9916d819315fdcb8963be42fef2e1f2484efd4f5accfdbd4ee2`。
兩名 AI 獨立標註者只讀凍結事件、fixture 及程序條件；標註與第三角色仲裁完成後才跑 A。
四案為 valid 2、excluded 2，無分歧。A 的有效程序召回為 **2/2**，已召回後選擇正確為
**2/2**，成功排除為 **2/2**；漏召回、排序錯誤與越界皆 0。這些分母各自獨立，
不能拿整體標籤正確率替代診斷準確率。此小集合有零個 none_applicable；舊 Node
兩個 none_applicable 仍是棄答反例，不計入召回漏失。

A 已正確，所以 B 不修改檢索或診斷，仍為 no_change；`results/resolver-summary.json`
與 `resolver-advisories.jsonl` 可重播 A/B。同一四案各跑 A/B 共 8 筆建議，仍只有四個案例。
新集合只驗證既有兩程序的受控適用路徑；沒有真實生產案例、診斷改善量、泛化或模型品質證據。
Codex Desktop 解析成功路徑、App Server 啟動及正式恢復仍未在這個增量驗證。

## 第四增量：Laya 本機 CPU 可行性，停於安裝失敗

從已驗收的 `792e0104f84d0132d84f23a506ad1ae176c1812b` 開始，僅準備隔離的
local choice 路徑。**35 個固定相依套件已安裝成功；Laya SDK 尚未安裝成功；
模型尚未載入，smoke 與三組 advisory 均未執行。** 兩次安裝皆退出 1，已依同一路徑
兩次失敗即停的界線停止。原協調任務收到阻礙後明示先封存，不進行第三次安裝、
修改 TEMP、重建 venv 或刪除既有下載。

`laya-environment.json` 保存全部 35 個 wheel 的精確版本、官方 URL、尺寸及 SHA-256；
`laya-requirements.lock.txt` 是對應的固定依賴清單。使用 bundled Python **3.12.14**，
關鍵套件為 torch **2.7.1+cpu**、transformers **5.0.0**、numpy **2.3.5**、
tokenizers **0.22.2**、huggingface-hub **1.3.5**、safetensors **0.7.0**。
已透過安裝目錄 metadata 逐一確認版本，尚未以匯入／載入模型證明執行相容性。

- [SDK 固定原始碼](https://github.com/NandhaKishorM/laya/tree/010bacef009c855ccba814b51f7c8e1d38ab5e3f)：
  `010bacef009c855ccba814b51f7c8e1d38ab5e3f`，宣告版本 0.3.7、Apache-2.0。
- [模型固定 snapshot](https://huggingface.co/convaiinnovations/laya-multilingual/tree/b4a904d1a2a54c822b829e24291d4b8f280fe43e)：
  `b4a904d1a2a54c822b829e24291d4b8f280fe43e`，模型卡宣告 Apache-2.0。
  六個檔案含模型卡共 **678,209,751 bytes**，均已下載並驗證。
- Encoder 上游名稱是 `jhu-clsp/mmBERT-base`，此次實際 encoder config 與 tokenizer
  都取自上述固定 Laya snapshot，沒有另外下載未鎖版的 base encoder weights。
  SDK CPU 路徑使用 FP32；config 中的 BF16 設定不是本機 CPU 已驗收的 dtype。

第一次 `pip-install` 在 PyTorch ATen 深層標頭檔遇到 ENOENT，pip 提示可能是 Windows 路徑長度限制。
第二次只將 pip 的 Python 路徑改用**已存在、且 samefile 核對相同的 NTFS 8.3 別名**，
沒有改全域 PATH、registry 或建立磁碟映射；相依套件安裝成功。接著 SDK wheel 建置
在仍為長路徑的 TEMP 目錄遇到 **WinError 206**，因此第二次整體安裝仍退出 1。
可能的 TEMP 短別名修正尚未套用，也未再次執行。

`results/laya-install.json` 保存兩次失敗、精確已安裝版本、原文 digest 與資源盤點。
原始 stdout/stderr 保留於被忽略的 `.lattice/life-harness/laya-local/`：
`pip-install.txt`、`pip-install-2.txt`、`sdk-install.txt`；下載、snapshot 與 venv 也在同處。
下載前保守額外估算 **4,227,578,028 bytes**；停止時新增檔案 **2,317,854,182 bytes**，
C 槽可用 **18,197,262,336 bytes**，符合新增不超過 6 GiB／可用至少 10 GiB。
未清除編譯快取或任何使用者資料。模型載入時間、推論時間與推論峰值 RSS 都是 null，
不能把未執行解讀成零成本。

已準備但**未做真實模型驗收**的執行路徑為 `laya-run.mjs`／`laya_choice.py`：
先套用 Windows Job Object 的程序及整體 8 GiB 提交記憶體限制，再加入本機套件路徑；
`-I -S -B` 停用 site／.pth 啟動，外層 300 秒逾時與磁碟監控，另讀 Windows 峰值 RSS。
本機 snapshot 驗證後使用完整工作副本，避免 SDK 改 tokenizer config 時修改原件。
Hub 與 tokenizer/config loader 僅接受本機目錄；停用 Python socket 與子程序、移除繼承的
token／proxy 環境。這是已審查 SDK 的 Python 層離線控制，**不是作業系統防火牆**。

`laya-inputs.mjs` 先重用來源校驗、排除條件與固定檢索，不讀 gold。27 案中 16 案排除、
3 案空候選直接棄答、8 案產生請求；後者仍須由真實 tokenizer 證明無截斷，超長即棄答。
保留兩種明示棄答與原始 SDK 分數，分數不當作校準信心或採用門檻。未執行模型，
所以目前沒有 C/D 成績；若後續有授權且完成執行，A=B 時只保存一次相同輸入的 C=D
觀察，不宣稱獨立實驗、因果改善或產品效益。`laya-summarize.mjs` 為該後續評估的未執行程式。

新增前置輸入 7 項測試與原有 26 項合計 **33 項 Node 測試通過**；
**4 項 Python 限制測試通過**，涵蓋 OS 限制讀回、失敗拒跑及 socket 阻擋。
這些結果只證明輸入與限制，不證明模型可用。精確命令及獨立來源／舊資料核對
保存在 `results/laya-verification.json`；原三組案例、標註、程序、原文與 A/B 結果均保持不變。

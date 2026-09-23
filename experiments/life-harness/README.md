# Life-Harness 離線先導基準

本目錄是獨立實驗，不會改正式 DB、ActivityKind、PROGRESS 或執行恢復流程。
重播不執行案例命令；受控採集只使用下述三條隔離 fixture 命令。
不建立任務、不安裝、不傳送外部資料。LATTICE MCP 本次不可用；
沒有正式任務身分。Control／PostgreSQL／Graphify 三核心及 Codex 原生執行迴圈不變。

## 執行

在 repository 根目錄，以 Node 24 執行（只有內建模組，無需安裝）：

```powershell
node experiments/life-harness/prepare.mjs
node --test experiments/life-harness/capture.test.mjs experiments/life-harness/harness.test.mjs apps/lattice-control/test/execution-recovery.test.mjs apps/lattice-control/test/codex-runtime-resolution.test.mjs
node experiments/life-harness/harness.mjs
node experiments/life-harness/harness.mjs --controlled
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
B 與 A 相同；在沒有合格原生失敗證據時，不強造改善。summary 明示 no_change，
不能據此宣稱產品退化、改善、模型品質、端到端效率或正式任務完成率。
這些都是開發先導案例，不提供泛化、校準或統計採用證據。後續須有已授權且可確認
當下適用性的原生失敗切片，才有理由選定一項改善，再跑真正有差異的 A/B。

## 已執行結果與限制

`results/verification.txt`：原 16 項加新增 7 項，共 23 項聚焦測試通過。新增封存保護回歸在隔離目錄驗證：
已有凍結投影但無本機原文時，採集在寫入前拒絕，既有檔案逐位元組不變且不建立 raw/index。
兩個既有診斷函式另有受控本機
失敗重播，未將函式錯誤冒充 commandExecution。`results/summary.json` 分開保存
19 合成／1 原生受控成功／0 真實案例；同一 20 案各跑 A/B，共 40 筆離線建議紀錄，
不是 40 個獨立樣本。
`results/replay-verification.txt` 記錄兩組資料重播與已驗收基準一致（程式雜湊與實際耗時除外）；
耗時只屬這個離線評估器，不能當作任務效率。候選有效4例、皆不適用2例、
資訊不足1例、排除13例；兩名 AI 盲標無分歧，主代理第三角色逐筆核對。
獨立程式審查已確認兩項溯源修正，最終驗收仍由原協調任務負責。

`results/laya-readiness.json` 僅記錄當時硬體／可用資源與 Python 套件盤點：
未下載或載入模型，encoder/tokenizer 與精確依賴相容性仍未知。
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

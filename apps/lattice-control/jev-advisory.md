# Jev 唯讀建議入口

`FormalTaskService.jevAdvisory(projectId, taskRef, request)` 是程序內明示函式，預設停用。
可信啟動組裝若要啟用，在原服務建構參數加入 `jevAdvisory: { enabled: true }`；
省略設定或使用 `enabled: false` 即關閉。沒有新增 HTTP、MCP、CLI、事件觸發或排程。
必須沿用已持有正式工作與活動原生回合的同一服務，不能為建議另啟或恢復回合。

Codex 明示要求唯讀建議後，由持有該服務的程序呼叫：

```js
const advice = await formalTasks.jevAdvisory(projectId, taskRef, {
  intent: 'read-only-advice',
  claimId, threadId, turnId, failureItemId,
});
```

這些參數只選取資料，不證明授權或來源。入口重新讀取 Runtime 的正式 claim、ledger、
project snapshot，以及同一 Codex App Server 的 thread 與 item/completed 通知。
通知須與原生失敗完全相符且不超過五分鐘；失去原通知、身分／狀態不明、封存、完成、
待答許可、拒絕、熔斷或讀取期間來源變動，均在傳送前停止。整次呼叫期限為五秒，
慢速來源讀取逾時會釋放工作佇列並忽略晚到結果；
最後傳送前與回應後也核對原生連線、回合、拒絕與許可序號。
底層 Runtime 唯讀請求不會被取消，共享客戶端佇列仍受既有請求逾時限制。
五秒總窗包含來源核對與模型等待；一例受控真實呼叫耗時 4785.224 毫秒，
餘裕約 215 毫秒，尚未證明穩定性。

第一版只接受可辨識的原生 Windows PowerShell 命令封裝，以及既有 Git／Codex
執行檔解析失敗特徵；其他平台拒絕，Node 相對模組缺失為 `none_applicable`。
字串匹配只代表建議可能適用，不證明實際失敗原因或 resolver producer。

HTTP body 的 `state` 僅有固定版本 `schema`、`platform`、`category`、`failure` 列舉與
`failed` 布林值；其他內容為固定模型和固定選項。命令、路徑、原始碼、日誌、工作／
對話身分、呼叫者額外欄位不會外送。來源通過後才讀取既有 `TYPESAFE_API_KEY`，
只用於 Authorization header。此入口不讀取 DPAPI 檔或建立／修改金鑰。

沿用 `jev-1.13.0`、固定端點、8 KiB 請求／16 KiB 回應上限、五秒 HTTP 期限、
拒絕轉址、不重試與嚴格回應結構。信心低於 0.8、版本錯誤、畸形回應、錯用途均棄權；
信心門檻未經校準。始終 `advisory_only=true`、`adopted=false`、
`authorization_verified=false`、`diagnostic_semantics_verified=false`。
`native_source_verified` 只描述本次本機來源核對，不授予任何操作權限。
函式沒有 steer、interrupt、repair、adopt 或耐久寫入效果。

測試可在可信建構參數提供 `transport`；回傳會標明 `mode=simulated` 和模擬 usage。
聚焦單元測試使用記憶體 Runtime／native 替身與模擬 HTTP，驗證本機接線與拒絕行為。
2026-10-03 另以產品版本 `46daf294` 完成一例受控真實接線：既有 Git resolver
以空 `pathValue` 選項產生原始解析失敗，再由持有同一正式活動回合的服務明示呼叫入口。
唯一一次 Jev HTTP 回覆為 200，總耗時 4785.224 毫秒，選出 `git-resolution`；
`native_source_verified=true`，usage 由伺服器回報且非模擬，建議與信任旗標維持上述限制。
保存證據後依設計中止專用回合；新程序讀回的回合狀態為 `INTERRUPTED`，正式工作仍為
`SUBMITTED`／`DRAFT`、result digest 為空，未建立驗收回合或執行修復。
這一例只證明真實來源、有限分類投影與 Jev 產品入口的單次接線，
不代表自然故障品質、信心校準、一般成功率、穩定性、建議採納或正式工作完成。
既有實驗入口與精確合成資料 allowlist 維持原有行為。

聚焦測試：

```sh
node --test apps/lattice-control/test/jev-task-advisory.test.mjs experiments/life-harness/jev-choice.test.mjs
```

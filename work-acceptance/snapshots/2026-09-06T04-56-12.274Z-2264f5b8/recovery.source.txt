// Codex owns execution and approvals. LATTICE only redirects its owned turn
// and bounds repeated denials; it never relaxes or rewrites a platform policy.
export const recoveryPrompt = `執行熔斷與替代方案：遇到明確政策拒絕或使用者拒絕時，立即停止重試該動作。先核對原始失敗與目前限制，再自行尋找已授權、實質不同且風險更低的替代方法，繼續推進原目標與不受影響的工作。替代方法必須有明確可驗證的成果；例如使用允許的內建瀏覽器驗證網頁，但不能把網頁驗收冒充桌面驗收。不要靠換 shell、包裝指令、改權限、停用安全控制或換工具重做同一個被拒絕的動作。不得把先前同意或時間經過當作平台已放行，也不要要求使用者尋找未實際存在的核准按鈕。同一回合再次遭拒絕時結束嘗試，回報已完成的部分、仍缺的驗收與可採用的下一步；未驗證的成果不可宣稱完成。`;

export const recoverySummary = "執行遭拒：已停止原動作，AI 正核對原因並尋找可驗證的替代方案。";
export const openCircuitSummary = "替代過程再次遭拒，已熔斷這輪自動嘗試；成果尚未驗收完成，請先核對限制與可行替代方案。";

const policyDenial = /(?:rejected:[\s\S]{0,160}blocked by policy|blocked by policy|policy forbids commands|approval required by policy|user rejected (?:the )?tool call)/iu;
function contentText(items) {
  return Array.isArray(items) ? items.filter((part) => ["text", "inputText"].includes(part?.type))
    .map((part) => part.text ?? "").join("\n") : "";
}

// Only native failed/declined tool results count. Prompts, successful searches,
// source code and assistant quotations of an error are not denial events.
export function isExecutionDenied(item) {
  if (!item || typeof item.id !== "string" || !item.id) return false;
  if (item.type === "commandExecution") {
    return item.status === "declined" || (item.status === "failed" && policyDenial.test(item.aggregatedOutput ?? ""));
  }
  if (item.type === "mcpToolCall" && item.status === "failed") {
    return policyDenial.test(item.error?.message ?? contentText(item.result?.content));
  }
  if (item.type === "dynamicToolCall" && (item.status === "failed" || item.success === false)) {
    return policyDenial.test(contentText(item.contentItems));
  }
  return false;
}

export function deniedItemIds(turn) {
  return new Set((turn?.items ?? []).filter(isExecutionDenied).map((item) => item.id));
}

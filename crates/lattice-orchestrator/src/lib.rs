//! Pure injected-port delivery orchestration for LATTICE.

use std::error::Error;
use std::fmt;

use lattice_contracts::{
    CodexDeliveryRequest, CompletedDeliveryEvidence, DeliveryContractError, DeliveryOutcomeRequest,
    DeliveryReceipt, DeliveryRunRequest, DeliveryStage, DeliveryStatusRequest,
    DeliveryTerminalStatus, DurableIntentEvidence,
};
use lattice_ports::{
    DeliveryCodexPort, DeliveryFailureCertainty, DeliveryLedgerPort, DeliveryPortError,
    PortErrorKind, TestRunnerPort, WorkspaceGitPort,
};

/// Pure coordinator failure. A terminal stage failure may still carry its
/// independently reloaded durable receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryOrchestratorError {
    /// Intent was not proved durable, so no later effect was attempted.
    Intent(DeliveryPortError),
    /// Trusted stage evidence violated the shared contract.
    Contract(DeliveryContractError),
    /// A terminal record could not be durably written.
    OutcomePersistence(DeliveryPortError),
    /// Durable receipt readback failed.
    ReceiptRead(DeliveryPortError),
    /// Durable readback did not match the exact request/outcome just written.
    ReceiptMismatch,
    /// A known or ambiguous stage failed and its terminal receipt was verified.
    Terminal {
        cause: DeliveryPortError,
        receipt: Box<DeliveryReceipt>,
    },
}

impl fmt::Display for DeliveryOrchestratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Intent(error) => write!(formatter, "delivery intent rejected: {error}"),
            Self::Contract(error) => write!(formatter, "delivery contract rejected: {error}"),
            Self::OutcomePersistence(error) => {
                write!(formatter, "delivery outcome persistence rejected: {error}")
            }
            Self::ReceiptRead(error) => write!(formatter, "delivery receipt rejected: {error}"),
            Self::ReceiptMismatch => formatter.write_str("delivery receipt cross-binding"),
            Self::Terminal { cause, .. } => write!(formatter, "delivery stage failed: {cause}"),
        }
    }
}

impl Error for DeliveryOrchestratorError {}

/// Runs one delivery using only injected abstract ports.
///
/// The fixed effect order is intent -> workspace preparation -> Codex ->
/// changed-path inspection -> fixed test -> Git commit -> terminal outcome ->
/// independent receipt readback.
///
/// # Errors
///
/// Returns immediately after the first failed gate. Failures after durable
/// intent are themselves recorded as failed or reconciliation-required before
/// the function returns, unless terminal persistence/readback is unavailable.
pub fn run_delivery<L, C, W>(
    request: &DeliveryRunRequest,
    ledger: &mut L,
    workspace_git: &mut W,
    codex: &mut C,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError>
where
    L: DeliveryLedgerPort,
    C: DeliveryCodexPort,
    W: WorkspaceGitPort + TestRunnerPort,
{
    let intent = ledger
        .record_intent(request)
        .map_err(DeliveryOrchestratorError::Intent)?;

    let workspace = match workspace_git.prepare(request, &intent) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    let codex_request =
        match CodexDeliveryRequest::new(request.clone(), intent.clone(), workspace.clone()) {
            Ok(request) => request,
            Err(error) => {
                return finish_contract_failure(
                    ledger,
                    request,
                    &intent,
                    DeliveryStage::WorkspacePrepare,
                    error,
                );
            }
        };
    let codex_evidence = match codex.run_delivery(codex_request) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    let changes = match workspace_git.inspect_changes(request, &intent, &workspace, &codex_evidence)
    {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    let test = match workspace_git.run_fixed(request, &workspace, &changes) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };
    let git = match workspace_git.commit(request, &workspace, &changes, &test) {
        Ok(evidence) => evidence,
        Err(error) => return finish_failure(ledger, request, &intent, error),
    };

    let completed = match CompletedDeliveryEvidence::new(
        request.clone(),
        intent.clone(),
        workspace,
        codex_evidence,
        changes,
        test,
        git,
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            return finish_post_commit_contract_failure(ledger, request, &intent, error);
        }
    };
    let outcome_request = match DeliveryOutcomeRequest::completed(request, completed) {
        Ok(outcome) => outcome,
        Err(error) => {
            return finish_post_commit_contract_failure(ledger, request, &intent, error);
        }
    };
    let outcome = ledger
        .record_outcome(&outcome_request)
        .map_err(outcome_persistence_after_durable_intent)?;
    let receipt = ledger
        .load_receipt(&request.status_request())
        .map_err(DeliveryOrchestratorError::ReceiptRead)?;
    if !receipt.matches_run(request)
        || receipt.status() != DeliveryTerminalStatus::Completed
        || receipt.outcome() != &outcome
    {
        return Err(DeliveryOrchestratorError::ReceiptMismatch);
    }
    Ok(receipt)
}

/// Loads one terminal delivery receipt without invoking any other port.
///
/// # Errors
///
/// Returns a receipt error or cross-binding error when durable state cannot be
/// safely associated with the exact status request.
pub fn delivery_status<L: DeliveryLedgerPort>(
    request: &DeliveryStatusRequest,
    ledger: &mut L,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let receipt = ledger
        .load_receipt(request)
        .map_err(DeliveryOrchestratorError::ReceiptRead)?;
    if !receipt.matches_status_request(request) {
        return Err(DeliveryOrchestratorError::ReceiptMismatch);
    }
    Ok(receipt)
}

fn finish_contract_failure<L: DeliveryLedgerPort>(
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    stage: DeliveryStage,
    error: DeliveryContractError,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let port_error = DeliveryPortError::new(
        stage,
        PortErrorKind::Malformed,
        DeliveryFailureCertainty::Known,
        "CONTRACT_EVIDENCE_REJECTED",
    );
    finish_failure(ledger, request, intent, port_error).map_err(|terminal| match terminal {
        DeliveryOrchestratorError::Terminal { .. } => DeliveryOrchestratorError::Contract(error),
        other => other,
    })
}

fn finish_post_commit_contract_failure<L: DeliveryLedgerPort>(
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    _error: DeliveryContractError,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let port_error = DeliveryPortError::new(
        DeliveryStage::GitCommit,
        PortErrorKind::Ambiguous,
        DeliveryFailureCertainty::Ambiguous,
        "POST_COMMIT_EVIDENCE_REJECTED",
    );
    finish_failure(ledger, request, intent, port_error)
}

fn finish_failure<L: DeliveryLedgerPort>(
    ledger: &mut L,
    request: &DeliveryRunRequest,
    intent: &DurableIntentEvidence,
    cause: DeliveryPortError,
) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
    let ambiguous = cause.certainty() == DeliveryFailureCertainty::Ambiguous
        || cause.kind() == PortErrorKind::Ambiguous;
    let outcome_request = if ambiguous {
        DeliveryOutcomeRequest::reconciliation_required(
            request,
            intent,
            cause.stage(),
            cause.code(),
        )
    } else {
        DeliveryOutcomeRequest::failed(request, intent, cause.stage(), cause.code())
    }
    .map_err(DeliveryOrchestratorError::Contract)?;
    let expected_status = outcome_request.status();
    let outcome = ledger
        .record_outcome(&outcome_request)
        .map_err(outcome_persistence_after_durable_intent)?;
    let receipt = ledger
        .load_receipt(&request.status_request())
        .map_err(DeliveryOrchestratorError::ReceiptRead)?;
    if !receipt.matches_run(request)
        || receipt.status() != expected_status
        || receipt.outcome() != &outcome
    {
        return Err(DeliveryOrchestratorError::ReceiptMismatch);
    }
    Err(DeliveryOrchestratorError::Terminal {
        cause,
        receipt: Box::new(receipt),
    })
}

fn outcome_persistence_after_durable_intent(
    _error: DeliveryPortError,
) -> DeliveryOrchestratorError {
    DeliveryOrchestratorError::OutcomePersistence(DeliveryPortError::new(
        DeliveryStage::Outcome,
        PortErrorKind::Ambiguous,
        DeliveryFailureCertainty::Ambiguous,
        "OUTCOME_PERSISTENCE_AFTER_DURABLE_INTENT_UNKNOWN",
    ))
}

use lattice_contracts::DeliveryStage;
use lattice_ports::{DeliveryFailureCertainty, DeliveryPortError, PortErrorKind};

#[test]
fn delivery_port_errors_keep_stage_and_effect_certainty_explicit() {
    let error = DeliveryPortError::new(
        DeliveryStage::GitCommit,
        PortErrorKind::Ambiguous,
        DeliveryFailureCertainty::Ambiguous,
        "commit-outcome-unknown",
    );

    assert_eq!(error.stage(), DeliveryStage::GitCommit);
    assert_eq!(error.kind(), PortErrorKind::Ambiguous);
    assert_eq!(error.certainty(), DeliveryFailureCertainty::Ambiguous);
    assert_eq!(error.code(), "commit-outcome-unknown");
}

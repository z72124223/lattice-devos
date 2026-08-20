use lattice_contracts::{
    CHATGPT_SECURE_MCP_TUNNEL_ACTOR_ID, CHATGPT_SECURE_MCP_TUNNEL_ADAPTER_ID, ContentDigest,
    ContractError, GatewayChannelId, GatewayInstanceId, LOCAL_CANONICAL_MCP_ACCEPTANCE_ACTOR_ID,
    LOCAL_CANONICAL_MCP_ACCEPTANCE_ADAPTER_ID, RuntimeKind, TaskIngressActorKind,
    TaskIngressClientKind, TaskIngressPeerEvidence,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn live_peer() -> TaskIngressPeerEvidence {
    TaskIngressPeerEvidence::new_chatgpt_secure_mcp_tunnel_live(
        GatewayInstanceId::new("lattice-mcp-production").expect("gateway instance"),
        "1.0.0",
        digest('a'),
        digest('b'),
        GatewayChannelId::new("stdio").expect("channel"),
        digest('c'),
        digest('d'),
    )
    .expect("live peer")
}

#[test]
fn fixed_chatgpt_mcp_peer_is_live_closed_and_server_evidence_only() {
    let peer = live_peer();

    assert_eq!(peer.runtime(), RuntimeKind::Live);
    assert_eq!(
        peer.client_kind(),
        TaskIngressClientKind::ChatGptSecureMcpTunnel
    );
    assert_eq!(
        peer.actor_kind(),
        TaskIngressActorKind::ControlledServiceProfile
    );
    assert_eq!(
        peer.gateway_instance_id().as_str(),
        "lattice-mcp-production"
    );
    assert_eq!(
        peer.adapter_id().as_str(),
        CHATGPT_SECURE_MCP_TUNNEL_ADAPTER_ID
    );
    assert_eq!(peer.adapter_version(), "1.0.0");
    assert_eq!(peer.adapter_binary_digest(), &digest('a'));
    assert_eq!(peer.schema_digest(), &digest('b'));
    assert_eq!(peer.actor_id().as_str(), CHATGPT_SECURE_MCP_TUNNEL_ACTOR_ID);
    assert_eq!(peer.channel_id().as_str(), "stdio");
    assert_eq!(peer.profile_digest(), &digest('c'));
    assert_eq!(peer.process_start_authority_digest(), &digest('d'));
}

#[test]
fn local_acceptance_peer_cannot_impersonate_the_chatgpt_secure_tunnel() {
    let tunnel = live_peer();
    let local = TaskIngressPeerEvidence::new_local_canonical_mcp_acceptance_live(
        GatewayInstanceId::new("lattice-mcp-local-acceptance").expect("gateway instance"),
        "1.0.0",
        digest('a'),
        digest('b'),
        GatewayChannelId::new("stdio").expect("channel"),
        digest('c'),
        digest('d'),
    )
    .expect("local peer");

    assert_eq!(
        local.client_kind(),
        TaskIngressClientKind::LocalCanonicalMcpAcceptance
    );
    assert_eq!(
        local.actor_kind(),
        TaskIngressActorKind::LocalAcceptanceHarness
    );
    assert_eq!(
        local.adapter_id().as_str(),
        LOCAL_CANONICAL_MCP_ACCEPTANCE_ADAPTER_ID
    );
    assert_eq!(
        local.actor_id().as_str(),
        LOCAL_CANONICAL_MCP_ACCEPTANCE_ACTOR_ID
    );
    assert_ne!(local.client_kind(), tunnel.client_kind());
    assert_ne!(local.actor_kind(), tunnel.actor_kind());
    assert_ne!(local.adapter_id(), tunnel.adapter_id());
    assert_ne!(local.actor_id(), tunnel.actor_id());
}

#[test]
fn fixed_chatgpt_mcp_peer_rejects_uncommitted_live_evidence() {
    let build = |adapter_binary_digest: ContentDigest,
                 profile_digest: ContentDigest,
                 process_start_authority_digest: ContentDigest| {
        TaskIngressPeerEvidence::new_chatgpt_secure_mcp_tunnel_live(
            GatewayInstanceId::new("lattice-mcp-production").expect("gateway instance"),
            "1.0.0",
            adapter_binary_digest,
            digest('b'),
            GatewayChannelId::new("stdio").expect("channel"),
            profile_digest,
            process_start_authority_digest,
        )
    };

    let zero = digest('0');
    assert_eq!(
        build(zero.clone(), digest('c'), digest('d')),
        Err(ContractError::InvalidGatewayValue {
            field: "task_ingress_commitment_source"
        })
    );
    assert_eq!(
        build(digest('a'), zero.clone(), digest('d')),
        Err(ContractError::InvalidGatewayValue {
            field: "task_ingress_commitment_source"
        })
    );
    assert_eq!(
        build(digest('a'), digest('c'), zero),
        Err(ContractError::InvalidGatewayValue {
            field: "task_ingress_commitment_source"
        })
    );
}

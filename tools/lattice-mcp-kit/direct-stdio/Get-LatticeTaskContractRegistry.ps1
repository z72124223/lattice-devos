[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

[ordered]@{
    schema = 'lattice.task-contract-registry.v1'
    entries = @(
        [ordered]@{
            contract_schema = 'lattice.task-contract.v1'
            contract_type = 'controlled_codex_canary'
            parameter_fields = @()
            mcp_tool = 'lattice_task_submit'
            intent = 'CONTROLLED_CODEX_CANARY'
            submit_fields = @('client_request_id', 'intent')
        }
    )
} | ConvertTo-Json -Compress -Depth 5

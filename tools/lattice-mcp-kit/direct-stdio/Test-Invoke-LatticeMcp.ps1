[CmdletBinding(DefaultParameterSetName = 'Test')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'Server')]
    [switch]$FixtureServer,

    [Parameter(ParameterSetName = 'Server')]
    [switch]$MismatchedCatalog
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:Utf8 = [Text.UTF8Encoding]::new($false)
$script:ExpectedTools = @(
    'lattice_delivery_reconcile',
    'lattice_delivery_run',
    'lattice_delivery_status',
    'lattice_foreman_checkpoint',
    'lattice_runtime_status',
    'lattice_task_status',
    'lattice_task_submit'
)

function Write-FixtureResponse {
    param([Parameter(Mandatory = $true)]$Message)

    [Console]::Out.WriteLine(($Message | ConvertTo-Json -Compress -Depth 30))
    [Console]::Out.Flush()
}

function Start-FixtureServer {
    [Console]::InputEncoding = $script:Utf8
    [Console]::OutputEncoding = $script:Utf8
    while ($true) {
        $line = [Console]::In.ReadLine()
        if ($null -eq $line) { return }
        $request = $line | ConvertFrom-Json -ErrorAction Stop
        if ($request.PSObject.Properties.Name -notcontains 'id') { continue }
        switch ([int]$request.id) {
            1 {
                Write-FixtureResponse -Message ([ordered]@{
                    jsonrpc = '2.0'
                    id = 1
                    result = [ordered]@{ protocolVersion = '2025-11-25' }
                })
            }
            2 {
                $tools = @($script:ExpectedTools)
                if ($MismatchedCatalog) { $tools = @($tools | Select-Object -SkipLast 1) }
                Write-FixtureResponse -Message ([ordered]@{
                    jsonrpc = '2.0'
                    id = 2
                    result = [ordered]@{
                        tools = @($tools | ForEach-Object { [ordered]@{ name = $_ } })
                    }
                })
            }
            3 {
                Write-FixtureResponse -Message ([ordered]@{
                    jsonrpc = '2.0'
                    id = 3
                    result = [ordered]@{
                        isError = $false
                        content = @()
                        structuredContent = [ordered]@{
                            received_tool = [string]$request.params.name
                            received_arguments = $request.params.arguments
                        }
                    }
                })
                return
            }
            default { throw 'FIXTURE_REQUEST_ID_REJECTED' }
        }
    }
}

if ($FixtureServer) {
    Start-FixtureServer
    exit 0
}

$wrapper = Join-Path $PSScriptRoot 'Invoke-LatticeMcp.ps1'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('lattice-direct-stdio-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )

    if (-not $Condition) { throw ('ASSERTION_FAILED|' + $Message) }
}

function Assert-ExactFields {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    $actual = @($Object.PSObject.Properties.Name)
    Assert-True -Condition (($actual -join "`n") -ceq ($Expected -join "`n")) -Message $Message
}

function Invoke-WrapperCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][ValidateSet('Discovery', 'TaskSubmit', 'TaskStatus')][string]$Action,
        [AllowNull()][AllowEmptyString()][string]$Objective,
        [AllowNull()][AllowEmptyString()][string]$ProjectId,
        [AllowNull()][AllowEmptyString()][string]$ProjectName,
        [AllowNull()][string]$TaskRef,
        [AllowNull()][AllowEmptyString()][string]$ClientRequestId,
        [switch]$UseObjective,
        [switch]$UseProjectId,
        [switch]$UseProjectName,
        [switch]$UseClientRequestId,
        [switch]$UseMismatchedCatalog
    )

    $hostPath = (Get-Process -Id $PID).Path
    $binaryArguments = @('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $PSCommandPath, '-FixtureServer')
    if ($UseMismatchedCatalog) { $binaryArguments += '-MismatchedCatalog' }
    $parameters = @{
        BinaryPath = $hostPath
        BinaryArgument = $binaryArguments
        Action = $Action
        TimeoutSeconds = 5
        ToolCallTimeoutSeconds = 5
        OutputDirectory = (Join-Path $testRoot $Name)
    }
    if ($UseClientRequestId) {
        $parameters.ClientRequestId = $(if ([string]::IsNullOrEmpty($ClientRequestId)) { 'offline-' + $Name } else { $ClientRequestId })
    }
    if ($UseObjective) { $parameters.Objective = $Objective }
    if ($UseProjectId) { $parameters.ProjectId = $ProjectId }
    if ($UseProjectName) { $parameters.ProjectName = $ProjectName }
    if (-not [string]::IsNullOrEmpty($TaskRef)) { $parameters.TaskRef = $TaskRef }
    $output = @(& $wrapper @parameters)
    return ([string]::Join([Environment]::NewLine, @($output | ForEach-Object { [string]$_ }))) |
        ConvertFrom-Json -ErrorAction Stop
}

try {
    $tokens = $null
    $parseErrors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile($wrapper, [ref]$tokens, [ref]$parseErrors)
    Assert-True -Condition ($parseErrors.Count -eq 0) -Message 'wrapper parses'
    $parameters = (Get-Command -Name $wrapper).Parameters
    foreach ($required in @('Objective', 'ProjectId', 'ProjectName')) {
        Assert-True -Condition $parameters.ContainsKey($required) -Message ('wrapper parameter ' + $required)
    }

    $discovery = Invoke-WrapperCase -Name 'discovery' -Action Discovery
    Assert-True -Condition ([string]$discovery.schema -ceq 'lattice.direct-stdio-client.v2') -Message 'summary schema v2'
    Assert-True -Condition ([bool]$discovery.success) -Message 'seven-tool discovery succeeds'
    Assert-True -Condition ([string]$discovery.classification -ceq 'DISCOVERY_OK') -Message 'discovery classification'
    Assert-True -Condition ([bool]$discovery.discovery.exact_seven) -Message 'exact seven marker'
    Assert-True -Condition ((@($discovery.discovery.tool_names | Sort-Object -CaseSensitive) -join "`n") -ceq ($script:ExpectedTools -join "`n")) -Message 'exact seven names'
    Assert-True -Condition ($null -eq $discovery.call) -Message 'discovery has no call'

    $canary = Invoke-WrapperCase -Name 'canary' -Action TaskSubmit -UseClientRequestId
    $canaryArguments = $canary.call.result.structuredContent.received_arguments
    Assert-ExactFields -Object $canaryArguments -Expected @('client_request_id', 'intent') -Message 'canary argument fields'
    Assert-True -Condition ([string]$canaryArguments.intent -ceq 'CONTROLLED_CODEX_CANARY') -Message 'canary intent preserved'

    $generalName = Invoke-WrapperCase -Name 'general-name' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectName 'AI 劇本' -UseProjectName
    $nameArguments = $generalName.call.result.structuredContent.received_arguments
    Assert-ExactFields -Object $nameArguments -Expected @('client_request_id', 'objective', 'project_name') -Message 'project-name argument fields'
    Assert-True -Condition ([string]$nameArguments.objective -ceq '完成角色系統') -Message 'objective preserved'
    Assert-True -Condition ([string]$nameArguments.project_name -ceq 'AI 劇本') -Message 'project name preserved'

    $projectId = 'legacy-project-id'
    $generalId = Invoke-WrapperCase -Name 'general-id' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectId $projectId -UseProjectId
    $idArguments = $generalId.call.result.structuredContent.received_arguments
    Assert-ExactFields -Object $idArguments -Expected @('client_request_id', 'objective', 'project_id') -Message 'project-id argument fields'
    Assert-True -Condition ([string]$idArguments.project_id -ceq $projectId) -Message 'project id preserved'

    $benignSkText = 'finish mask-based validation'
    $benignSk = Invoke-WrapperCase -Name 'benign-sk-boundary' -Action TaskSubmit `
        -Objective $benignSkText -UseObjective -ProjectId $projectId -UseProjectId
    Assert-True -Condition ([bool]$benignSk.success) -Message 'embedded sk text accepted'
    Assert-True -Condition ([string]$benignSk.call.result.structuredContent.received_arguments.objective -ceq $benignSkText) `
        -Message 'embedded sk text preserved'

    $invalidProjectId = Invoke-WrapperCase -Name 'invalid-project-id' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectId 'INVALID' -UseProjectId
    Assert-True -Condition (-not [bool]$invalidProjectId.success) -Message 'invalid project id rejected'
    Assert-True -Condition (-not [bool]$invalidProjectId.process.started) -Message 'invalid project id rejected before process'
    Assert-True -Condition ([string]$invalidProjectId.failure_message -ceq 'PROJECT_ID_REJECTED') -Message 'invalid project id failure code'

    $secretProjectId = 'sk-do-not-use'
    $secretProject = Invoke-WrapperCase -Name 'secret-project-id' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectId $secretProjectId -UseProjectId
    Assert-True -Condition (-not [bool]$secretProject.success) -Message 'secret-shaped project id rejected'
    Assert-True -Condition (-not [bool]$secretProject.process.started) -Message 'secret-shaped project id rejected before process'
    Assert-True -Condition ([string]$secretProject.failure_message -ceq 'PROJECT_ID_REJECTED') `
        -Message 'secret-shaped project id uses safe failure code'
    $secretProjectSummary = [IO.File]::ReadAllText([string]$secretProject.artifacts.summary)
    Assert-True -Condition (-not $secretProjectSummary.Contains($secretProjectId)) `
        -Message 'secret-shaped project id absent from summary'

    $generalUnique = Invoke-WrapperCase -Name 'general-unique' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective
    Assert-ExactFields -Object $generalUnique.call.result.structuredContent.received_arguments `
        -Expected @('client_request_id', 'objective') -Message 'selector-free fields'

    $emoji = [char]::ConvertFromUtf32(0x1f600)
    $objective512 = $emoji * 512
    $objectiveBoundary = Invoke-WrapperCase -Name 'objective-512-scalars' -Action TaskSubmit `
        -Objective $objective512 -UseObjective
    Assert-True -Condition ([bool]$objectiveBoundary.success) -Message '512-scalar objective accepted'
    Assert-True -Condition ([string]$objectiveBoundary.call.result.structuredContent.received_arguments.objective -ceq $objective512) `
        -Message '512-scalar objective accepted intact'

    $objective513 = Invoke-WrapperCase -Name 'objective-513-scalars' -Action TaskSubmit `
        -Objective ($emoji * 513) -UseObjective
    Assert-True -Condition (-not [bool]$objective513.success) -Message '513-scalar objective rejected'
    Assert-True -Condition (-not [bool]$objective513.process.started) -Message '513-scalar rejection before process'

    $project64 = $emoji * 64
    $projectBoundary = Invoke-WrapperCase -Name 'project-64-scalars' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectName $project64 -UseProjectName
    Assert-True -Condition ([bool]$projectBoundary.success) -Message '64-scalar project name accepted'
    Assert-True -Condition ([string]$projectBoundary.call.result.structuredContent.received_arguments.project_name -ceq $project64) `
        -Message '64-scalar project name accepted intact'

    $project65 = Invoke-WrapperCase -Name 'project-65-scalars' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectName ($emoji * 65) -UseProjectName
    Assert-True -Condition (-not [bool]$project65.success) -Message '65-scalar project name rejected'
    Assert-True -Condition (-not [bool]$project65.process.started) -Message '65-scalar rejection before process'

    $unpairedSurrogate = [string][char]0xd800
    $unpairedObjective = Invoke-WrapperCase -Name 'unpaired-surrogate' -Action TaskSubmit `
        -Objective $unpairedSurrogate -UseObjective
    Assert-True -Condition (-not [bool]$unpairedObjective.success) -Message 'unpaired surrogate rejected'
    Assert-True -Condition (-not [bool]$unpairedObjective.process.started) -Message 'unpaired surrogate rejection before process'

    $taskRef = 'a' * 64
    $status = Invoke-WrapperCase -Name 'status' -Action TaskStatus -TaskRef $taskRef
    $statusArguments = $status.call.result.structuredContent.received_arguments
    Assert-ExactFields -Object $statusArguments -Expected @('task_ref') -Message 'status is task-ref only'
    Assert-True -Condition ([string]$statusArguments.task_ref -ceq $taskRef) -Message 'status task ref preserved'

    $canaryStatus = Invoke-WrapperCase -Name 'canary-status' -Action TaskStatus -TaskRef $taskRef `
        -ClientRequestId 'offline-canary' -UseClientRequestId
    $canaryStatusArguments = $canaryStatus.call.result.structuredContent.received_arguments
    Assert-ExactFields -Object $canaryStatusArguments -Expected @('task_ref', 'client_request_id') `
        -Message 'explicit client id retained for canary status'
    Assert-True -Condition ([string]$canaryStatusArguments.client_request_id -ceq 'offline-canary') `
        -Message 'canary status client id preserved'

    $selectorConflict = Invoke-WrapperCase -Name 'selector-conflict' -Action TaskSubmit `
        -Objective '完成角色系統' -UseObjective -ProjectId $projectId -UseProjectId `
        -ProjectName 'AI 劇本' -UseProjectName
    Assert-True -Condition (-not [bool]$selectorConflict.success) -Message 'selector conflict rejected'
    Assert-True -Condition (-not [bool]$selectorConflict.process.started) -Message 'selector conflict before process'

    $canarySelector = Invoke-WrapperCase -Name 'canary-selector' -Action TaskSubmit `
        -ProjectName 'AI 劇本' -UseProjectName
    Assert-True -Condition (-not [bool]$canarySelector.success) -Message 'canary selector rejected'
    Assert-True -Condition (-not [bool]$canarySelector.process.started) -Message 'canary selector before process'

    $blankObjective = Invoke-WrapperCase -Name 'blank-objective' -Action TaskSubmit `
        -Objective ' padded ' -UseObjective
    Assert-True -Condition (-not [bool]$blankObjective.success) -Message 'noncanonical objective rejected'
    Assert-True -Condition (-not [bool]$blankObjective.process.started) -Message 'objective rejection before process'

    $secretText = 'password=super-secret-value'
    $secretObjective = Invoke-WrapperCase -Name 'secret-objective' -Action TaskSubmit `
        -Objective $secretText -UseObjective
    Assert-True -Condition (-not [bool]$secretObjective.success) -Message 'secret objective rejected'
    Assert-True -Condition (-not [bool]$secretObjective.process.started) -Message 'secret rejection before process'
    Assert-True -Condition ([string]$secretObjective.failure_message -ceq 'TASK_OBJECTIVE_REJECTED') -Message 'secret-safe failure code'
    $secretSummary = [IO.File]::ReadAllText([string]$secretObjective.artifacts.summary)
    Assert-True -Condition (-not $secretSummary.Contains($secretText)) -Message 'secret absent from summary'

    $secretClientRequestId = 'sk-do-not-use'
    $secretClientSubmit = Invoke-WrapperCase -Name 'secret-client-id-submit' -Action TaskSubmit `
        -ClientRequestId $secretClientRequestId -UseClientRequestId `
        -Objective '完成角色系統' -UseObjective
    Assert-True -Condition (-not [bool]$secretClientSubmit.success) -Message 'secret client id rejected for submit'
    Assert-True -Condition (-not [bool]$secretClientSubmit.process.started) -Message 'secret client id rejected before submit process'
    Assert-True -Condition ([string]$secretClientSubmit.failure_message -ceq 'CLIENT_REQUEST_ID_REJECTED') `
        -Message 'secret client id uses a safe submit failure code'
    $secretClientSubmitSummary = [IO.File]::ReadAllText([string]$secretClientSubmit.artifacts.summary)
    Assert-True -Condition (-not $secretClientSubmitSummary.Contains($secretClientRequestId)) `
        -Message 'secret client id absent from submit summary'

    $assignmentClientRequestId = 'password=do-not-use'
    $assignmentClientSubmit = Invoke-WrapperCase -Name 'assignment-client-id-submit' -Action TaskSubmit `
        -ClientRequestId $assignmentClientRequestId -UseClientRequestId `
        -Objective '完成角色系統' -UseObjective
    Assert-True -Condition (-not [bool]$assignmentClientSubmit.success) `
        -Message 'assignment-shaped client id rejected for submit'
    Assert-True -Condition (-not [bool]$assignmentClientSubmit.process.started) `
        -Message 'assignment-shaped client id rejected before parameter echo or process'
    Assert-True -Condition ([string]$assignmentClientSubmit.failure_message -ceq 'CLIENT_REQUEST_ID_REJECTED') `
        -Message 'assignment-shaped client id uses a safe failure code'
    $assignmentClientSubmitSummary = [IO.File]::ReadAllText([string]$assignmentClientSubmit.artifacts.summary)
    Assert-True -Condition (-not $assignmentClientSubmitSummary.Contains($assignmentClientRequestId)) `
        -Message 'assignment-shaped client id absent from submit summary'

    $embeddedSecretClientRequestId = 'xghp_do-not-use'
    $embeddedSecretClientSubmit = Invoke-WrapperCase -Name 'embedded-secret-client-id-submit' -Action TaskSubmit `
        -ClientRequestId $embeddedSecretClientRequestId -UseClientRequestId `
        -Objective '完成角色系統' -UseObjective
    Assert-True -Condition (-not [bool]$embeddedSecretClientSubmit.success) `
        -Message 'embedded secret client id rejected for submit'
    Assert-True -Condition (-not [bool]$embeddedSecretClientSubmit.process.started) `
        -Message 'embedded secret client id rejected before submit process'
    Assert-True -Condition ([string]$embeddedSecretClientSubmit.failure_message -ceq 'CLIENT_REQUEST_ID_REJECTED') `
        -Message 'embedded secret client id uses a safe submit failure code'
    $embeddedSecretClientSubmitSummary = [IO.File]::ReadAllText([string]$embeddedSecretClientSubmit.artifacts.summary)
    Assert-True -Condition (-not $embeddedSecretClientSubmitSummary.Contains($embeddedSecretClientRequestId)) `
        -Message 'embedded secret client id absent from submit summary'

    $secretClientStatus = Invoke-WrapperCase -Name 'secret-client-id-status' -Action TaskStatus `
        -TaskRef $taskRef -ClientRequestId $secretClientRequestId -UseClientRequestId
    Assert-True -Condition (-not [bool]$secretClientStatus.success) -Message 'secret client id rejected for status'
    Assert-True -Condition (-not [bool]$secretClientStatus.process.started) -Message 'secret client id rejected before status process'
    Assert-True -Condition ([string]$secretClientStatus.failure_message -ceq 'CLIENT_REQUEST_ID_REJECTED') `
        -Message 'secret client id uses a safe status failure code'
    $secretClientStatusSummary = [IO.File]::ReadAllText([string]$secretClientStatus.artifacts.summary)
    Assert-True -Condition (-not $secretClientStatusSummary.Contains($secretClientRequestId)) `
        -Message 'secret client id absent from status summary'

    $embeddedSecretClientStatus = Invoke-WrapperCase -Name 'embedded-secret-client-id-status' -Action TaskStatus `
        -TaskRef $taskRef -ClientRequestId $embeddedSecretClientRequestId -UseClientRequestId
    Assert-True -Condition (-not [bool]$embeddedSecretClientStatus.success) `
        -Message 'embedded secret client id rejected for status'
    Assert-True -Condition (-not [bool]$embeddedSecretClientStatus.process.started) `
        -Message 'embedded secret client id rejected before status process'
    Assert-True -Condition ([string]$embeddedSecretClientStatus.failure_message -ceq 'CLIENT_REQUEST_ID_REJECTED') `
        -Message 'embedded secret client id uses a safe status failure code'
    $embeddedSecretClientStatusSummary = [IO.File]::ReadAllText([string]$embeddedSecretClientStatus.artifacts.summary)
    Assert-True -Condition (-not $embeddedSecretClientStatusSummary.Contains($embeddedSecretClientRequestId)) `
        -Message 'embedded secret client id absent from status summary'

    $secretTaskRef = 'password=hunter2'
    $secretTaskStatus = Invoke-WrapperCase -Name 'secret-task-ref-status' -Action TaskStatus `
        -TaskRef $secretTaskRef
    Assert-True -Condition (-not [bool]$secretTaskStatus.success) -Message 'secret task ref rejected'
    Assert-True -Condition (-not [bool]$secretTaskStatus.process.started) `
        -Message 'secret task ref rejected before process'
    Assert-True -Condition ([string]$secretTaskStatus.failure_message -ceq 'TASK_REF_REJECTED') `
        -Message 'secret task ref uses a safe status failure code'
    $secretTaskStatusSummary = [IO.File]::ReadAllText([string]$secretTaskStatus.artifacts.summary)
    Assert-True -Condition (-not $secretTaskStatusSummary.Contains($secretTaskRef)) `
        -Message 'secret task ref absent from status summary'

    $expandedSecretIndex = 0
    foreach ($sensitiveObjective in @(
        '完成設定 secret=hunter2',
        'credential: do-not-store',
        'Cookie = session-value',
        'refresh_token=do-not-store',
        '{"password":"hunter2"}',
        '{"api_key":"do-not-store"}',
        ("password{0}=hunter2" -f [char]0x2003),
        ("api_key{0}:do-not-store" -f [char]0x00a0),
        (([char]0x212a) + 'password=do-not-store'),
        (([char]0x212a) + 'sk-do-not-store'),
        'private key----- marker before -----begin marker',
        '使用 AKIAIOSFODNN7EXAMPLE 完成設定'
    )) {
        $rejected = Invoke-WrapperCase -Name ("expanded-secret-objective-{0}" -f $expandedSecretIndex) -Action TaskSubmit `
            -Objective $sensitiveObjective -UseObjective
        Assert-True -Condition (-not [bool]$rejected.success) -Message 'expanded secret objective rejected'
        Assert-True -Condition (-not [bool]$rejected.process.started) -Message 'expanded secret rejection before process'
        Assert-True -Condition ([string]$rejected.failure_message -ceq 'TASK_OBJECTIVE_REJECTED') -Message 'expanded secret-safe failure code'
        $rejectedSummary = [IO.File]::ReadAllText([string]$rejected.artifacts.summary)
        Assert-True -Condition (-not $rejectedSummary.Contains($sensitiveObjective)) -Message 'expanded secret absent from summary'
        $expandedSecretIndex++
    }

    $mismatch = Invoke-WrapperCase -Name 'catalog-mismatch' -Action Discovery -UseMismatchedCatalog
    Assert-True -Condition (-not [bool]$mismatch.success) -Message 'catalog mismatch rejected'
    Assert-True -Condition ([string]$mismatch.classification -ceq 'TOOL_SET_MISMATCH') -Message 'catalog mismatch classification'
    Assert-True -Condition (-not [bool]$mismatch.discovery.exact_seven) -Message 'catalog mismatch marker'

    [ordered]@{
        result = 'PASS'
        cases = 33
        fixture_processes = 11
        tool_calls = 9
        rejected_before_process = 22
        live_service_calls = 0
    } | ConvertTo-Json
}
finally {
    if (Test-Path -LiteralPath $testRoot) {
        [IO.Directory]::Delete($testRoot, $true)
    }
}

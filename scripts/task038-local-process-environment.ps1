Set-StrictMode -Version Latest

function Test-Task038SensitiveChildEnvironmentName {
    param([Parameter(Mandatory = $true)][string]$Name)

    return (
        $Name -like 'LATTICE_*' -or
        $Name -match '^(?i:HERMES_|OPENCLAW_)' -or
        $Name -match '(?i)(^|_)(API_?KEY|TOKEN|SECRET|PASSWORD|CREDENTIALS?|CONNECTION_STRING|DSN)($|_)' -or
        $Name -match '^(?i:AWS_|AZURE_|GOOGLE_|GITHUB_|GH_)' -or
        $Name -in @(
            'PGPASSWORD', 'PGPASSFILE', 'DATABASE_URL', 'CODEX_HOME',
            'GIT_ASKPASS', 'SSH_ASKPASS', 'SSH_AUTH_SOCK', 'RUST_LOG'
        )
    )
}

function Set-Task038ClosedChildEnvironment {
    param(
        [Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$EnvironmentValues
    )

    foreach ($name in @($StartInfo.EnvironmentVariables.Keys)) {
        if (Test-Task038SensitiveChildEnvironmentName -Name ([string]$name)) {
            $StartInfo.EnvironmentVariables.Remove([string]$name)
        }
    }
    foreach ($entry in $EnvironmentValues.GetEnumerator()) {
        if ([string]$entry.Key -notmatch '^[A-Z][A-Z0-9_]{0,127}$' -or $null -eq $entry.Value) {
            throw 'TASK038_CHILD_ENVIRONMENT_VALUE_REJECTED'
        }
        $StartInfo.EnvironmentVariables[[string]$entry.Key] = [string]$entry.Value
    }
    $StartInfo.EnvironmentVariables['NO_COLOR'] = '1'
}

# Windows installer verification

Run the complete, already-built EXE on a Windows x64 host with WSL2 ready:

```powershell
powershell.exe -NoProfile -NonInteractive -File scripts/verify-lattice-windows-installer.ps1 `
  -InstallerPath C:\candidate\LATTICE-Setup.exe `
  -ExpectedSha256 <the-verified-64-character-sha256> `
  -TrialRoot C:\acceptance\fresh-trial `
  -Output C:\acceptance\result.json
```

The trial directory and output file must not exist. Existing files, junctions,
and redirected paths are rejected. Output must be outside the trial directory.
The script never downloads an EXE or chooses a release/hash for the caller.

The host gate only reads Windows, virtualization, pending reboot, and WSL
status. A blocked host is reported without enabling features, requesting UAC,
restarting Windows, or changing security settings. Installation uses fresh
Codex/user/roaming/local/temp directories, a system-only PATH, and an OS
environment whitelist without runner tokens or provider keys. No AI turn runs.

Exit 0 requires a new, completed SFX receipt with whole-CMD child exit 0,
three-core installation, persisted Graphify data matching MCP readback,
runtime/PostgreSQL process restart evidence, the global hook, applied Codex
preferences, portable profile, and all required environment commands passing.
After those reports pass, a separate helper opens one owned MCP connection
using the installed Python and sealed launcher. Windows process module
observations must show `latticed.exe` loading its pinned app-local
`vcruntime140.dll` and PostgreSQL loading its pinned app-local
`vcruntime140.dll` and `msvcp140.dll`. Disk presence alone cannot pass this check.
The helper closes its own MCP connection and leaves PostgreSQL running.
The SFX process exit alone cannot pass acceptance. Other outcomes exit 2 with
the observed phase/code. The 50-minute process timeout preserves partial work;
it does not terminate an installer, PostgreSQL, or WSL distribution.

Upload only the selected `-Output` JSON: it contains allowlisted results and
report hashes. Do not upload the trial tree, raw process logs, Codex home,
runtime configuration, databases, or authentication material. Original local
reports and partial installations remain available for bounded diagnosis.

On a GitHub-hosted Windows Server 2025 runner the scope explicitly identifies
that independent VM. Local runs remain local isolated-install evidence.
Neither scope proves ordinary Windows 10/11 first-boot, UAC/reboot handling,
Codex Desktop login, or model availability. Standard public `windows-2025`
runners are suitable only after the live host gate succeeds; no paid runner or
security-setting change is required by this script.

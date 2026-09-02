import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

async function repositoryFile(relativePath) {
  return readFile(path.join(repositoryRoot, relativePath), "utf8");
}

test("the desktop shell is a dedicated WebView2 window rather than an external browser", async () => {
  const [project, windowMarkup, windowCode, desktopPolicy, packageJson] = await Promise.all([
    repositoryFile("apps/lattice-control-desktop/Lattice.Control.Desktop.csproj"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml.cs"),
    repositoryFile("apps/lattice-control-desktop/DesktopPolicy.cs"),
    repositoryFile("package.json"),
  ]);

  assert.match(project, /<TargetFramework>net8\.0-windows<\/TargetFramework>/u);
  assert.match(project, /<UseWPF>true<\/UseWPF>/u);
  assert.match(project, /Microsoft\.Web\.WebView2" Version="1\.0\.4191\.47"/u);
  assert.match(windowMarkup, /WindowStyle="None"/u);
  assert.match(windowMarkup, /<wv2:WebView2/u);
  assert.match(desktopPolicy, /http:\/\/127\.0\.0\.1/u);
  assert.match(windowCode, /AreDevToolsEnabled = false/u);
  assert.match(windowCode, /AreDefaultContextMenusEnabled = false/u);
  assert.match(windowCode, /NewWindowRequested/u);
  assert.match(windowCode, /AddWebResourceRequestedFilter/u);
  assert.match(windowCode, /Core_WebResourceRequested/u);
  assert.match(windowCode, /CreateWebResourceResponse/u);
  assert.match(windowCode, /HashSet<ulong>/u);
  assert.match(windowCode, /e\.NavigationId/u);
  assert.match(windowCode, /_currentNavigationId = e\.NavigationId;[\s\S]{0,250}IsApprovedControlNavigation/u);
  assert.match(windowCode, /_navigationGeneration/u);
  assert.match(windowCode, /ControlView\.Source/u);
  assert.match(desktopPolicy, /target\.Port == controlUri\.Port/u);
  assert.doesNotMatch(windowCode, /msedge\.exe|Process\.Start\([^\n]*https?:/iu);
  assert.match(packageJson, /"desktop:build"/u);
});

test("the desktop shell keeps WebView2 data outside the candidate and reconnects without a preview lifetime", async () => {
  const [windowMarkup, windowCode, desktopPolicy, controlMarkup] = await Promise.all([
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml.cs"),
    repositoryFile("apps/lattice-control-desktop/DesktopPolicy.cs"),
    repositoryFile("apps/lattice-control/public/index.html"),
  ]);

  assert.match(windowMarkup, /AutomationProperties\.AutomationId="LatticeConnectionStatus"/u);
  assert.match(windowMarkup, /x:Name="ConnectionOverlay"[\s\S]*Visibility="Visible"/u);
  assert.match(windowCode, /CoreWebView2Environment\.CreateAsync/u);
  assert.match(windowCode, /DesktopPolicy\.WebViewUserDataFolder/u);
  assert.match(windowCode, /DispatcherTimer/u);
  assert.match(windowCode, /DesktopPolicy\.ReconnectInterval/u);
  assert.match(windowCode, /ShowConnectingState\(\)/u);
  assert.match(windowCode, /_reconnectTimer\.Start\(\)/u);
  assert.doesNotMatch(windowCode, /Environment\.Exit|Application\.Current\.Shutdown|900_000|FromMinutes\(15\)/u);
  assert.match(desktopPolicy, /http:\/\/127\.0\.0\.1:4317\//u);
  assert.match(desktopPolicy, /Environment\.SpecialFolder\.LocalApplicationData/u);
  assert.match(desktopPolicy, /"LATTICE",\s*"ControlDesktop",\s*"WebView2"/u);
  assert.match(desktopPolicy, /target\.Port == controlUri\.Port/u);
  assert.doesNotMatch(desktopPolicy, /AppContext\.BaseDirectory|Environment\.CurrentDirectory/u);
  assert.match(controlMarkup, /function renderEmptyCores\(\)[\s\S]*recentList\.replaceChildren\(element\("p","目前沒有工作。","empty"\)\)/u);
  assert.match(controlMarkup, /grid-template-rows:\s*auto auto auto minmax\(520px,1fr\) auto/u);
});

test("the resizable desktop can reach compact scrolling without desktop or mobile regressions", async () => {
  const [windowMarkup, windowCode, resizePolicy, controlMarkup] = await Promise.all([
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml.cs"),
    repositoryFile("apps/lattice-control-desktop/WindowResizeHitTestPolicy.cs"),
    repositoryFile("apps/lattice-control/public/index.html"),
  ]);
  const minimumWidth = Number(windowMarkup.match(/MinWidth="(\d+)"/u)?.[1]);
  const minimumHeight = Number(windowMarkup.match(/MinHeight="(\d+)"/u)?.[1]);
  const referenceShell = controlMarkup.match(
    /<style id="lattice-reference-shell">([\s\S]*?)<\/style>/u,
  )?.[1] ?? "";
  const compactStart = referenceShell.indexOf("@media (max-width:");
  const wideCss = referenceShell.slice(0, compactStart);
  const compactCss = referenceShell.slice(compactStart);
  const compactBreakpoint = Number(compactCss.match(/@media \(max-width:(\d+)px\)/u)?.[1]);

  assert.ok(
    minimumWidth <= compactBreakpoint,
    `desktop minimum ${minimumWidth}px must reach compact breakpoint ${compactBreakpoint}px`,
  );
  assert.equal(minimumWidth, 640);
  assert.equal(minimumHeight, 480);
  assert.ok(minimumWidth <= 720 && minimumHeight <= 520);
  assert.match(windowMarkup, /WindowStyle="None"[\s\S]*ResizeMode="CanResize"/u);
  assert.match(windowMarkup, /<WindowChrome CaptionHeight="42"[\s\S]*ResizeBorderThickness="7"/u);
  assert.match(windowMarkup, /MouseLeftButtonDown="TitleBar_MouseLeftButtonDown"/u);
  assert.match(windowMarkup, /Click="Minimize_Click"/u);
  assert.match(windowMarkup, /Click="MaximizeRestore_Click"/u);
  assert.match(windowCode, /TitleBar_MouseLeftButtonDown[\s\S]*e\.ClickCount == 2[\s\S]*ToggleMaximize\(\)[\s\S]*DragMove\(\)/u);
  assert.match(windowCode, /Minimize_Click[\s\S]{0,120}WindowState = WindowState\.Minimized/u);
  assert.match(windowCode, /MaximizeRestore_Click[\s\S]{0,120}ToggleMaximize\(\)/u);
  assert.match(windowCode, /ToggleMaximize\(\)[\s\S]{0,180}WindowState == WindowState\.Maximized \? WindowState\.Normal : WindowState\.Maximized/u);
  assert.match(windowCode, /SourceInitialized \+= MainWindow_SourceInitialized/u);
  assert.match(windowCode, /AddHook\(WindowProc\)/u);
  assert.match(windowCode, /RemoveHook\(WindowProc\)/u);
  assert.match(windowCode, /message != WindowResizeHitTestPolicy\.WmNcHitTest/u);
  assert.match(windowCode, /WindowState == WindowState\.Maximized/u);
  assert.match(resizePolicy, /enum WindowResizeHit[\s\S]*TopLeft = 13[\s\S]*BottomRight = 17/u);
  assert.match(resizePolicy, /EvaluatePhysical\(/u);
  assert.match(wideCss, /body \{ overflow:hidden;/u);
  assert.match(wideCss, /\.app \{ display:grid; grid-template-columns:248px minmax\(0,1fr\); width:100vw; height:100dvh;/u);
  assert.match(compactCss, /body \{ overflow-x:hidden; overflow-y:auto; \}/u);
  assert.match(compactCss, /\.app \{ display:block; min-height:100dvh; height:auto; \}/u);
  assert.match(compactCss, /\.workspace \{ display:grid; grid-template-rows:auto auto auto minmax\(520px,1fr\) auto; min-height:100dvh;/u);
  assert.match(compactCss, /\.graph-viewport \{ min-height:500px; overscroll-behavior-x:contain; overscroll-behavior-y:auto; \}/u);
  assert.match(compactCss, /\.transcript \{ padding:13px; overscroll-behavior-y:auto; \}/u);
  assert.match(compactCss, /\.command-dock \{ position:sticky; bottom:75px;/u);
  assert.match(compactCss, /\.graph-list,#graph-edge-layer \{ min-width:720px; min-height:500px; \}/u);
  assert.equal((controlMarkup.match(/data-core-target="/gu) ?? []).length, 4);
});

test("the desktop probes and owns only its compatible default Control before navigation", async () => {
  const [windowMarkup, windowCode, desktopPolicy, runtimeCode, project] = await Promise.all([
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml.cs"),
    repositoryFile("apps/lattice-control-desktop/DesktopPolicy.cs"),
    repositoryFile("apps/lattice-control-desktop/ControlRuntime.cs"),
    repositoryFile("apps/lattice-control-desktop/Lattice.Control.Desktop.csproj"),
  ]);

  assert.match(windowMarkup, /AutomationProperties\.AutomationId="LatticeRuntimeHealth"/u);
  assert.match(windowCode, /ControlRuntimeManager\.CreatePackaged/u);
  assert.match(windowCode, /EnsureReadyAsync[\s\S]*core\.Navigate\(_controlUri\.AbsoluteUri\)/u);
  assert.match(windowCode, /ProbeAsync/u);
  assert.match(windowCode, /_healthTimer/u);
  assert.match(windowCode, /HealthTimer_Tick[\s\S]*ProbeAsync/u);
  assert.match(windowCode, /Core_NavigationCompleted[\s\S]*_healthTimer\.Start\(\)/u);
  assert.ok((windowCode.match(/CanApplyNavigationResult/gu) ?? []).length >= 2);
  assert.match(windowCode, /ShowDiagnosedConnectionFailureAsync\(\s*e\.NavigationId/u);
  assert.doesNotMatch(windowCode, /ShowBlockedNavigation\(\)[\s\S]{0,500}SetRuntimeStatus/u);
  assert.match(windowCode, /ControlRuntimeHealth\.STOPPED/u);
  assert.match(windowCode, /ControlRuntimeHealth\.UNREACHABLE/u);
  assert.match(windowCode, /ControlRuntimeHealth\.INCOMPATIBLE/u);
  assert.match(windowCode, /CompleteShutdownAsync/u);
  assert.match(windowCode, /_controlRuntime\.ShutdownAsync\(CancellationToken\.None\)/u);
  const shutdownBody = windowCode.match(
    /private async Task CompleteShutdownAsync\(\)[\s\S]+?\n    \}\n\}/u,
  )?.[0] ?? "";
  assert.match(shutdownBody, /catch \(Exception error\)/u);
  assert.match(shutdownBody, /_isClosing = false/u);
  assert.match(windowMarkup, /x:Name="ReconnectButton"/u);
  assert.match(shutdownBody, /ReconnectButton\.Content = "重試安全關閉"/u);
  assert.match(windowCode, /Reconnect_Click[\s\S]*?_shutdownFailed[\s\S]*?CompleteShutdownAsync/u);
  assert.doesNotMatch(shutdownBody, /shutdown_failed[\s\S]*?ScheduleReconnect/u);
  assert.doesNotMatch(shutdownBody, /finally[\s\S]+?_shutdownComplete = true/u);
  assert.match(desktopPolicy, /ResolveControlTarget/u);
  assert.match(desktopPolicy, /ManageControl/u);
  assert.match(runtimeCode, /ControlRuntimeContract\.EvaluateProbe/u);
  assert.match(runtimeCode, /ControlRuntimeAction\.StartOwned/u);
  assert.match(runtimeCode, /Process\.Start/u);
  assert.match(runtimeCode, /RedirectStandardInput = true/u);
  assert.match(runtimeCode, /lattice\.control\.desktop-shutdown\.v1/u);
  assert.match(runtimeCode, /killOwnedProcess = killOwnedProcess[\s\S]*Kill\(entireProcessTree: true\)/u);
  assert.match(runtimeCode, /LastStopUsedHardKill = true[\s\S]*killOwnedProcess\(process\)/u);
  assert.doesNotMatch(runtimeCode, /GetProcesses|GetProcessById|netstat|Get-NetTCPConnection/u);
  assert.match(project, /LogicalName="Lattice\.Control\.RuntimeIdentity\.json"/u);
  assert.match(project, /LogicalName="Lattice\.Control\.DataScopeContract\.json"/u);
});

test("the Windows candidate is a repeatable self-contained per-user installable package", async () => {
  const [publishScript, installerScript, uninstallerScript, installerCommon, installerTest, acceptanceScript, managedControlScript, externalNavigationFixture, isolatedControlFixture, incompatibleControlFixture, policyTestIgnore, packageJson] = await Promise.all([
    repositoryFile("scripts/Publish-LatticeDesktopCandidate.ps1"),
    repositoryFile("scripts/Install-LATTICE.ps1"),
    repositoryFile("scripts/Uninstall-LATTICE.ps1"),
    repositoryFile("scripts/LatticeDesktopInstaller.Common.ps1"),
    repositoryFile("scripts/Test-LatticeDesktopInstaller.ps1"),
    repositoryFile("scripts/Test-LatticeDesktopCandidate.ps1"),
    repositoryFile("scripts/Test-LatticeDesktopManagedControl.ps1"),
    repositoryFile("apps/lattice-control/test/fixtures/desktop-external-redirect.mjs"),
    repositoryFile("apps/lattice-control/test/fixtures/desktop-isolated-control.mjs"),
    repositoryFile("apps/lattice-control/test/fixtures/desktop-incompatible-control.mjs"),
    repositoryFile("apps/lattice-control-desktop-policy-tests/.gitignore"),
    repositoryFile("package.json"),
  ]);

  assert.match(publishScript, /PORTABLE_RELEASE_CANDIDATE/u);
  assert.match(publishScript, /WINDOWS_PER_USER_INSTALLER/u);
  assert.match(publishScript, /--self-contained[\s\S]*true/u);
  assert.match(publishScript, /win-x64/u);
  assert.match(publishScript, /Compress-Archive/u);
  assert.match(publishScript, /LocalApplicationData/u);
  assert.match(publishScript, /HANDOFF\.md/u);
  assert.match(publishScript, /\$expectedProtectedDirtyState = ' M HANDOFF\.md'/u);
  assert.match(publishScript, /git -C \$repositoryRoot diff --cached --name-only/u);
  assert.match(publishScript, /files\s*=\s*\$artifactFiles/u);
  assert.match(publishScript, /lattice\.control\.desktop-portable-candidate\.v2/u);
  assert.match(publishScript, /lattice\.control\.desktop-per-user-installer\.v1/u);
  assert.match(publishScript, /control-runtime[\\/]node\.exe/iu);
  assert.match(publishScript, /apps[\\/]lattice-control[\\/]runtime-identity\.json/iu);
  assert.match(publishScript, /apps[\\/]lattice-control[\\/]data-scope-contract\.json/iu);
  assert.match(publishScript, /Join-Path \$controlSourceRoot 'src'/iu);
  assert.match(publishScript, /Join-Path \$controlSourceRoot 'public'/iu);
  assert.match(publishScript, /node_version/u);
  assert.match(publishScript, /node_sha256/u);
  assert.match(publishScript, /control_runtime/u);
  assert.match(publishScript, /Install-LATTICE\.ps1/u);
  assert.match(publishScript, /Uninstall-LATTICE\.ps1/u);
  assert.doesNotMatch(publishScript, /msi|wix|nsis/iu);
  assert.match(installerScript, /RollbackToCommit/u);
  assert.match(installerScript, /Invoke-LatticeDesktopInstall/u);
  assert.match(uninstallerScript, /Invoke-LatticeDesktopUninstall/u);
  assert.match(installerCommon, /lattice\.control\.desktop-install-owner\.v1/u);
  assert.match(installerCommon, /Get-LatticeInstallerBundle/u);
  assert.match(installerCommon, /\[IO\.Compression\.ZipFile\]::OpenRead/u);
  assert.match(installerCommon, /\[IO\.FileMode\]::CreateNew/u);
  assert.match(installerCommon, /\$input\.CopyTo\(\$output\)/u);
  assert.doesNotMatch(installerCommon, /Expand-Archive/u);
  assert.match(installerCommon, /LocalApplicationData[\s\S]*Programs[\\/]LATTICE/u);
  assert.match(installerCommon, /versions/u);
  assert.match(installerCommon, /\.staging/u);
  assert.match(installerCommon, /Start Menu/u);
  assert.match(installerCommon, /CurrentVersion[\\/]Uninstall[\\/]LATTICE/u);
  assert.match(installerCommon, /LATTICE_CONTROL_DESKTOP/u);
  assert.match(installerTest, /failure_preserved_previous_activation/u);
  assert.match(installerTest, /control_data_preserved/u);
  assert.match(installerTest, /webview_data_preserved/u);
  assert.match(acceptanceScript, /MinimumLifetimeSeconds/u);
  assert.match(acceptanceScript, /\$monitorStartedAt\s*=\s*\[DateTimeOffset\]::UtcNow/u);
  assert.match(acceptanceScript, /\$monitorStartedAt\.AddSeconds\(\$MinimumLifetimeSeconds\)/u);
  assert.match(acceptanceScript, /DESKTOP_CANDIDATE_EXTERNAL_CAPTURE_OBSERVED/u);
  assert.match(acceptanceScript, /Remove-TemporaryRootWithRetry/u);
  assert.match(acceptanceScript, /DESKTOP_CANDIDATE_CLEANUP_FAILED_AFTER_PRIMARY/u);
  assert.match(acceptanceScript, /Write-Warning[\s\S]{0,200}-WarningAction Continue/u);
  assert.match(acceptanceScript, /if \(-not \$OwnedProcess\.WaitForExit\(10000\)\)/u);
  assert.match(acceptanceScript, /DESKTOP_CANDIDATE_OWNED_PROCESS_STOP_TIMEOUT/u);
  assert.match(acceptanceScript, /LatticeConnectionStatus/u);
  assert.match(acceptanceScript, /candidate-manifest\.json/u);
  assert.match(acceptanceScript, /schema_version/u);
  assert.match(acceptanceScript, /source_commit/u);
  assert.match(acceptanceScript, /artifact_type/u);
  assert.match(acceptanceScript, /runtime_identifier/u);
  assert.match(acceptanceScript, /self_contained/u);
  assert.match(acceptanceScript, /executable_sha256/u);
  assert.match(publishScript, /function Get-Sha256Hex/u);
  assert.match(acceptanceScript, /function Get-Sha256Hex/u);
  assert.doesNotMatch(publishScript, /Get-FileHash/u);
  assert.doesNotMatch(acceptanceScript, /Get-FileHash/u);
  assert.match(acceptanceScript, /Expand-Archive/u);
  assert.match(acceptanceScript, /desktop-webview2/u);
  assert.match(acceptanceScript, /WEBVIEW2_USER_DATA_FOLDER/u);
  assert.match(acceptanceScript, /desktop-external-redirect\.mjs/u);
  assert.match(acceptanceScript, /-UseBasicParsing/u);
  assert.match(acceptanceScript, /EnvironmentVariables\['LOCALAPPDATA'\]/u);
  assert.match(acceptanceScript, /external_navigation_blocked/u);
  assert.match(managedControlScript, /no_listener_started_owned_control/u);
  assert.match(managedControlScript, /interruption_observed_status/u);
  assert.match(managedControlScript, /compatible_control_reused/u);
  assert.match(managedControlScript, /incompatible_status/u);
  assert.match(managedControlScript, /Get-CimInstance Win32_Process -Filter "ParentProcessId/u);
  assert.doesNotMatch(managedControlScript, /Get-NetTCPConnection|Stop-Process\s+-Name|taskkill|\.ArgumentList|\.Kill\(\$true\)/iu);
  assert.match(externalNavigationFixture, /writeHead\(302/u);
  assert.match(externalNavigationFixture, /\/outside/u);
  assert.match(externalNavigationFixture, /LATTICE_DESKTOP_REDIRECT_MARKER/u);
  assert.match(externalNavigationFixture, /LATTICE_DESKTOP_CAPTURE_MARKER/u);
  assert.match(externalNavigationFixture, /listen\(0, "127\.0\.0\.1"/u);
  assert.match(isolatedControlFixture, /createLatticeServer/u);
  assert.match(isolatedControlFixture, /listen\(0, "127\.0\.0\.1"/u);
  assert.match(incompatibleControlFixture, /0\.0\.0-foreign/u);
  assert.match(incompatibleControlFixture, /id: "postgresql"[\s\S]*status: "NOT_IMPLEMENTED"/u);
  assert.match(policyTestIgnore, /^bin\/$/mu);
  assert.match(policyTestIgnore, /^obj\/$/mu);
  assert.match(packageJson, /"desktop:policy-test"/u);
  assert.match(packageJson, /"desktop:installer-test"/u);
  assert.match(packageJson, /"desktop:publish"/u);
  assert.match(packageJson, /"desktop:candidate-test"/u);
  assert.match(packageJson, /"desktop:managed-control-test"/u);
});

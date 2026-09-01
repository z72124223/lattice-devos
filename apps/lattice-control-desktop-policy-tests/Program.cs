using Lattice.Control.Desktop;

static void Require(bool condition, string message)
{
    if (!condition) throw new InvalidOperationException(message);
}

ControlEndpointSelection defaultSelection = DesktopPolicy.ResolveControlTarget(["LATTICE.exe"], null);
Uri defaultUri = defaultSelection.Uri;
Require(defaultUri == new Uri("http://127.0.0.1:4317/"), "default Control origin changed");
Require(defaultSelection.ManageControl, "implicit default 4317 Control was not managed");

foreach (string rejected in new[]
{
    "https://example.com/",
    "http://localhost:4317/",
    "http://127.0.0.1/",
    "http://user@127.0.0.1:4317/",
    "https://127.0.0.1:4317/",
})
{
    ControlEndpointSelection resolved = DesktopPolicy.ResolveControlTarget(
        ["LATTICE.exe", "--url", rejected],
        null);
    Require(resolved.Uri == DesktopPolicy.DefaultControlUri,
        $"unapproved Control origin accepted: {rejected}");
    Require(resolved.ManageControl,
        $"invalid explicit origin bypassed management after default fallback: {rejected}");
}

ControlEndpointSelection alternateSelection = DesktopPolicy.ResolveControlTarget(
    ["LATTICE.exe", "--url", "http://127.0.0.1:54321/"],
    null);
Uri alternateLoopback = alternateSelection.Uri;
Require(alternateLoopback.Port == 54321, "explicit loopback test origin was rejected");
Require(!alternateSelection.ManageControl, "explicit alternate Control was incorrectly managed");
foreach (string selected4317 in new[]
{
    "http://127.0.0.1:4317/",
    "http://127.0.0.1:4317/profile",
    "http://127.0.0.1:4317/?profile=external",
})
{
    ControlEndpointSelection explicitSelection = DesktopPolicy.ResolveControlTarget(
        ["LATTICE.exe", "--url", selected4317],
        null);
    Require(!explicitSelection.ManageControl,
        $"an explicit --url was mistaken for the implicit managed 4317 target: {selected4317}");
}
ControlEndpointSelection profileSelection = DesktopPolicy.ResolveControlTarget(
    ["LATTICE.exe"],
    "http://127.0.0.1:4317/profile");
Require(!profileSelection.ManageControl,
    "an explicit environment profile was mistaken for the implicit managed 4317 target");
Require(
    DesktopPolicy.IsApprovedControlNavigation(new Uri("http://127.0.0.1:4317/api/four-core"), defaultUri),
    "same-origin Control navigation was rejected");
Require(
    !DesktopPolicy.IsApprovedControlNavigation(new Uri("http://127.0.0.1:4318/"), defaultUri),
    "different loopback port was accepted");
Require(
    !DesktopPolicy.IsApprovedControlNavigation(new Uri("https://example.com/"), defaultUri),
    "external navigation was accepted");
long navigationGeneration = DesktopPolicy.NextNavigationGeneration(0);
long capturedGeneration = navigationGeneration;
Require(
    DesktopPolicy.CanApplyNavigationResult(42, 42, navigationGeneration, capturedGeneration, isClosing: false),
    "current navigation result was rejected");
navigationGeneration = DesktopPolicy.NextNavigationGeneration(navigationGeneration);
Require(
    !DesktopPolicy.CanApplyNavigationResult(43, 42, navigationGeneration, capturedGeneration, isClosing: false),
    "N completion was accepted after blocked N+1 became current");
Require(
    !DesktopPolicy.CanApplyNavigationResult(43, 43, navigationGeneration, capturedGeneration, isClosing: false),
    "older async generation was accepted for the current navigation id");
Require(
    !DesktopPolicy.CanApplyNavigationResult(43, 43, navigationGeneration, navigationGeneration, isClosing: true),
    "closing window accepted a late navigation result");
Require(
    !DesktopPolicy.CanApplyNavigationResult(null, 42, navigationGeneration, navigationGeneration, isClosing: false),
    "missing current navigation accepted a result");

string localApplicationData = Path.GetFullPath(
    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData));
string userDataFolder = Path.GetFullPath(DesktopPolicy.WebViewUserDataFolder);
Require(
    userDataFolder.StartsWith(localApplicationData + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase),
    "WebView2 user data is not under LocalApplicationData");
Require(
    userDataFolder.EndsWith(
        Path.Combine("LATTICE", "ControlDesktop", "WebView2"),
        StringComparison.OrdinalIgnoreCase),
    "WebView2 user data folder identity changed");
Require(
    DesktopPolicy.ReconnectInterval >= TimeSpan.FromSeconds(1)
        && DesktopPolicy.ReconnectInterval <= TimeSpan.FromSeconds(10),
    "desktop reconnect interval is not bounded");

ControlRuntimeFailurePresentation missingFiles = DesktopPolicy.DescribeRuntimeFailure(new(
    ControlRuntimeHealth.STOPPED,
    ControlRuntimeAction.FailClosed,
    "CONTROL_RUNTIME_FILES_MISSING"));
Require(!missingFiles.AutoReconnect, "missing runtime files were advertised as automatically recoverable");
Require(missingFiles.Detail.Contains("無法自動修復", StringComparison.Ordinal),
    "missing runtime files did not explain the permanent failure");
ControlRuntimeFailurePresentation startFailed = DesktopPolicy.DescribeRuntimeFailure(new(
    ControlRuntimeHealth.STOPPED,
    ControlRuntimeAction.FailClosed,
    "CONTROL_PROCESS_START_FAILED"));
Require(!startFailed.AutoReconnect, "process start failure was advertised as automatically recoverable");
Require(startFailed.Detail.Contains("停止自動重試", StringComparison.Ordinal),
    "process start failure did not explain that retry stopped");
ControlRuntimeFailurePresentation interruptedControl = DesktopPolicy.DescribeRuntimeFailure(new(
    ControlRuntimeHealth.STOPPED,
    ControlRuntimeAction.FailClosed,
    "CONTROL_OWNED_PROCESS_EXITED"));
Require(interruptedControl.AutoReconnect, "owned process interruption was not recoverable");
Require(interruptedControl.Detail.Contains("重連", StringComparison.Ordinal),
    "recoverable process interruption did not explain reconnect behavior");

ControlRuntimeIdentity expectedIdentity = ControlRuntimeContract.ExpectedIdentity;
Require(expectedIdentity.SchemaVersion == "lattice.control.runtime-identity.v1", "runtime identity schema changed");
Require(expectedIdentity.Product == "LATTICE_CONTROL", "runtime product identity changed");
Require(expectedIdentity.Version == "1.0.0-rc.2", "runtime compatibility version changed");

string expectedDatabasePath = Path.GetFullPath(Path.Combine(
    Path.GetTempPath(),
    "lattice-policy-scope",
    "LATTICE",
    "control",
    "lattice-control.db"));
ControlRuntimeDataScope expectedScope = ControlRuntimeContract.DataScopeForDatabasePath(
    expectedDatabasePath);
Require(expectedScope.SchemaVersion == "lattice.control.data-scope.v1", "data scope schema changed");
Require(expectedScope.Store == "CONTROL_SQLITE", "data scope store changed");
Require(expectedScope.StoreSchemaVersion == 7, "data scope store schema changed");
Require(expectedScope.AuthorityClass == "CONTROL_LOCAL_PRODUCT_STATE", "data scope authority changed");
Require(expectedScope.RegistryAuthority == "NONE", "data scope registry authority changed");
Require(expectedScope.Digest.Length == 64, "data scope digest is not SHA-256");
ControlRuntimeDataScope unicodeScope = ControlRuntimeContract.DataScopeForDatabasePath(
    @"C:\LATTICE\資料\控制.db");
Require(
    unicodeScope.Digest == "6abca5698e2c85cf0e0de89a8bc7b1adfafb502b61999164c897da31346e4976",
    "non-ASCII data scope digest drifted from the shared Node/.NET fixture");

string compatibleSurface = """
{
  "schema_version":"lattice.control.runtime-surface.v2",
  "identity":{
    "schema_version":"lattice.control.runtime-identity.v1",
    "product":"LATTICE_CONTROL",
    "version":"1.0.0-rc.2"
  },
  "data_scope":{
    "schema_version":"lattice.control.data-scope.v1",
    "store":"CONTROL_SQLITE",
    "store_schema_version":7,
    "authority_class":"CONTROL_LOCAL_PRODUCT_STATE",
    "registry_authority":"NONE",
    "digest":"EXPECTED_SCOPE_DIGEST"
  },
  "reconciliation_required":false,
  "health":"HEALTHY",
  "capabilities":[
    {"id":"control_sqlite","label":"Control／SQLite","status":"HEALTHY","has_data":null},
    {"id":"codex_app_server","label":"Codex App Server","status":"STOPPED","has_data":null},
    {"id":"work_mcp","label":"Work MCP","status":"HEALTHY","has_data":false},
    {"id":"decision_mcp","label":"Decision MCP","status":"HEALTHY","has_data":false},
    {"id":"postgresql","label":"正式 PostgreSQL","status":"NOT_IMPLEMENTED","has_data":null}
  ]
}
""".Replace("EXPECTED_SCOPE_DIGEST", expectedScope.Digest, StringComparison.Ordinal);
ControlRuntimeEvaluation compatible = ControlRuntimeContract.EvaluateProbe(
    tcpReachable: true,
    statusCode: 200,
    responseBody: compatibleSurface,
    expectedScope: expectedScope);
Require(compatible.Health == ControlRuntimeHealth.HEALTHY, "compatible Control was not healthy");
Require(compatible.Action == ControlRuntimeAction.Reuse, "compatible Control was not reused");
ControlRuntimeEvaluation reconciliation = ControlRuntimeContract.EvaluateProbe(
    tcpReachable: true,
    statusCode: 200,
    responseBody: compatibleSurface.Replace(
        "\"reconciliation_required\":false",
        "\"reconciliation_required\":true",
        StringComparison.Ordinal),
    expectedScope: expectedScope);
Require(reconciliation.Health == ControlRuntimeHealth.HEALTHY,
    "reconciliation-required compatible Control was hidden from recovery UI");
Require(reconciliation.Action == ControlRuntimeAction.Reuse,
    "reconciliation-required compatible Control was not reused");
Require(reconciliation.Detail == "CONTROL_RECONCILIATION_REQUIRED",
    "reconciliation-required detail was not preserved");
ControlRuntimeEvaluation degradedMcp = ControlRuntimeContract.EvaluateProbe(
    tcpReachable: true,
    statusCode: 200,
    responseBody: compatibleSurface.Replace(
        "\"work_mcp\",\"label\":\"Work MCP\",\"status\":\"HEALTHY\"",
        "\"work_mcp\",\"label\":\"Work MCP\",\"status\":\"UNREACHABLE\"",
        StringComparison.Ordinal),
    expectedScope: expectedScope);
Require(degradedMcp.Health == ControlRuntimeHealth.HEALTHY,
    "one unreachable MCP incorrectly made the compatible Control listener unhealthy");
Require(degradedMcp.Action == ControlRuntimeAction.Reuse,
    "one unreachable MCP prevented compatible Control reuse");

ControlRuntimeEvaluation absent = ControlRuntimeContract.EvaluateProbe(false, null, null, expectedScope);
Require(absent.Health == ControlRuntimeHealth.UNREACHABLE, "absent Control was not unreachable");
Require(absent.Action == ControlRuntimeAction.StartOwned, "absent Control did not permit owned start");

ControlRuntimeEvaluation unknownListener = ControlRuntimeContract.EvaluateProbe(true, null, null, expectedScope);
Require(unknownListener.Health == ControlRuntimeHealth.INCOMPATIBLE, "unknown listener was not incompatible");
Require(unknownListener.Action == ControlRuntimeAction.FailClosed, "unknown listener did not fail closed");

foreach (string incompatibleSurface in new[]
{
    compatibleSurface.Replace("1.0.0-rc.2", "0.9.0", StringComparison.Ordinal),
    compatibleSurface.Replace("LATTICE_CONTROL", "FOREIGN_CONTROL", StringComparison.Ordinal),
    compatibleSurface.Replace("lattice.control.runtime-surface.v2", "lattice.control.runtime-surface.v1", StringComparison.Ordinal),
    compatibleSurface.Replace(expectedScope.Digest, new string('0', 64), StringComparison.Ordinal),
    compatibleSurface.Replace("\"work_mcp\",\"label\":\"Work MCP\",\"status\":\"HEALTHY\"", "\"work_mcp\",\"label\":\"Work MCP\",\"status\":\"NO_DATA\"", StringComparison.Ordinal),
    compatibleSurface.Replace("\"has_data\":false", "\"has_data\":null", StringComparison.Ordinal),
    compatibleSurface.Replace("\"HEALTHY\"", "\"UNKNOWN\"", StringComparison.Ordinal),
    "not-json",
})
{
    ControlRuntimeEvaluation rejected = ControlRuntimeContract.EvaluateProbe(
        true,
        200,
        incompatibleSurface,
        expectedScope);
    Require(rejected.Health == ControlRuntimeHealth.INCOMPATIBLE, "incompatible Control surface was accepted");
    Require(rejected.Action == ControlRuntimeAction.FailClosed, "incompatible Control did not fail closed");
}

string[] healthStates = Enum.GetNames<ControlRuntimeHealth>();
Require(
    healthStates.SequenceEqual(new[]
    {
        "HEALTHY",
        "NOT_IMPLEMENTED",
        "STOPPED",
        "UNREACHABLE",
        "INCOMPATIBLE",
        "NO_DATA",
    }),
    "runtime health vocabulary changed");

Console.WriteLine("LATTICE_DESKTOP_POLICY_TEST_PASS");

using Lattice.Control.Desktop;

static void Require(bool condition, string message)
{
    if (!condition) throw new InvalidOperationException(message);
}

Uri defaultUri = DesktopPolicy.ResolveControlUri(["LATTICE.exe"], null);
Require(defaultUri == new Uri("http://127.0.0.1:4317/"), "default Control origin changed");

foreach (string rejected in new[]
{
    "https://example.com/",
    "http://localhost:4317/",
    "http://127.0.0.1/",
    "http://user@127.0.0.1:4317/",
    "https://127.0.0.1:4317/",
})
{
    Uri resolved = DesktopPolicy.ResolveControlUri(["LATTICE.exe", "--url", rejected], null);
    Require(resolved == DesktopPolicy.DefaultControlUri, $"unapproved Control origin accepted: {rejected}");
}

Uri alternateLoopback = DesktopPolicy.ResolveControlUri(
    ["LATTICE.exe", "--url", "http://127.0.0.1:54321/"],
    null);
Require(alternateLoopback.Port == 54321, "explicit loopback test origin was rejected");
Require(DesktopPolicy.ShouldManageControl(defaultUri), "default 4317 Control was not managed");
Require(!DesktopPolicy.ShouldManageControl(alternateLoopback), "alternate test Control was incorrectly managed");
Require(
    DesktopPolicy.IsApprovedControlNavigation(new Uri("http://127.0.0.1:4317/api/four-core"), defaultUri),
    "same-origin Control navigation was rejected");
Require(
    !DesktopPolicy.IsApprovedControlNavigation(new Uri("http://127.0.0.1:4318/"), defaultUri),
    "different loopback port was accepted");
Require(
    !DesktopPolicy.IsApprovedControlNavigation(new Uri("https://example.com/"), defaultUri),
    "external navigation was accepted");
Require(
    DesktopPolicy.CanApplyNavigationResult(42, 42, isClosing: false),
    "current navigation result was rejected");
Require(
    !DesktopPolicy.CanApplyNavigationResult(43, 42, isClosing: false),
    "stale navigation result was accepted");
Require(
    !DesktopPolicy.CanApplyNavigationResult(42, 42, isClosing: true),
    "closing window accepted a late navigation result");
Require(
    !DesktopPolicy.CanApplyNavigationResult(null, 42, isClosing: false),
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

ControlRuntimeIdentity expectedIdentity = ControlRuntimeContract.ExpectedIdentity;
Require(expectedIdentity.SchemaVersion == "lattice.control.runtime-identity.v1", "runtime identity schema changed");
Require(expectedIdentity.Product == "LATTICE_CONTROL", "runtime product identity changed");
Require(expectedIdentity.Version == "1.0.0-rc.1", "runtime compatibility version changed");

string compatibleSurface = """
{
  "schema_version":"lattice.control.runtime-surface.v1",
  "identity":{
    "schema_version":"lattice.control.runtime-identity.v1",
    "product":"LATTICE_CONTROL",
    "version":"1.0.0-rc.1"
  },
  "health":"HEALTHY",
  "capabilities":[
    {"id":"control_sqlite","label":"Control／SQLite","status":"HEALTHY"},
    {"id":"codex_app_server","label":"Codex App Server","status":"STOPPED"},
    {"id":"work_mcp","label":"Work MCP","status":"NO_DATA"},
    {"id":"decision_mcp","label":"Decision MCP","status":"NO_DATA"},
    {"id":"postgresql","label":"正式 PostgreSQL","status":"NOT_IMPLEMENTED"}
  ]
}
""";
ControlRuntimeEvaluation compatible = ControlRuntimeContract.EvaluateProbe(
    tcpReachable: true,
    statusCode: 200,
    responseBody: compatibleSurface);
Require(compatible.Health == ControlRuntimeHealth.HEALTHY, "compatible Control was not healthy");
Require(compatible.Action == ControlRuntimeAction.Reuse, "compatible Control was not reused");

ControlRuntimeEvaluation absent = ControlRuntimeContract.EvaluateProbe(false, null, null);
Require(absent.Health == ControlRuntimeHealth.UNREACHABLE, "absent Control was not unreachable");
Require(absent.Action == ControlRuntimeAction.StartOwned, "absent Control did not permit owned start");

ControlRuntimeEvaluation unknownListener = ControlRuntimeContract.EvaluateProbe(true, null, null);
Require(unknownListener.Health == ControlRuntimeHealth.INCOMPATIBLE, "unknown listener was not incompatible");
Require(unknownListener.Action == ControlRuntimeAction.FailClosed, "unknown listener did not fail closed");

foreach (string incompatibleSurface in new[]
{
    compatibleSurface.Replace("1.0.0-rc.1", "0.9.0", StringComparison.Ordinal),
    compatibleSurface.Replace("LATTICE_CONTROL", "FOREIGN_CONTROL", StringComparison.Ordinal),
    compatibleSurface.Replace("lattice.control.runtime-surface.v1", "lattice.control.runtime-surface.v2", StringComparison.Ordinal),
    compatibleSurface.Replace("\"NO_DATA\"", "\"UNKNOWN\"", StringComparison.Ordinal),
    "not-json",
})
{
    ControlRuntimeEvaluation rejected = ControlRuntimeContract.EvaluateProbe(true, 200, incompatibleSurface);
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

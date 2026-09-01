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
Require(
    DesktopPolicy.IsApprovedControlNavigation(new Uri("http://127.0.0.1:4317/api/four-core"), defaultUri),
    "same-origin Control navigation was rejected");
Require(
    !DesktopPolicy.IsApprovedControlNavigation(new Uri("http://127.0.0.1:4318/"), defaultUri),
    "different loopback port was accepted");
Require(
    !DesktopPolicy.IsApprovedControlNavigation(new Uri("https://example.com/"), defaultUri),
    "external navigation was accepted");

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

Console.WriteLine("LATTICE_DESKTOP_POLICY_TEST_PASS");

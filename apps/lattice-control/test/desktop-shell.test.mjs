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
  const [project, windowMarkup, windowCode, packageJson] = await Promise.all([
    repositoryFile("apps/lattice-control-desktop/Lattice.Control.Desktop.csproj"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml"),
    repositoryFile("apps/lattice-control-desktop/MainWindow.xaml.cs"),
    repositoryFile("package.json"),
  ]);

  assert.match(project, /<TargetFramework>net8\.0-windows<\/TargetFramework>/u);
  assert.match(project, /<UseWPF>true<\/UseWPF>/u);
  assert.match(project, /Microsoft\.Web\.WebView2" Version="1\.0\.4191\.47"/u);
  assert.match(windowMarkup, /WindowStyle="None"/u);
  assert.match(windowMarkup, /<wv2:WebView2/u);
  assert.match(windowCode, /http:\/\/127\.0\.0\.1/u);
  assert.match(windowCode, /AreDevToolsEnabled = false/u);
  assert.match(windowCode, /AreDefaultContextMenusEnabled = false/u);
  assert.match(windowCode, /NewWindowRequested/u);
  assert.match(windowCode, /uri\.Port == _controlUri\.Port/u);
  assert.doesNotMatch(windowCode, /msedge\.exe|Process\.Start\([^\n]*https?:/iu);
  assert.match(packageJson, /"desktop:build"/u);
});

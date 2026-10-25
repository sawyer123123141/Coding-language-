// The Kestrel extension: starts kestrel-lsp and connects it to the editor. Highlighting and
// brackets come from the grammar and the language configuration, without the server.
"use strict";

const fs = require("fs");
const path = require("path");
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

/// The server executable: the setting, else `kestrel-lsp` on PATH, else the one built in this
/// repository (the extension lives in kestrel/editors/vscode).
function serverPath() {
  const configured = vscode.workspace.getConfiguration("kestrel").get("server.path");
  if (configured) {
    return configured;
  }
  const exe = process.platform === "win32" ? "kestrel-lsp.exe" : "kestrel-lsp";
  for (const dir of (process.env.PATH || "").split(path.delimiter)) {
    const candidate = path.join(dir, exe);
    if (dir && fs.existsSync(candidate)) {
      return candidate;
    }
  }
  // The extension's folder may be a link into the repository: follow it.
  const root = path.resolve(fs.realpathSync(__dirname), "..", "..");
  for (const profile of ["release", "debug"]) {
    const candidate = path.join(root, "target", profile, exe);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return null;
}

function costLensSettings() {
  const c = vscode.workspace.getConfiguration("kestrel");
  return {
    costLens: {
      enabled: c.get("costLens.enabled"),
      build: c.get("costLens.build"),
      show: c.get("costLens.show"),
    },
  };
}

function activate(context) {
  const command = serverPath();
  if (!command) {
    vscode.window.showErrorMessage(
      "Kestrel: can't find kestrel-lsp. Build it with `cargo build --release -p kestrel-lsp` in kestrel/, or set kestrel.server.path."
    );
    return;
  }
  const serverOptions = { run: { command }, debug: { command } };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "kestrel" }],
    initializationOptions: costLensSettings(),
    // Sends `workspace/didChangeConfiguration` with the `kestrel` settings when they change.
    synchronize: { configurationSection: "kestrel" },
  };
  client = new LanguageClient("kestrel", "Kestrel", serverOptions, clientOptions);
  client.start();
  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };

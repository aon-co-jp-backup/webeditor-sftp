import * as path from "path";
import * as os from "os";
import * as fs from "fs";
import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wes-vscode-"));
  const exe = process.env.VSCODE_EXE ?? "C:\Users\noruk\AppData\Local\Programs\Microsoft VS Code\Code.exe";
  await runTests({
    vscodeExecutablePath: exe,
    extensionDevelopmentPath: path.resolve(__dirname, "../../"),
    extensionTestsPath: path.resolve(__dirname, "./suite/index"),
    launchArgs: ["--disable-extensions", "--disable-workspace-trust", "--user-data-dir", path.join(root, "u"), "--extensions-dir", path.join(root, "e")],
  });
}
main().catch((e) => {
  console.error(e);
  process.exit(1);
});

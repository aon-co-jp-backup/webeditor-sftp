import * as assert from "assert";
import * as vscode from "vscode";

const LABEL = "vscode-test-key";
const exec = <T>(id: string, args?: unknown) => vscode.commands.executeCommand<T>(id, args);

suite("webeditor-sftp", () => {
  suiteSetup(async () => {
    await vscode.extensions.getExtension("aon-co-jp.webeditor-sftp")?.activate();
  });
  suiteTeardown(async () => {
    try {
      await exec("webeditorSftp.deleteKeypair", { label: LABEL });
    } catch {
      /* ignore */
    }
  });

  test("コマンドが登録されている", async () => {
    const all = await vscode.commands.getCommands(true);
    for (const c of ["openBlogPreview", "openBlogEditor", "convertToHtml", "convertToBlog", "generateKeypair", "deleteKeypair", "showPairingQr", "appendAuthorizedKey", "uploadCurrentFile"]) {
      assert.ok(all.includes(`webeditorSftp.${c}`), c);
    }
  });

  test("ブログ→HTML→ブログ変換", async () => {
    const doc = await vscode.workspace.openTextDocument({ content: "# 題\n\n本文 **太字**", language: "markdown" });
    await vscode.window.showTextDocument(doc);
    const html = await exec<string>("webeditorSftp.convertToHtml");
    assert.match(html, /<h1>題<\/h1>/);
    assert.match(html, /<strong>太字<\/strong>/);
    const htmlDoc = await vscode.workspace.openTextDocument({ content: html, language: "html" });
    await vscode.window.showTextDocument(htmlDoc);
    const blog = await exec<string>("webeditorSftp.convertToBlog");
    assert.match(blog, /# 題/);
  });

  test("鍵の生成・QR・削除", async () => {
    const k = await exec<{ label: string; public_key_openssh: string }>("webeditorSftp.generateKeypair", { label: LABEL });
    assert.equal(k.label, LABEL);
    assert.match(k.public_key_openssh, /^ssh-ed25519 /);
    const qr = await exec<string>("webeditorSftp.showPairingQr", { label: LABEL });
    assert.match(qr, /^data:image\/png;base64,/);
    assert.equal(await exec("webeditorSftp.deleteKeypair", { label: LABEL }), true);
  });

  test("ブログモード編集の書き戻し", async () => {
    const { applyBlogText } = await import("../../blogEditorPanel");
    const { SidecarClient, resolveSidecarPath } = await import("../../sidecarClient");
    const side = new SidecarClient(resolveSidecarPath(vscode.extensions.getExtension("aon-co-jp.webeditor-sftp")!.extensionPath, ""));
    const doc = await vscode.workspace.openTextDocument({ content: "<p>old</p>", language: "html" });
    await vscode.window.showTextDocument(doc);
    const html = await applyBlogText(side, doc, "## 新\n\n- a");
    assert.equal(doc.getText().split("\r\n").join("\n"), html);
    assert.match(doc.getText(), /<h2>新<\/h2>/);
    side.dispose();
  });

  test("ブログモード編集/プレビューのWebviewが開く", async () => {
    const doc = await vscode.workspace.openTextDocument({ content: "<h1>x</h1>", language: "html" });
    await vscode.window.showTextDocument(doc);
    await exec("webeditorSftp.openBlogEditor");
    await exec("webeditorSftp.openBlogPreview");
    await new Promise((r) => setTimeout(r, 1500));
    const tabs = vscode.window.tabGroups.all.flatMap((g) => g.tabs).map((t) => t.label);
    assert.ok(tabs.some((t) => t.includes("ブログモード編集")), tabs.join(","));
  });

  test("アップロード失敗(鍵なし)は例外で返る", async () => {
    await assert.rejects(async () =>
      exec("webeditorSftp.uploadCurrentFile", { keyLabel: "no-such", host: "127.0.0.1", port: 1, username: "x", remotePath: "/x", content: "x", execAfterUpload: "" }),
      /./,
    );
  });
});

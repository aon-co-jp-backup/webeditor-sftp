import * as vscode from "vscode";
import { SidecarClient, resolveSidecarPath } from "./sidecarClient";
import { BlogPreviewPanel } from "./blogPreviewPanel";
import { BlogEditorPanel } from "./blogEditorPanel";

let sidecar: SidecarClient | undefined;

function getSidecar(context: vscode.ExtensionContext): SidecarClient {
  if (sidecar) return sidecar;
  const configured = vscode.workspace.getConfiguration("webeditorSftp").get<string>("sidecarPath");
  sidecar = new SidecarClient(resolveSidecarPath(context.extensionPath, configured));
  return sidecar;
}

export interface KeyInfo {
  label: string;
  algorithm: string;
  public_key_openssh: string;
  fingerprint: string;
}

export interface UploadResult {
  remote_path: string;
  bytes_written: number;
  host_key_trust: "trusted_first_time" | "known";
  exec_output?: { exit_status?: number; stdout: string; stderr: string };
}

/** 各コマンドが受け取る引数(省略した項目だけ入力ボックスで尋ねる。自動試験・キーバインド・タスクから使える)。 */
export interface ConnectionArgs {
  keyLabel?: string;
  host?: string;
  port?: number;
  username?: string;
}
export interface UploadArgs extends ConnectionArgs {
  remotePath?: string;
  execAfterUpload?: string;
  /** 省略時は開いているエディターの内容 */
  content?: string;
}
export interface AppendKeyArgs extends ConnectionArgs {
  publicKeyLine?: string;
}

const KEYS_STATE = "webeditorSftp.keys";

function storedKeys(context: vscode.ExtensionContext): KeyInfo[] {
  return context.globalState.get<KeyInfo[]>(KEYS_STATE, []);
}

/** 引数があればそれを使い、無ければ入力ボックスで尋ねる(キャンセルならundefined)。 */
async function ask(preset: string | undefined, options: vscode.InputBoxOptions): Promise<string | undefined> {
  if (preset !== undefined) return preset;
  const v = await vscode.window.showInputBox(options);
  return v && v.trim() ? v.trim() : undefined;
}

export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel("webeditor-sftp");
  context.subscriptions.push(output);

  /** コマンド共通の実行ラッパー。引数つき(自動実行)の失敗は例外として返し、対話実行はメッセージ表示のみ。 */
  const guard =
    <A, R>(fn: (args: A | undefined) => Promise<R | undefined>) =>
    async (args?: A): Promise<R | undefined> => {
      try {
        return await fn(args);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        if (args !== undefined) throw e;
        void vscode.window.showErrorMessage(`webeditor-sftp: ${msg}`);
        return undefined;
      }
    };

  const reg = (id: string, fn: (...a: never[]) => unknown) =>
    context.subscriptions.push(vscode.commands.registerCommand(id, fn as (...a: unknown[]) => unknown));

  // --- 変換 ---
  reg(
    "webeditorSftp.openBlogPreview",
    guard(async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) return void vscode.window.showWarningMessage("プレビューするファイルを開いてください。");
      await BlogPreviewPanel.createOrShow(getSidecar(context), editor.document);
    }),
  );
  reg(
    "webeditorSftp.openBlogEditor",
    guard(async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) return void vscode.window.showWarningMessage("編集するHTMLファイルを開いてください。");
      await BlogEditorPanel.createOrShow(getSidecar(context), editor.document);
    }),
  );
  reg(
    "webeditorSftp.convertToHtml",
    guard(async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) return undefined;
      const html = await getSidecar(context).request<string>("editor.blogToHtml", { text: editor.document.getText() });
      const doc = await vscode.workspace.openTextDocument({ content: html, language: "html" });
      await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside });
      return html;
    }),
  );
  reg(
    "webeditorSftp.convertToBlog",
    guard(async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) return undefined;
      const text = await getSidecar(context).request<string>("editor.htmlToBlog", { html: editor.document.getText() });
      const doc = await vscode.workspace.openTextDocument({ content: text, language: "plaintext" });
      await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside });
      return text;
    }),
  );

  // --- 鍵管理 ---
  reg(
    "webeditorSftp.generateKeypair",
    guard<{ label?: string }, KeyInfo>(async (args) => {
      const label = await ask(args?.label, {
        prompt: "鍵ラベル(識別名)を入力してください",
        placeHolder: "例: audiocafe.tokyo-root",
      });
      if (!label) return undefined;
      const info = await getSidecar(context).request<KeyInfo>("sftp.generateKeypair", { label });
      await context.globalState.update(KEYS_STATE, [...storedKeys(context).filter((k) => k.label !== info.label), info]);
      output.appendLine(`鍵ペア生成完了: ${info.label} (${info.fingerprint})`);
      output.appendLine(`公開鍵: ${info.public_key_openssh}`);
      output.show(true);
      void vscode.window.showInformationMessage(
        `鍵ペア "${info.label}" を生成しOSセキュアストレージへ保存しました(フィンガープリント: ${info.fingerprint})。`,
      );
      return info;
    }),
  );
  reg(
    "webeditorSftp.deleteKeypair",
    guard<{ label?: string }, boolean>(async (args) => {
      const label =
        args?.label ??
        (await vscode.window.showQuickPick(storedKeys(context).map((k) => k.label), { placeHolder: "削除する鍵を選択" }));
      if (!label) return undefined;
      await getSidecar(context).request("sftp.deleteKeypair", { label });
      await context.globalState.update(KEYS_STATE, storedKeys(context).filter((k) => k.label !== label));
      output.appendLine(`鍵を削除しました: ${label}`);
      return true;
    }),
  );
  reg(
    "webeditorSftp.showPairingQr",
    guard<{ label?: string }, string>(async (args) => {
      const keys = storedKeys(context);
      const label =
        args?.label ?? (await vscode.window.showQuickPick(keys.map((k) => k.label), { placeHolder: "QRにする公開鍵を選択" }));
      if (!label) return undefined;
      const key = keys.find((k) => k.label === label);
      if (!key) throw new Error(`鍵「${label}」の公開鍵情報が見つかりません(このVSCodeで生成した鍵のみ対象です)`);
      const dataUri = await getSidecar(context).request<string>("sftp.generatePairingQr", {
        payload: { label: key.label, public_key_openssh: key.public_key_openssh, fingerprint: key.fingerprint },
      });
      const panel = vscode.window.createWebviewPanel("webeditorSftp.pairingQr", `公開鍵QR: ${key.label}`, vscode.ViewColumn.Beside, {});
      panel.webview.html = `<body style="font-family:sans-serif;padding:16px">
        <p>別端末(スマホ等のwebeditor-sftp)のカメラで読み取ると、公開鍵を受け渡せます(秘密鍵はQR化されません)。</p>
        <img src="${dataUri}" alt="公開鍵QR" style="width:320px;height:320px;background:#fff">
        <p><code>${key.fingerprint}</code></p></body>`;
      return dataUri;
    }),
  );
  reg(
    "webeditorSftp.appendAuthorizedKey",
    guard<AppendKeyArgs, UploadResult>(async (args) => {
      const keyLabel = await ask(args?.keyLabel, { prompt: "サーバーへ接続に使う鍵ラベル" });
      if (!keyLabel) return undefined;
      const host = await ask(args?.host, { prompt: "ホスト" });
      if (!host) return undefined;
      const username = await ask(args?.username, { prompt: "ユーザー名" });
      if (!username) return undefined;
      const publicKeyLine = await ask(args?.publicKeyLine, {
        prompt: "登録する公開鍵(ssh-ed25519 AAAA... 形式。別端末のアプリで表示/コピーしたもの)",
      });
      if (!publicKeyLine) return undefined;
      const result = await getSidecar(context).request<UploadResult>("sftp.appendAuthorizedKey", {
        host,
        port: args?.port ?? 22,
        username,
        keyLabel,
        publicKeyLine,
      });
      report(output, result);
      return result;
    }),
  );

  // --- アップロード ---
  reg(
    "webeditorSftp.uploadCurrentFile",
    guard<UploadArgs, UploadResult>(async (args) => {
      const content = args?.content ?? vscode.window.activeTextEditor?.document.getText();
      if (content === undefined) return void vscode.window.showWarningMessage("アップロードするファイルを開いてください。");
      const keyLabel = await ask(args?.keyLabel, { prompt: "使用する鍵ラベル" });
      if (!keyLabel) return undefined;
      const host = await ask(args?.host, { prompt: "ホスト", placeHolder: "例: audiocafe.tokyo" });
      if (!host) return undefined;
      const username = await ask(args?.username, { prompt: "ユーザー名", placeHolder: "例: root" });
      if (!username) return undefined;
      const remotePath = await ask(args?.remotePath, {
        prompt: "アップロード先パス",
        placeHolder: "例: /var/www/audiocafe.tokyo/public/index.html",
      });
      if (!remotePath) return undefined;
      const port = args?.port ?? (Number(await ask(undefined, { prompt: "ポート", value: "22" })) || 22);
      const execAfterUpload =
        args?.execAfterUpload ??
        (await vscode.window.showInputBox({
          prompt: "アップロード後に実行するコマンド(任意、空欄可)",
          placeHolder: "例: cd /path && git pull && cargo build --release && systemctl restart ...",
        }));
      const result = await getSidecar(context).request<UploadResult>("sftp.uploadText", {
        host,
        port,
        username,
        keyLabel,
        remotePath,
        content,
        execAfterUpload: execAfterUpload || undefined,
      });
      report(output, result);
      void vscode.window.showInformationMessage(`アップロード完了: ${result.remote_path}`);
      return result;
    }),
  );

  // --- HTMLファイルを開いているときだけ出るステータスバーの入口(アイコンが見つからない場合の代替) ---
  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  status.text = "$(book) ブログモード";
  status.tooltip = "webeditor-sftp: このHTMLをタグを意識しないブログモードで編集";
  status.command = "webeditorSftp.openBlogEditor";
  const refreshStatus = () => {
    if (vscode.window.activeTextEditor?.document.languageId === "html") status.show();
    else status.hide();
  };
  context.subscriptions.push(status, vscode.window.onDidChangeActiveTextEditor(refreshStatus));
  refreshStatus();
}

function report(output: vscode.OutputChannel, result: UploadResult): void {
  if (result.host_key_trust === "trusted_first_time") {
    output.appendLine("⚠️ このホストへの初回接続です。ホスト鍵を新規に信頼して記録しました(TOFU)。");
  }
  output.appendLine(`完了: ${result.remote_path} (${result.bytes_written} bytes)`);
  if (result.exec_output) {
    output.appendLine(`コマンド実行結果 (exit=${result.exec_output.exit_status ?? "unknown"}):`);
    if (result.exec_output.stdout.trim()) output.appendLine(`  stdout: ${result.exec_output.stdout.trim()}`);
    if (result.exec_output.stderr.trim()) output.appendLine(`  stderr: ${result.exec_output.stderr.trim()}`);
  }
  output.show(true);
}

export function deactivate(): void {
  sidecar?.dispose();
  sidecar = undefined;
}

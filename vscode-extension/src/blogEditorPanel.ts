import * as vscode from "vscode";
import { SidecarClient } from "./sidecarClient";

/**
 * ブログモードの編集画面(Webview)。開いているHTMLを「タグを意識しない文章」として
 * 編集でき、「HTMLへ反映」で変換結果を元のファイルへ書き戻す(双方向編集)。
 * 変換ロジック本体はRust側(webeditor-core::editor)で、ここではサイドカー経由で呼ぶだけ。
 */
export class BlogEditorPanel {
  private static panels = new Map<string, BlogEditorPanel>();
  private readonly disposables: vscode.Disposable[] = [];

  private constructor(
    private readonly panel: vscode.WebviewPanel,
    private readonly sidecar: SidecarClient,
    private readonly document: vscode.TextDocument,
  ) {
    this.panel.onDidDispose(() => this.dispose(), null, this.disposables);
    this.panel.webview.onDidReceiveMessage((m) => void this.onMessage(m), null, this.disposables);
    void this.load();
  }

  static async createOrShow(sidecar: SidecarClient, document: vscode.TextDocument): Promise<void> {
    const key = document.uri.toString();
    const existing = BlogEditorPanel.panels.get(key);
    if (existing) {
      existing.panel.reveal();
      await existing.load();
      return;
    }
    const name = document.fileName.split(/[\\/]/).pop();
    const panel = vscode.window.createWebviewPanel(
      "webeditorSftp.blogEditor",
      `ブログモード編集: ${name}`,
      vscode.ViewColumn.Beside,
      { enableScripts: true },
    );
    BlogEditorPanel.panels.set(key, new BlogEditorPanel(panel, sidecar, document));
  }

  /** 元のHTMLからブログ文章を作って画面へ送る(開き直し/再読み込み)。 */
  private async load(): Promise<void> {
    try {
      const text = await this.sidecar.request<string>("editor.htmlToBlog", { html: this.document.getText() });
      this.panel.webview.html = this.render(text);
    } catch (e) {
      this.panel.webview.html = `<body style="font-family:sans-serif;padding:16px;color:var(--vscode-errorForeground)">
        <b>変換エラー</b><pre>${escapeHtml(e instanceof Error ? e.message : String(e))}</pre></body>`;
    }
  }

  private async onMessage(m: { type: string; text?: string }): Promise<void> {
    try {
      if (m.type === "apply" && typeof m.text === "string") {
        await applyBlogText(this.sidecar, this.document, m.text);
        void this.panel.webview.postMessage({ type: "status", message: "HTMLへ反映しました(ファイルは未保存の状態です)" });
      } else if (m.type === "reload") {
        const text = await this.sidecar.request<string>("editor.htmlToBlog", { html: this.document.getText() });
        void this.panel.webview.postMessage({ type: "text", text, message: "HTMLの現在の内容から読み込み直しました" });
      }
    } catch (e) {
      void this.panel.webview.postMessage({ type: "status", message: `エラー: ${e instanceof Error ? e.message : String(e)}` });
    }
  }

  private render(blogText: string): string {
    const nonce = Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
    return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-${nonce}';">
<style>
  body { font-family: system-ui, sans-serif; padding: 12px; display: flex; flex-direction: column; height: 100vh; box-sizing: border-box; margin: 0; }
  textarea { flex: 1; width: 100%; box-sizing: border-box; font-size: 14px; line-height: 1.6; padding: 10px;
    background: var(--vscode-input-background); color: var(--vscode-input-foreground); border: 1px solid var(--vscode-input-border, #555); }
  .bar { display: flex; gap: 8px; align-items: center; margin: 8px 0; }
  button { padding: 6px 14px; background: var(--vscode-button-background); color: var(--vscode-button-foreground); border: 0; cursor: pointer; }
  button.sub { background: var(--vscode-button-secondaryBackground); color: var(--vscode-button-secondaryForeground); }
  .hint { color: var(--vscode-descriptionForeground); font-size: 12px; }
</style></head><body>
<div class="hint"># 見出し / - 箇条書き / 1. 番号 / &gt; 引用 / \`\`\` コード / **太字** / *斜体* / [文字](URL) / ![代替](URL) / | 表 |</div>
<textarea id="t" spellcheck="false">${escapeHtml(blogText)}</textarea>
<div class="bar"><button id="apply">HTMLへ反映</button><button id="reload" class="sub">HTMLから読み込み直す</button><span id="s" class="hint"></span></div>
<script nonce="${nonce}">
  const vscode = acquireVsCodeApi();
  const t = document.getElementById('t'), s = document.getElementById('s');
  document.getElementById('apply').onclick = () => vscode.postMessage({ type: 'apply', text: t.value });
  document.getElementById('reload').onclick = () => vscode.postMessage({ type: 'reload' });
  window.addEventListener('message', (e) => {
    if (e.data.type === 'text') t.value = e.data.text;
    s.textContent = e.data.message || '';
  });
</script></body></html>`;
  }

  private dispose(): void {
    BlogEditorPanel.panels.delete(this.document.uri.toString());
    this.disposables.forEach((d) => d.dispose());
  }
}

/** ブログ文章をHTMLへ変換し、ドキュメント全体を置き換える。変換後のHTMLを返す。 */
export async function applyBlogText(
  sidecar: SidecarClient,
  document: vscode.TextDocument,
  blogText: string,
): Promise<string> {
  const html = await sidecar.request<string>("editor.blogToHtml", { text: blogText });
  const edit = new vscode.WorkspaceEdit();
  edit.replace(document.uri, new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)), html);
  if (!(await vscode.workspace.applyEdit(edit))) {
    throw new Error("ドキュメントへの書き戻しに失敗しました");
  }
  return html;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

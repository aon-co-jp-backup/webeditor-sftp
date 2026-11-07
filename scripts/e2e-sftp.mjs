// 実SSH/SFTPサーバーに対する通し試験(Windows標準のOpenSSH sshd.exeを一般ユーザー権限で
// 高ポートへ起動して使う)。webeditor-sidecarを実プロセスで起動し、本物のプロトコルで検証する。
//
// 使い方(リポジトリ直下): cargo build -p webeditor-sidecar && node scripts/e2e-sftp.mjs
//
// 検証項目:
//  1. 初回接続=TOFU信頼(trusted_first_time)+ファイル内容の一致
//  2. 2回目接続=既知(known)
//  3. アップロード後コマンドの標準出力と終了コード(0/非0)
//  4. 未登録の鍵での認証失敗・存在しない鍵ラベル・接続先なし、のエラー
//  5. ホスト鍵を差し替えたサーバー(なりすまし)への接続拒否+何も書き込まれないこと
//  6. authorized_keys追記(既存の鍵を保持)と重複追記のスキップ
//
// 後始末: 触った~/.ssh/authorized_keysはバイト単位で復元、試験用の鍵(資格情報)と
// known_hostsの記録は削除、sshdは停止する。

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import assert from "node:assert/strict";

const PORT = Number(process.env.E2E_PORT ?? 2222);
const SSHD = "C:/Windows/System32/OpenSSH/sshd.exe";
const SFTP_SERVER = "C:/Windows/System32/OpenSSH/sftp-server.exe";
const repo = path.resolve(import.meta.dirname, "..");
const exe = process.platform === "win32" ? "webeditor-sidecar.exe" : "webeditor-sidecar";
const sidecarPath = [path.join(repo, "target", "debug", exe), path.join(repo, "target", "release", exe)].find(fs.existsSync);
if (!sidecarPath) throw new Error("webeditor-sidecarが見つかりません。先に cargo build -p webeditor-sidecar を実行してください");

const work = fs.mkdtempSync(path.join(os.tmpdir(), "webeditor-e2e-"));
const uploadDir = path.join(work, "upload");
fs.mkdirSync(uploadDir);
const user = os.userInfo().username;
const realAuthKeys = path.join(os.homedir(), ".ssh", "authorized_keys");
const toSftpPath = (p) => "/" + p.replace(/\\/g, "/");

// ---- サイドカーとの通信(JSON Lines) ----
const sidecar = spawn(sidecarPath, [], { stdio: ["pipe", "pipe", "inherit"] });
const pending = new Map();
let nextId = 1;
readline.createInterface({ input: sidecar.stdout }).on("line", (line) => {
  const m = JSON.parse(line);
  pending.get(m.id)?.(m);
  pending.delete(m.id);
});
const call = (method, params) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    sidecar.stdin.write(JSON.stringify({ id, method, params }) + "\n");
  });
const ok = async (method, params) => {
  const r = await call(method, params);
  assert.ok(r.ok, `${method} が失敗: ${r.error}`);
  return r.result;
};

// ---- 試験用sshd ----
let sshd;
function startSshd() {
  const hostKey = path.join(work, "hostkey");
  fs.rmSync(hostKey, { force: true });
  fs.rmSync(hostKey + ".pub", { force: true });
  assert.equal(spawnSync("ssh-keygen", ["-t", "ed25519", "-N", "", "-f", hostKey, "-q"]).status, 0);
  const cfg = path.join(work, "sshd_config");
  fs.writeFileSync(
    cfg,
    [
      `Port ${PORT}`,
      "ListenAddress 127.0.0.1",
      `HostKey ${hostKey.replace(/\\/g, "/")}`,
      `AuthorizedKeysFile ${path.join(work, "authorized_keys").replace(/\\/g, "/")}`,
      "StrictModes no",
      "PasswordAuthentication no",
      `Subsystem sftp ${SFTP_SERVER}`,
    ].join("\n"),
  );
  sshd = spawn(SSHD, ["-D", "-e", "-f", cfg], { stdio: "ignore" });
}
const stopSshd = async () => {
  sshd?.kill();
  await new Promise((r) => setTimeout(r, 800));
};
const waitListening = async () => {
  for (let i = 0; i < 50; i++) {
    const up = await new Promise((r) => {
      const s = net.connect(PORT, "127.0.0.1").once("connect", () => (s.destroy(), r(true))).once("error", () => r(false));
    });
    if (up) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("sshdが起動しませんでした");
};
import net from "node:net";

const base = { host: "127.0.0.1", port: PORT, username: user };
const upload = (label, name, content, extra = {}) =>
  call("sftp.uploadText", { ...base, keyLabel: label, remotePath: toSftpPath(path.join(uploadDir, name)), content, ...extra });

const results = [];
const check = (name, fn) => results.push([name, fn]);
let authBackup;
const labels = ["e2e-main-key", "e2e-other-key"];

try {
  startSshd();
  await waitListening();

  const main = await ok("sftp.generateKeypair", { label: labels[0] });
  const other = await ok("sftp.generateKeypair", { label: labels[1] });
  fs.writeFileSync(path.join(work, "authorized_keys"), main.public_key_openssh + "\n");
  await ok("sftp.forgetHost", { host: "127.0.0.1", port: PORT });

  check("初回接続: TOFU信頼+内容一致", async () => {
    const r = await upload(labels[0], "a.html", "<h1>one</h1>");
    assert.ok(r.ok, r.error);
    assert.equal(r.result.host_key_trust, "trusted_first_time");
    assert.equal(fs.readFileSync(path.join(uploadDir, "a.html"), "utf8"), "<h1>one</h1>");
  });
  check("2回目接続: 既知+上書き", async () => {
    const r = await upload(labels[0], "a.html", "<h1>two</h1>");
    assert.ok(r.ok, r.error);
    assert.equal(r.result.host_key_trust, "known");
    assert.equal(fs.readFileSync(path.join(uploadDir, "a.html"), "utf8"), "<h1>two</h1>");
  });
  check("後続コマンド: 標準出力と終了コード0", async () => {
    const r = await upload(labels[0], "a.html", "x", { execAfterUpload: "echo hello" });
    assert.ok(r.ok, r.error);
    assert.equal(r.result.exec_output.exit_status, 0);
    assert.match(r.result.exec_output.stdout, /hello/);
  });
  check("後続コマンド: 非0の終了コードを取得", async () => {
    const r = await upload(labels[0], "a.html", "x", { execAfterUpload: "cmd /c exit 7" });
    assert.ok(r.ok, r.error);
    assert.equal(r.result.exec_output.exit_status, 7);
  });
  check("未登録の鍵は認証失敗", async () => {
    const r = await upload(labels[1], "b.html", "x");
    assert.equal(r.ok, false);
    assert.match(r.error, /認証に失敗/);
  });
  check("存在しない鍵ラベルは分かりやすい日本語", async () => {
    const r = await upload("no-such-key-label", "b.html", "x");
    assert.equal(r.ok, false);
    assert.match(r.error, /見つかりません/);
  });
  check("接続先が無い場合はエラー", async () => {
    const r = await call("sftp.uploadText", { ...base, port: PORT + 1000, keyLabel: labels[0], remotePath: "/x", content: "x" });
    assert.equal(r.ok, false);
    assert.match(r.error, /接続/);
  });
  check("authorized_keys追記(既存を保持)+重複スキップ", async () => {
    fs.mkdirSync(path.dirname(realAuthKeys), { recursive: true });
    authBackup = fs.existsSync(realAuthKeys) ? fs.readFileSync(realAuthKeys) : null; // バイト単位で退避
    const before = authBackup ? authBackup.toString("utf8") : "";
    const p = { ...base, keyLabel: labels[0], publicKeyLine: other.public_key_openssh };
    const r1 = await call("sftp.appendAuthorizedKey", p);
    assert.ok(r1.ok, r1.error);
    const after = fs.readFileSync(realAuthKeys, "utf8");
    assert.ok(after.startsWith(before.trimEnd()), "既存の内容が保持されていない");
    assert.ok(after.includes(other.public_key_openssh));
    const r2 = await call("sftp.appendAuthorizedKey", p);
    assert.ok(r2.ok, r2.error);
    assert.match(r2.result.exec_output.stdout, /登録済み/);
    assert.equal(fs.readFileSync(realAuthKeys, "utf8").split(other.public_key_openssh).length, 2, "重複して追記された");
  });
  check("なりすまし(ホスト鍵差し替え)は拒否され何も書き込まれない", async () => {
    await stopSshd();
    startSshd(); // ホスト鍵を再生成
    await waitListening();
    const r = await upload(labels[0], "evil.html", "leaked");
    assert.equal(r.ok, false);
    assert.match(r.error, /一致しません/, `詳細な拒否理由が出ていない: ${r.error}`);
    assert.equal(fs.existsSync(path.join(uploadDir, "evil.html")), false);
    // 意図した鍵変更時の復旧手順(記録リセット→再信頼)も確認
    await ok("sftp.forgetHost", { host: "127.0.0.1", port: PORT });
    const r2 = await upload(labels[0], "evil.html", "ok");
    assert.ok(r2.ok, r2.error);
    assert.equal(r2.result.host_key_trust, "trusted_first_time");
  });

  let failed = 0;
  for (const [name, fn] of results) {
    try {
      await fn();
      console.log(`✅ ${name}`);
    } catch (e) {
      failed++;
      console.log(`❌ ${name}\n   ${e.message}`);
    }
  }
  console.log(failed ? `\n${failed}件失敗` : `\n全${results.length}件成功`);
  process.exitCode = failed ? 1 : 0;
} finally {
  // 後始末(失敗時も必ず実行)
  if (authBackup !== undefined) {
    if (authBackup === null) fs.rmSync(realAuthKeys, { force: true });
    else fs.writeFileSync(realAuthKeys, authBackup);
  }
  for (const l of labels) await call("sftp.deleteKeypair", { label: l });
  await call("sftp.forgetHost", { host: "127.0.0.1", port: PORT });
  sidecar.kill();
  await stopSshd();
  fs.rmSync(work, { recursive: true, force: true });
}

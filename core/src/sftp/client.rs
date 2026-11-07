//! SFTP接続・アップロード(russh + russh-sftpによる非同期実装)。
//!
//! ホスト鍵検証はTOFU(`known_hosts.rs`)で行う。初回接続はフィンガー
//! プリントを記録して許可、以後は記録済み値との一致を必須とする。

use std::sync::{Arc, Mutex};

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;

use super::known_hosts::{self, TrustOutcome};
use super::SftpError;

struct TofuHostKeyVerifier {
    host: String,
    port: u16,
    /// `check_server_key`はHandlerトレイトの制約上フィンガープリントの
    /// 照合結果を戻り値(bool)でしか返せないため、詳細(初回信頼か既知か)
    /// を呼び出し元へ伝えるためにここへ書き込む。
    outcome: Arc<Mutex<Option<TrustOutcome>>>,
    /// ホスト鍵を拒否した理由(記録済みと今回のフィンガープリント等)。russhは
    /// 拒否時に「Unknown server key」としか返さないため、詳細を呼び出し元へ伝える。
    rejection: Arc<Mutex<Option<String>>>,
}

impl client::Handler for TofuHostKeyVerifier {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_public_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Ok(false);
        };
        // 既存のknown_hostsとの互換のため、旧版と同じ「SHA256:」接頭辞なしの形式で照合する。
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let fingerprint = fingerprint.strip_prefix("SHA256:").unwrap_or(&fingerprint).to_string();
        match known_hosts::verify_or_trust(&self.host, self.port, &fingerprint) {
            Ok(outcome) => {
                *self.outcome.lock().unwrap() = Some(outcome);
                Ok(true)
            }
            Err(e) => {
                // 不一致(なりすましの疑い)。接続を拒否し、理由を保存する。
                *self.rejection.lock().unwrap() = Some(e.to_string());
                Ok(false)
            }
        }
    }
}

pub struct UploadResult {
    pub remote_path: String,
    pub bytes_written: u64,
    pub host_key_trust: TrustOutcome,
    /// `exec_after_upload`を指定した場合のコマンド実行結果。
    pub exec_output: Option<ExecOutput>,
}

pub struct ExecOutput {
    pub exit_status: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

async fn connect_and_authenticate(
    host: &str,
    port: u16,
    username: &str,
    key_pair: PrivateKey,
) -> Result<(Handle<TofuHostKeyVerifier>, TrustOutcome), SftpError> {
    let outcome_slot: Arc<Mutex<Option<TrustOutcome>>> = Arc::new(Mutex::new(None));
    let rejection_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let handler = TofuHostKeyVerifier {
        host: host.to_string(),
        port,
        outcome: outcome_slot.clone(),
        rejection: rejection_slot.clone(),
    };

    let config = Arc::new(client::Config::default());
    let session = client::connect(config, (host, port), handler)
        .await
        .map_err(|e| {
            // TOFU検証で拒否された場合は、その詳細な理由(記録済み/今回の
            // フィンガープリントと対処法)をそのまま返す。
            if let Some(reason) = rejection_slot.lock().unwrap().take() {
                return SftpError::Connection(reason);
            }
            SftpError::Connection(format!("接続に失敗しました({e})"))
        })?;

    let outcome = outcome_slot
        .lock()
        .unwrap()
        .ok_or_else(|| SftpError::Connection("ホスト鍵検証が実行されませんでした".into()))?;

    let mut session = session;
    let authenticated = session
        .authenticate_publickey(username, PrivateKeyWithHashAlg::new(Arc::new(key_pair), Some(HashAlg::Sha512)))
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?
        .success();
    if !authenticated {
        return Err(SftpError::Connection(
            "公開鍵認証に失敗しました(サーバー側にauthorized_keysが未登録の可能性)".into(),
        ));
    }

    Ok((session, outcome))
}

/// 公開鍵認証でSSH接続し、SFTPで1ファイルをアップロードする。
/// `exec_after_upload`を指定すると、アップロード完了後にそのコマンドを
/// SSH経由で実行する(例: `audiocafe-tokyo-rust`のような`include_str!`
/// でコンパイル時埋め込みされるサイトの再ビルド・再起動用)。
pub async fn upload_file(
    host: &str,
    port: u16,
    username: &str,
    key_pair: PrivateKey,
    remote_path: &str,
    content: &[u8],
    exec_after_upload: Option<&str>,
) -> Result<UploadResult, SftpError> {
    let (mut session, host_key_trust) =
        connect_and_authenticate(host, port, username, key_pair).await?;

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?;

    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;

    let mut file = sftp
        .create(remote_path)
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;

    use tokio::io::AsyncWriteExt;
    file.write_all(content)
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;
    file.shutdown()
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;

    sftp.close().await.map_err(|e| SftpError::Sftp(e.to_string()))?;

    let exec_output = match exec_after_upload {
        Some(cmd) if !cmd.trim().is_empty() => Some(run_command(&mut session, cmd).await?),
        _ => None,
    };

    Ok(UploadResult {
        remote_path: remote_path.to_string(),
        bytes_written: content.len() as u64,
        host_key_trust,
        exec_output,
    })
}

/// 既存のSFTP接続(公開鍵認証済み)を使って、リモートの
/// `~/.ssh/authorized_keys` に公開鍵を1行追記する
/// (QRコードで受け取った他端末の公開鍵をサーバーへ登録する用途)。
pub async fn append_authorized_key(
    host: &str,
    port: u16,
    username: &str,
    key_pair: PrivateKey,
    public_key_line: &str,
) -> Result<UploadResult, SftpError> {
    let (session, host_key_trust) =
        connect_and_authenticate(host, port, username, key_pair).await?;

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;

    let remote_path = ".ssh/authorized_keys";
    let mut existing = String::new();
    if let Ok(mut f) = sftp.open(remote_path).await {
        use tokio::io::AsyncReadExt;
        let _ = f.read_to_string(&mut existing).await;
    }

    if existing.lines().any(|l| l.trim() == public_key_line.trim()) {
        return Ok(UploadResult {
            remote_path: remote_path.to_string(),
            bytes_written: 0,
            host_key_trust,
            exec_output: Some(ExecOutput {
                exit_status: Some(0),
                stdout: "この公開鍵は既にauthorized_keysに登録済みです(重複追記をスキップ)".into(),
                stderr: String::new(),
            }),
        });
    }

    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(public_key_line.trim());
    existing.push('\n');

    let mut file = sftp
        .create(remote_path)
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(existing.as_bytes())
        .await
        .map_err(|e| SftpError::Sftp(e.to_string()))?;
    file.shutdown().await.map_err(|e| SftpError::Sftp(e.to_string()))?;
    sftp.close().await.map_err(|e| SftpError::Sftp(e.to_string()))?;

    Ok(UploadResult {
        remote_path: remote_path.to_string(),
        bytes_written: public_key_line.len() as u64,
        host_key_trust,
        exec_output: None,
    })
}

async fn run_command(
    session: &mut Handle<TofuHostKeyVerifier>,
    command: &str,
) -> Result<ExecOutput, SftpError> {
    use russh::ChannelMsg;

    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| SftpError::Connection(e.to_string()))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;

    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
            Some(ChannelMsg::ExtendedData { data, .. }) => stderr.extend_from_slice(&data),
            Some(ChannelMsg::ExitStatus { exit_status: status }) => {
                exit_status = Some(status);
            }
            // 終了コード(ExitStatus)はEofの後・Closeの前に届くことが多いため、
            // Eofでは打ち切らずCloseまで待つ(実sshdでの通し試験で発覚、2026-11-07)。
            Some(ChannelMsg::Close) | None => break,
            _ => {}
        }
    }

    Ok(ExecOutput {
        exit_status,
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
    })
}

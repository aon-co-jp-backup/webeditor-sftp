//! 鍵ペア生成(ed25519)。

use russh::keys::{Algorithm, HashAlg, PrivateKey};
use serde::{Deserialize, Serialize};

use super::keystore;
use super::SftpError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyInfo {
    /// このアプリ内で鍵を識別するラベル(例: "audiocafe.tokyo-root")
    pub label: String,
    pub algorithm: String,
    /// `ssh-ed25519 AAAA... label` 形式(authorized_keysに追記可能)
    pub public_key_openssh: String,
    pub fingerprint: String,
}

/// ed25519鍵ペアを新規生成し、秘密鍵をOSセキュアストレージへ保存する。
/// 呼び出し元(フロントエンド)には公開鍵とフィンガープリントのみを返す。
pub fn generate_keypair(label: &str) -> Result<KeyInfo, SftpError> {
    if label.trim().is_empty() {
        return Err(SftpError::InvalidInput("ラベルが空です".into()));
    }

    let key_pair = PrivateKey::random(&mut rand_ssh::rng(), Algorithm::Ed25519)
        .map_err(|e| SftpError::KeyGeneration(format!("ed25519鍵ペアの生成に失敗しました: {e}")))?;

    let private_pem = key_pair
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|e| SftpError::KeyGeneration(e.to_string()))?
        .to_string();

    let public_openssh = key_pair
        .public_key()
        .to_openssh()
        .map_err(|e| SftpError::KeyGeneration(e.to_string()))?;
    let public_key_openssh = format!("{public_openssh} {label}");
    // 既存のknown_hostsとの互換のため、旧版と同じ「SHA256:」接頭辞なしのbase64で保持する。
    let fingerprint = key_pair.public_key().fingerprint(HashAlg::Sha256).to_string();
    let fingerprint = fingerprint.strip_prefix("SHA256:").unwrap_or(&fingerprint).to_string();

    keystore::store_private_key(label, &private_pem)?;

    Ok(KeyInfo {
        label: label.to_string(),
        algorithm: "ed25519".to_string(),
        public_key_openssh,
        fingerprint,
    })
}

/// 保存済み秘密鍵を読み込み、SFTP接続に使う`KeyPair`を復元する。
pub fn load_keypair(label: &str) -> Result<PrivateKey, SftpError> {
    let pem = keystore::load_private_key(label)?;
    russh::keys::decode_secret_key(&pem, None).map_err(|e| SftpError::KeyStore(e.to_string()))
}

/// 秘密鍵を削除する(公開鍵の記録はフロント側/呼び出し元の責任で管理)。
pub fn delete_keypair(label: &str) -> Result<(), SftpError> {
    keystore::delete_private_key(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成した鍵をOpenSSH形式で書き出し、復元しても同じ公開鍵になること。
    /// フィンガープリントは旧版互換の「SHA256:」接頭辞なしで、43文字のbase64になること。
    #[test]
    fn generated_key_round_trips_and_fingerprint_keeps_the_legacy_format() {
        let key = PrivateKey::random(&mut rand_ssh::rng(), Algorithm::Ed25519).unwrap();
        let pem = key.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().to_string();
        let back = russh::keys::decode_secret_key(&pem, None).unwrap();
        assert_eq!(key.public_key(), back.public_key());
        let fp = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let fp = fp.strip_prefix("SHA256:").unwrap();
        assert_eq!(fp.len(), 43);
    }
}

//! OSセキュアストレージ(Windows Credential Manager / macOS Keychain /
//! Linux Secret Service / Android Keystore)への秘密鍵の保管。
//!
//! `keyring`クレートがプラットフォームごとのネイティブAPIを抽象化する。
//! 秘密鍵の値そのものは呼び出し元(コマンド層)からもディスクにも一切
//! 書き出さない — このモジュールを通じてのみ読み書きする。

use keyring::Entry;

use super::SftpError;

const SERVICE: &str = "webeditor-sftp";

fn entry(label: &str) -> Result<Entry, SftpError> {
    Entry::new(SERVICE, label).map_err(|e| SftpError::KeyStore(e.to_string()))
}

/// 秘密鍵(OpenSSH PEM形式の文字列)をOSセキュアストレージへ保存する。
pub fn store_private_key(label: &str, private_key_pem: &str) -> Result<(), SftpError> {
    entry(label)?
        .set_password(private_key_pem)
        .map_err(|e| SftpError::KeyStore(e.to_string()))
}

/// 秘密鍵をOSセキュアストレージから読み出す。
pub fn load_private_key(label: &str) -> Result<String, SftpError> {
    entry(label)?.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => SftpError::KeyStore(format!(
            "鍵ラベル「{label}」の秘密鍵がこの端末のセキュアストレージに見つかりません\
             (別の端末で生成した鍵は、この端末では使えません。鍵ペアを生成し直してください)"
        )),
        other => SftpError::KeyStore(other.to_string()),
    })
}

/// 秘密鍵をOSセキュアストレージから削除する。
pub fn delete_private_key(label: &str) -> Result<(), SftpError> {
    match entry(label)?.delete_credential() {
        Ok(()) => Ok(()),
        // 既に無い場合は削除成功と同義として扱う(冪等性)。
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(SftpError::KeyStore(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_verify_via_cmdkey() {
        let label = "diag-verify-key";
        store_private_key(label, "dummy-private-key-content").unwrap();

        let loaded = load_private_key(label).unwrap();
        assert_eq!(loaded, "dummy-private-key-content");

        let output = std::process::Command::new("cmdkey")
            .arg("/list")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        eprintln!("=== cmdkey /list output ===\n{stdout}");
        eprintln!("=== contains 'webeditor-sftp'? {} ===", stdout.contains("webeditor-sftp"));
        eprintln!("=== contains '{label}'? {} ===", stdout.contains(label));

        delete_private_key(label).unwrap();
    }
}

use ed25519_dalek::SigningKey;
use std::env;
use std::fmt;
use std::fs;

pub struct LocalSigner(SigningKey);

#[derive(Debug)]
pub enum WalletError {
    EnvVarMissing,
    ReadFailed(String),
    BadFormat(String),
}

impl fmt::Display for WalletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalletError::EnvVarMissing => write!(f, "SOLANA_KEYPAIR_PATH not set"),
            WalletError::ReadFailed(e) => write!(f, "failed to read keypair file: {e}"),
            WalletError::BadFormat(e) => write!(f, "malformed keypair file: {e}"),
        }
    }
}

impl LocalSigner {
    pub fn load() -> Result<Self, WalletError> {
        let path = env::var("SOLANA_KEYPAIR_PATH").map_err(|_| WalletError::EnvVarMissing)?;
        let raw = fs::read_to_string(&path).map_err(|e| WalletError::ReadFailed(e.to_string()))?;
        let bytes: Vec<u8> = serde_json::from_str(&raw)
            .map_err(|e| WalletError::BadFormat(format!("not a JSON byte array: {e}")))?;
        if bytes.len() != 64 {
            return Err(WalletError::BadFormat(format!(
                "expected 64 bytes (seed || pubkey), got {}",
                bytes.len()
            )));
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes[..32]);
        Ok(Self(SigningKey::from_bytes(&seed)))
    }

    pub fn pubkey_bytes(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    pub fn pubkey_base58(&self) -> String {
        bs58::encode(self.pubkey_bytes()).into_string()
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rejects_missing_env_var() {
        // SAFETY: test-only env manipulation, single-threaded test.
        unsafe { env::remove_var("SOLANA_KEYPAIR_PATH") };
        assert!(matches!(LocalSigner::load(), Err(WalletError::EnvVarMissing)));
    }

    #[test]
    fn loads_a_valid_keypair_file_and_derives_pubkey() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let mut bytes = signing_key.to_bytes().to_vec();
        bytes.extend_from_slice(&signing_key.verifying_key().to_bytes());

        let mut tmp = std::env::temp_dir();
        tmp.push("mimi_test_keypair.json");
        let mut f = fs::File::create(&tmp).unwrap();
        f.write_all(serde_json::to_string(&bytes).unwrap().as_bytes()).unwrap();

        // SAFETY: test-only env manipulation, single-threaded test.
        unsafe { env::set_var("SOLANA_KEYPAIR_PATH", tmp.to_str().unwrap()) };
        let signer = LocalSigner::load().expect("should load valid keypair");
        assert_eq!(signer.pubkey_bytes(), signing_key.verifying_key().to_bytes());
        assert!(!signer.pubkey_base58().is_empty());

        fs::remove_file(&tmp).ok();
        unsafe { env::remove_var("SOLANA_KEYPAIR_PATH") };
    }
}


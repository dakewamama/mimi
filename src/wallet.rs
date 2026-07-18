//! Local signer. SOLANA_KEYPAIR_PATH points to a keypair file on disk --
//! never raw key bytes in env, chat, or git. The file is a JSON array of
//! 64 bytes (seed || pubkey), parsed directly to avoid solana-sdk's OpenSSL.

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


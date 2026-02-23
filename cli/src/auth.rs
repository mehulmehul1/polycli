use std::str::FromStr;

use anyhow::{Context, Result};
use polymarket_client_sdk::auth::LocalSigner;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::auth::Signer as _;
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::clob::types::SignatureType;
use polymarket_client_sdk::{POLYGON, clob};

use crate::config;

fn parse_signature_type(s: &str) -> SignatureType {
    match s {
        "proxy" => SignatureType::Proxy,
        "gnosis-safe" => SignatureType::GnosisSafe,
        _ => SignatureType::Eoa,
    }
}

pub fn resolve_signer(
    private_key: Option<&str>,
) -> Result<impl polymarket_client_sdk::auth::Signer> {
    let (key, _source) = config::resolve_key(private_key);
    let key = key.ok_or_else(|| anyhow::anyhow!("{}", config::NO_WALLET_MSG))?;
    let signer = LocalSigner::from_str(&key)
        .context("Invalid private key")?
        .with_chain_id(Some(POLYGON));
    Ok(signer)
}

pub async fn authenticated_clob_client(
    private_key: Option<&str>,
    signature_type_flag: Option<&str>,
) -> Result<clob::Client<Authenticated<Normal>>> {
    let (key, _source) = config::resolve_key(private_key);
    let key = key.ok_or_else(|| anyhow::anyhow!("{}", config::NO_WALLET_MSG))?;

    let signer = LocalSigner::from_str(&key)
        .context("Invalid private key")?
        .with_chain_id(Some(POLYGON));

    let sig_type_str = config::resolve_signature_type(signature_type_flag);
    let sig_type = parse_signature_type(&sig_type_str);

    let client = clob::Client::default();
    let authenticated = client
        .authentication_builder(&signer)
        .signature_type(sig_type)
        .authenticate()
        .await
        .context("Failed to authenticate with Polymarket CLOB")?;

    Ok(authenticated)
}

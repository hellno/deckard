//! ERC-7730 clear-signing descriptor spike.
//!
//! The types here intentionally model only the small subset Deckard needs to prove the
//! consumption path: bind descriptor context first, then normalize user-facing intent + field rows.
//! A descriptor is display metadata, not a security oracle.

use std::collections::BTreeMap;
use std::str::FromStr;

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

/// Minimal ERC-7730 descriptor model. Unknown fields are ignored by serde so draft-version additions
/// do not make old Deckard builds fail closed before context binding and format validation run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Descriptor {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub context: Erc7730Context,
    #[serde(default)]
    pub metadata: Erc7730Metadata,
    pub display: Erc7730Display,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Context {
    pub contract: Option<Erc7730ContractContext>,
    /// Kept as a typed presence bit for the spike. Full EIP-712 message binding is future work.
    #[serde(default)]
    pub messages: BTreeMap<String, Erc7730MessageContext>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730ContractContext {
    #[serde(default)]
    pub deployments: Vec<Erc7730Deployment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Deployment {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub address: String,
}

/// Placeholder for the message-binding half of ERC-7730. The map is intentionally opaque in this
/// spike so unknown draft fields do not get rendered as trusted UI.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc7730MessageContext {}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Metadata {
    pub owner: Option<String>,
    #[serde(rename = "contractName")]
    pub contract_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Display {
    #[serde(default)]
    pub formats: BTreeMap<String, Erc7730Format>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Format {
    pub intent: String,
    #[serde(rename = "interpolatedIntent")]
    pub interpolated_intent: Option<String>,
    #[serde(default)]
    pub fields: Vec<Erc7730Field>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Erc7730Field {
    pub path: String,
    pub label: String,
    pub format: String,
    /// Minimal parameter support for common ERC-7730 descriptors such as `tokenPath`.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearSigningReview {
    pub intent: String,
    pub interpolated_intent: Option<String>,
    pub owner: Option<String>,
    pub contract_name: Option<String>,
    pub fields: Vec<ClearSigningField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearSigningField {
    pub path: String,
    pub label: String,
    pub format: ClearSigningFieldFormat,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClearSigningFieldFormat {
    Raw,
    AddressName,
    TokenAmount { token_path: Option<String> },
    Amount,
    Date,
    String,
    Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearSigningFallback {
    pub reason: ClearSigningError,
    pub warning: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClearSigningError {
    DescriptorMissing,
    DescriptorInvalid,
    UnsupportedMessageContext,
    ContextMismatch,
    FormatMissing,
    EmptyIntent,
    EmptyFieldLabel,
    EmptyFieldPath,
    UnsupportedFieldFormat(String),
}

/// Normalize an ERC-7730 contract-call descriptor only after binding it to the reviewed chain and
/// target contract. The `format_key` is the ERC-7730 display key, e.g.
/// `transfer(address to,uint256 value)`.
pub fn normalize_contract_call_descriptor(
    descriptor: &Erc7730Descriptor,
    chain_id: u64,
    verifying_contract: Address,
    format_key: &str,
) -> Result<ClearSigningReview, ClearSigningError> {
    let Some(contract) = &descriptor.context.contract else {
        return Err(ClearSigningError::UnsupportedMessageContext);
    };

    if !deployment_matches(contract, chain_id, verifying_contract) {
        return Err(ClearSigningError::ContextMismatch);
    }

    let Some(format) = descriptor.display.formats.get(format_key) else {
        return Err(ClearSigningError::FormatMissing);
    };

    if format.intent.trim().is_empty() {
        return Err(ClearSigningError::EmptyIntent);
    }

    let fields = format
        .fields
        .iter()
        .map(normalize_field)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ClearSigningReview {
        intent: format.intent.clone(),
        interpolated_intent: format.interpolated_intent.clone(),
        owner: descriptor.metadata.owner.clone(),
        contract_name: descriptor.metadata.contract_name.clone(),
        fields,
    })
}

pub fn clear_signing_fallback(reason: ClearSigningError) -> ClearSigningFallback {
    ClearSigningFallback {
        reason,
        warning:
            "Clear-signing descriptor unavailable or unsafe to apply; show blind-signing warning.",
    }
}

fn deployment_matches(
    contract: &Erc7730ContractContext,
    chain_id: u64,
    verifying_contract: Address,
) -> bool {
    contract.deployments.iter().any(|deployment| {
        if deployment.chain_id != chain_id {
            return false;
        }
        Address::from_str(&deployment.address)
            .map(|address| address == verifying_contract)
            .unwrap_or(false)
    })
}

fn normalize_field(field: &Erc7730Field) -> Result<ClearSigningField, ClearSigningError> {
    if field.label.trim().is_empty() {
        return Err(ClearSigningError::EmptyFieldLabel);
    }
    if field.path.trim().is_empty() {
        return Err(ClearSigningError::EmptyFieldPath);
    }

    Ok(ClearSigningField {
        path: field.path.clone(),
        label: field.label.clone(),
        format: normalize_field_format(field)?,
    })
}

fn normalize_field_format(
    field: &Erc7730Field,
) -> Result<ClearSigningFieldFormat, ClearSigningError> {
    match field.format.as_str() {
        "raw" => Ok(ClearSigningFieldFormat::Raw),
        "addressName" => Ok(ClearSigningFieldFormat::AddressName),
        "tokenAmount" => Ok(ClearSigningFieldFormat::TokenAmount {
            token_path: field.params.get("tokenPath").cloned(),
        }),
        "amount" => Ok(ClearSigningFieldFormat::Amount),
        "date" => Ok(ClearSigningFieldFormat::Date),
        "string" => Ok(ClearSigningFieldFormat::String),
        "bytes" => Ok(ClearSigningFieldFormat::Bytes),
        other => Err(ClearSigningError::UnsupportedFieldFormat(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USDT: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";

    fn descriptor() -> Erc7730Descriptor {
        serde_json::from_str(include_str!(
            "../tests/fixtures/erc7730-valid-transfer.json"
        ))
        .expect("valid fixture parses")
    }

    #[test]
    fn descriptor_present_normalizes_review_rows() {
        let contract = Address::from_str(USDT).expect("address parses");
        let review = normalize_contract_call_descriptor(
            &descriptor(),
            1,
            contract,
            "transfer(address to,uint256 value)",
        )
        .expect("descriptor applies");

        assert_eq!(review.intent, "Send");
        assert_eq!(review.owner.as_deref(), Some("Example"));
        assert_eq!(review.contract_name.as_deref(), Some("Example Token"));
        assert_eq!(review.fields.len(), 2);
        assert_eq!(
            review.fields,
            vec![
                ClearSigningField {
                    path: "value".into(),
                    label: "Amount".into(),
                    format: ClearSigningFieldFormat::TokenAmount {
                        token_path: Some("@.to".into()),
                    },
                },
                ClearSigningField {
                    path: "to".into(),
                    label: "To".into(),
                    format: ClearSigningFieldFormat::AddressName,
                },
            ]
        );
    }

    #[test]
    fn context_mismatch_falls_back_before_rendering() {
        let other_contract = Address::repeat_byte(0x11);
        let err = normalize_contract_call_descriptor(
            &descriptor(),
            1,
            other_contract,
            "transfer(address to,uint256 value)",
        )
        .expect_err("wrong contract must not render descriptor labels");

        assert_eq!(err, ClearSigningError::ContextMismatch);
        let fallback = clear_signing_fallback(err);
        assert_eq!(fallback.reason, ClearSigningError::ContextMismatch);
        assert!(fallback.warning.contains("blind-signing"));
    }

    #[test]
    fn invalid_descriptor_falls_back_to_blind_warning() {
        let parse_err = serde_json::from_str::<Erc7730Descriptor>(include_str!(
            "../tests/fixtures/erc7730-invalid-missing-display.json"
        ));
        assert!(parse_err.is_err());

        let fallback = clear_signing_fallback(ClearSigningError::DescriptorInvalid);
        assert_eq!(fallback.reason, ClearSigningError::DescriptorInvalid);
        assert!(fallback.warning.contains("unsafe to apply"));
    }

    #[test]
    fn unsupported_format_is_explicit_not_silent() {
        let mut descriptor = descriptor();
        let format = descriptor
            .display
            .formats
            .get_mut("transfer(address to,uint256 value)")
            .expect("format exists");
        let field = format.fields.first_mut().expect("field exists");
        field.format = "magicRiskHidingFormat".into();

        let contract = Address::from_str(USDT).expect("address parses");
        let err = normalize_contract_call_descriptor(
            &descriptor,
            1,
            contract,
            "transfer(address to,uint256 value)",
        )
        .expect_err("unsupported field format must fail closed");

        assert_eq!(
            err,
            ClearSigningError::UnsupportedFieldFormat("magicRiskHidingFormat".into())
        );
    }
}

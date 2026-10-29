//! Strict native-ZEC reader for Zkool 6.35.0's PCZT v2 wire format.
//! Schema pinned to zcash-shielded-assets/librustzcash fb3b4de5c304d2f07b0d8bf96f87f0c8337c68d5.
//! Only absent extensions and an explicit native-ZEC asset base are accepted; all
//! standard fields (including signatures, ciphertexts, derivations and expiry) are preserved.
//! The result is an inspection copy only. The original package must be used for signing.
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::collections::BTreeMap;

// A Some discriminant is rejected immediately, without guessing the unsupported payload schema.
fn absent<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<()>, D::Error> {
    match Option::<()>::deserialize(d)? {
        None => Ok(None),
        Some(()) => Err(serde::de::Error::custom("ZSA fields are unsupported")),
    }
}

// Fork AssetBase::zatoshi hashes the same personalization and b"v" as the ordinary Orchard
// value commitment base. Value=1 and trapdoor=0 returns precisely that canonical point encoding.
fn native_asset() -> [u8; 32] {
    use orchard::value::{NoteValue, ValueCommitTrapdoor, ValueCommitment};
    ValueCommitment::derive(
        NoteValue::from_raw(1) - NoteValue::ZERO,
        ValueCommitTrapdoor::from_bytes([0; 32]).unwrap(),
    )
    .to_bytes()
}
fn zec_asset<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
    let asset = Option::<[u8; 32]>::deserialize(d)?;
    if asset.is_none_or(|asset| asset == native_asset()) {
        Ok(asset)
    } else {
        Err(serde::de::Error::custom("non-ZEC assets are unsupported"))
    }
}

#[derive(Serialize, Deserialize)]
struct Wire {
    global: pczt::common::Global,
    transparent: Option<pczt::transparent::Bundle>,
    sapling: Option<pczt::sapling::Bundle>,
    orchard: Option<Bundle>,
    ironwood: Option<Bundle>,
    #[serde(deserialize_with = "absent", skip_serializing)]
    _issue: Option<()>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Zip32Derivation {
    seed_fingerprint: [u8; 32],
    derivation_path: Vec<u32>,
}

/// Decode exactly the released Zkool v2 schema, rejecting unknown extensions and trailing bytes.
pub fn inspection_pczt(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    if bytes.get(..8) != Some(b"PCZT\x02\x00\x00\x00") || bytes.len() > 8 * 1024 * 1024 {
        return Err(Error::Pczt("unsupported Zkool PCZT encoding".into()));
    }
    let mut deserializer = postcard::Deserializer::from_bytes(&bytes[8..]);
    let wire: Wire = serde_path_to_error::deserialize(&mut deserializer).map_err(|e| {
        Error::Pczt(format!(
            "Zkool native-ZEC schema at {}: {}",
            e.path(),
            e.inner()
        ))
    })?;
    let remaining = deserializer
        .finalize()
        .map_err(|e| Error::Pczt(e.to_string()))?;
    if !remaining.is_empty() {
        return Err(Error::Pczt("trailing Zkool PCZT data".into()));
    }
    let mut out = bytes[..8].to_vec();
    out.extend(postcard::to_allocvec(&wire).map_err(|e| Error::Pczt(e.to_string()))?);
    // Require the standard library to validate the normalized inspection copy as well.
    pczt::Pczt::parse(&out).map_err(|e| Error::Pczt(format!("Zkool inspection copy: {e:?}")))?;
    Ok(out)
}

/// A serializable representation of Orchard note plaintext versions.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum SerializedNoteVersion {
    V2,
    V3,
}

/// PCZT fields that are specific to producing the transaction's Orchard bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Bundle {
    actions: Vec<Action>,
    flags: u8,
    value_sum: (u64, bool),
    anchor: Option<[u8; 32]>,
    note_version: SerializedNoteVersion,
    zkproof: Option<Vec<u8>>,
    bsk: Option<pczt::common::SecretKeyBytes>,
}

/// Information about an Orchard action within a transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Action {
    cv_net: Option<[u8; 32]>,
    spend: Spend,
    output: Output,
    rcv: Option<[u8; 32]>,
}

/// Information about the spend part of an Orchard action.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Spend {
    #[serde_as(as = "Option<[_; 32]>")]
    nullifier: Option<[u8; 32]>,
    #[serde_as(as = "Option<[_; 32]>")]
    rk: Option<[u8; 32]>,
    #[serde_as(as = "Option<[_; 64]>")]
    spend_auth_sig: Option<[u8; 64]>,
    #[serde_as(as = "Option<[_; 43]>")]
    recipient: Option<[u8; 43]>,
    value: Option<u64>,
    rho: Option<[u8; 32]>,
    rseed: Option<[u8; 32]>,
    #[serde(deserialize_with = "absent", skip_serializing)]
    _rseed_split_note: Option<()>,
    #[serde_as(as = "Option<[_; 96]>")]
    fvk: Option<[u8; 96]>,
    witness: Option<(u32, [[u8; 32]; 32])>,
    alpha: Option<[u8; 32]>,
    zip32_derivation: Option<Zip32Derivation>,
    dummy_sk: Option<pczt::common::SecretKeyBytes>,
    proprietary: BTreeMap<String, Vec<u8>>,
    #[serde(deserialize_with = "zec_asset", skip_serializing)]
    _asset: Option<[u8; 32]>,
}

/// Information about the output part of an Orchard action.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Output {
    cmx: Option<[u8; 32]>,
    ephemeral_key: [u8; 32],
    enc_ciphertext: pczt::orchard::EncCiphertext,
    out_ciphertext: Vec<u8>,
    #[serde_as(as = "Option<[_; 43]>")]
    recipient: Option<[u8; 43]>,
    value: Option<u64>,
    rseed: Option<[u8; 32]>,
    ock: Option<[u8; 32]>,
    zip32_derivation: Option<Zip32Derivation>,
    user_address: Option<String>,
    proprietary: BTreeMap<String, Vec<u8>>,
    #[serde(deserialize_with = "zec_asset", skip_serializing)]
    _asset: Option<[u8; 32]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    const STANDARD: &[u8] = include_bytes!("../tests/fixtures/payment-unsigned.pczt");

    fn sample() -> Wire {
        let pczt = pczt::Pczt::parse(STANDARD).unwrap();
        let v2 = pczt::v2::Pczt::try_from(pczt).unwrap();
        let mut value = serde_json::to_value(v2).unwrap();
        value["_issue"] = serde_json::Value::Null;
        for pool in ["orchard", "ironwood"] {
            if let Some(actions) = value
                .get_mut(pool)
                .and_then(|b| b.get_mut("actions"))
                .and_then(serde_json::Value::as_array_mut)
            {
                for action in actions {
                    action["spend"]["_rseed_split_note"] = serde_json::Value::Null;
                    action["spend"]["_asset"] = serde_json::Value::Null;
                    action["output"]["_asset"] = serde_json::Value::Null;
                }
            }
        }
        serde_json::from_value(value).unwrap()
    }

    // Independent fork encoder follows the published positional schema. Every standard
    // substructure is encoded by serde; fork markers are inserted at their declared positions.
    fn push<T: Serialize>(out: &mut Vec<u8>, value: &T) {
        out.extend(postcard::to_allocvec(value).unwrap());
    }
    #[serde_as]
    #[derive(Serialize)]
    struct Prefix<'a> {
        nullifier: &'a Option<[u8; 32]>,
        rk: &'a Option<[u8; 32]>,
        #[serde_as(as = "&Option<[_;64]>")]
        sig: &'a Option<[u8; 64]>,
        #[serde_as(as = "&Option<[_;43]>")]
        recipient: &'a Option<[u8; 43]>,
        value: &'a Option<u64>,
        rho: &'a Option<[u8; 32]>,
        rseed: &'a Option<[u8; 32]>,
    }
    fn fork(wire: &Wire, bad: Option<(usize, usize)>) -> Vec<u8> {
        let mut out = b"PCZT\x02\0\0\0".to_vec();
        push(&mut out, &wire.global);
        push(&mut out, &wire.transparent);
        push(&mut out, &wire.sapling);
        for (pool, bundle) in [&wire.orchard, &wire.ironwood].into_iter().enumerate() {
            let Some(bundle) = bundle else {
                out.push(0);
                continue;
            };
            out.push(1);
            push(&mut out, &bundle.actions.len());
            for action in &bundle.actions {
                push(&mut out, &action.cv_net);
                let s = &action.spend;
                let prefix = postcard::to_allocvec(&Prefix {
                    nullifier: &s.nullifier,
                    rk: &s.rk,
                    sig: &s.spend_auth_sig,
                    recipient: &s.recipient,
                    value: &s.value,
                    rho: &s.rho,
                    rseed: &s.rseed,
                })
                .unwrap();
                let spend = postcard::to_allocvec(s).unwrap();
                assert!(spend.starts_with(&prefix));
                out.extend(prefix);
                out.push(u8::from(bad == Some((pool, 0))));
                out.extend(&spend[out_field_prefix(s)..]);
                push(
                    &mut out,
                    &if bad == Some((pool, 1)) {
                        Some([0u8; 32])
                    } else {
                        s._asset
                    },
                );
                push(&mut out, &action.output);
                push(
                    &mut out,
                    &if bad == Some((pool, 2)) {
                        Some([0u8; 32])
                    } else {
                        action.output._asset
                    },
                );
                push(&mut out, &action.rcv);
            }
            push(&mut out, &bundle.flags);
            push(&mut out, &bundle.value_sum);
            push(&mut out, &bundle.anchor);
            if bad == Some((pool, 3)) {
                out.push(2);
            } else {
                push(&mut out, &bundle.note_version);
            }
            push(&mut out, &bundle.zkproof);
            push(&mut out, &bundle.bsk);
        }
        out.push(u8::from(bad == Some((2, 0))));
        out
    }
    fn out_field_prefix(s: &Spend) -> usize {
        postcard::to_allocvec(&Prefix {
            nullifier: &s.nullifier,
            rk: &s.rk,
            sig: &s.spend_auth_sig,
            recipient: &s.recipient,
            value: &s.value,
            rho: &s.rho,
            rseed: &s.rseed,
        })
        .unwrap()
        .len()
    }

    #[test]
    fn native_fork_encoding_preserves_every_standard_byte_and_payment() {
        let wire = sample();
        let encoded = fork(&wire, None);
        let normalized = inspection_pczt(&encoded).unwrap();
        let canonical = pczt::Pczt::parse(STANDARD).unwrap().serialize().unwrap();
        assert_eq!(normalized, canonical);
        assert_eq!(
            crate::summary(&normalized).unwrap(),
            crate::summary(STANDARD).unwrap()
        );
        let ufvk = include_str!("../tests/fixtures/payee.ufvk").trim();
        assert_eq!(
            crate::inspect(&normalized, ufvk, crate::Network::Testnet).unwrap(),
            crate::inspect(STANDARD, ufvk, crate::Network::Testnet).unwrap()
        );
        let redacted = crate::redact_for_payee(&normalized).unwrap();
        assert_eq!(
            crate::summary(&redacted).unwrap(),
            crate::summary(STANDARD).unwrap()
        );
        assert_eq!(
            crate::inspect(&redacted, ufvk, crate::Network::Testnet).unwrap(),
            crate::inspect(STANDARD, ufvk, crate::Network::Testnet).unwrap()
        );
        assert_eq!(
            encoded,
            include_bytes!("../tests/fixtures/payment-zkool-unsigned.pczt")
        );
    }

    #[test]
    fn every_fork_extension_and_zsa_note_version_is_rejected_in_both_pools() {
        let mut wire = sample();
        // Exercise both layouts even when the reference payment funds from only one pool.
        let populated = wire
            .ironwood
            .as_ref()
            .or(wire.orchard.as_ref())
            .unwrap()
            .clone();
        wire.orchard = Some(populated.clone());
        wire.ironwood = Some(populated);
        assert!(inspection_pczt(&fork(&wire, None)).is_ok());
        for pool in 0..2 {
            for field in 0..4 {
                assert!(
                    inspection_pczt(&fork(&wire, Some((pool, field)))).is_err(),
                    "pool {pool} extension {field}"
                );
            }
        }
        assert!(inspection_pczt(&fork(&wire, Some((2, 0)))).is_err());
    }

    #[test]
    fn explicit_native_zec_assets_preserve_effects_in_both_pools() {
        // Independent compressed point from orchard 0.16's published value_commit_v GENERATOR:
        // x in little endian, with the even y sign bit clear.
        assert_eq!(
            hex::encode(native_asset()),
            "6743f93a6ebda72a8c7c5a2b7fa304fe32b29b4f706aa8f7420f3d8e7a59702f"
        );
        let mut wire = sample();
        let original = inspection_pczt(&fork(&wire, None)).unwrap();
        for bundle in [&mut wire.orchard, &mut wire.ironwood]
            .into_iter()
            .flatten()
        {
            for action in &mut bundle.actions {
                action.spend._asset = Some(native_asset());
                action.output._asset = Some(native_asset());
            }
        }
        let normalized = inspection_pczt(&fork(&wire, None)).unwrap();
        assert_eq!(normalized, original);
        let ufvk = include_str!("../tests/fixtures/payee.ufvk").trim();
        let expected = crate::inspect(&original, ufvk, crate::Network::Testnet).unwrap();
        assert_eq!(
            crate::inspect(&normalized, ufvk, crate::Network::Testnet).unwrap(),
            expected
        );
        assert_eq!(
            crate::inspect(
                &crate::redact_for_payee(&normalized).unwrap(),
                ufvk,
                crate::Network::Testnet
            )
            .unwrap(),
            expected
        );
        let populated = sample().ironwood.unwrap();
        wire.orchard = Some(populated.clone());
        wire.ironwood = Some(populated);
        let before = inspection_pczt(&fork(&wire, None)).unwrap();
        for pool in 0..2 {
            let count = if pool == 0 {
                wire.orchard.as_ref()
            } else {
                wire.ironwood.as_ref()
            }
            .unwrap()
            .actions
            .len();
            for index in 0..count {
                for field in 0..2 {
                    let action = &mut if pool == 0 {
                        wire.orchard.as_mut()
                    } else {
                        wire.ironwood.as_mut()
                    }
                    .unwrap()
                    .actions[index];
                    if field == 0 {
                        action.spend._asset = Some(native_asset());
                    } else {
                        action.output._asset = Some(native_asset());
                    }
                    assert_eq!(inspection_pczt(&fork(&wire, None)).unwrap(), before);
                }
            }
        }
    }

    #[test]
    fn wrong_versions_truncation_and_trailing_data_are_rejected() {
        let valid = fork(&sample(), None);
        for len in [0, 4, 8, valid.len() / 2, valid.len() - 1] {
            assert!(inspection_pczt(&valid[..len]).is_err());
        }
        let mut changed = valid.clone();
        changed.push(0);
        assert!(inspection_pczt(&changed).is_err());
        let mut changed = valid;
        changed[4] = 1;
        assert!(inspection_pczt(&changed).is_err());
    }
}

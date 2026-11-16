use super::*;

fn fixture() -> Vec<u8> {
    (0..20_000).map(|n| (n % 251) as u8).collect()
}
fn refresh_encoded(envelope: &mut Envelope) {
    envelope.encoded_bytes = envelope.data.len();
    envelope.encoded_crc32 = crc32(&envelope.data);
}

#[test]
fn builtin_codecs_roundtrip_empty_binary_and_large_compressible_data() {
    let registry = Registry::default();
    for id in [LZ4, ZLIB] {
        for raw in [
            Vec::new(),
            vec![0, 255, 27, 128, 10],
            fixture(),
            vec![b'x'; 9 << 20],
        ] {
            let envelope = registry.encode(id, &raw, Limits::default()).unwrap();
            assert_eq!(envelope.codec, id);
            assert!(envelope.data.len() <= Limits::default().max_encoded_bytes);
            assert_eq!(registry.decode(&envelope, Limits::default()).unwrap(), raw);
            let binary = envelope.to_bytes(Limits::default()).unwrap();
            let restored = Envelope::from_bytes(&binary, Limits::default()).unwrap();
            assert_eq!(restored, envelope);
            assert_eq!(registry.decode(&restored, Limits::default()).unwrap(), raw);
        }
    }
}

#[test]
fn encoded_and_decoded_checksums_detect_distinct_damage() {
    let registry = Registry::default();
    for id in [LZ4, ZLIB] {
        let mut envelope = registry.encode(id, &fixture(), Limits::default()).unwrap();
        envelope.data[0] ^= 255;
        assert!(matches!(
            registry.decode(&envelope, Limits::default()),
            Err(Error::Checksum {
                what: "encoded",
                ..
            })
        ));
        let mut envelope = registry.encode(id, &fixture(), Limits::default()).unwrap();
        envelope.raw_crc32 ^= 1;
        assert!(matches!(
            registry.decode(&envelope, Limits::default()),
            Err(Error::Checksum {
                what: "decoded",
                ..
            })
        ));
    }
}

#[test]
fn truncation_trailing_data_and_recomputed_corrupt_blocks_still_fail() {
    let registry = Registry::default();
    for id in [LZ4, ZLIB] {
        let original = registry.encode(id, &fixture(), Limits::default()).unwrap();
        let mut truncated = original.clone();
        truncated.data.pop();
        refresh_encoded(&mut truncated);
        assert!(registry.decode(&truncated, Limits::default()).is_err());
        let mut trailing = original.clone();
        trailing.data.extend_from_slice(b"unexpected");
        refresh_encoded(&mut trailing);
        assert!(registry.decode(&trailing, Limits::default()).is_err());
        let mut damaged = original;
        let mid = damaged.data.len() / 2;
        damaged.data[mid] ^= 255;
        refresh_encoded(&mut damaged);
        assert!(registry.decode(&damaged, Limits::default()).is_err());
    }
}

#[test]
fn claims_and_expansion_are_rejected_with_bounded_output() {
    let registry = Registry::default();
    for id in [LZ4, ZLIB] {
        let mut envelope = registry
            .encode(id, &vec![b'x'; 1 << 20], Limits::default())
            .unwrap();
        envelope.decoded_bytes = usize::MAX;
        assert!(matches!(
            registry.decode(&envelope, Limits::default()),
            Err(Error::Limit {
                what: "decoded",
                ..
            })
        ));
        envelope.decoded_bytes = 1;
        assert!(registry.decode(&envelope, Limits::default()).is_err());
        let envelope = registry.encode(id, b"payload", Limits::default()).unwrap();
        assert!(matches!(
            registry.decode(
                &envelope,
                Limits {
                    max_decoded_bytes: 6,
                    ..Limits::default()
                }
            ),
            Err(Error::Limit {
                what: "decoded",
                ..
            })
        ));
        assert!(matches!(
            registry.encode(
                id,
                b"payload",
                Limits {
                    max_encoded_bytes: 1,
                    ..Limits::default()
                }
            ),
            Err(Error::Limit {
                what: "encoded",
                ..
            })
        ));
    }
}

#[test]
fn binary_lengths_are_checked_before_allocating_and_trailing_bytes_are_rejected() {
    let registry = Registry::default();
    let original = registry
        .encode(LZ4, b"payload", Limits::default())
        .unwrap()
        .to_bytes(Limits::default())
        .unwrap();
    for range in [10..18, 18..26] {
        let mut bytes = original.clone();
        bytes[range].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            Envelope::from_bytes(&bytes, Limits::default()),
            Err(Error::Limit { .. })
        ));
    }
    for size in [0, 7, 33, original.len() - 1] {
        assert!(Envelope::from_bytes(&original[..size], Limits::default()).is_err());
    }
    let mut trailing = original.clone();
    trailing.push(0);
    assert!(matches!(
        Envelope::from_bytes(&trailing, Limits::default()),
        Err(Error::Progress {
            what: "binary envelope length",
            ..
        })
    ));
    let mut corrupt = original;
    *corrupt.last_mut().unwrap() ^= 255;
    assert!(matches!(
        Envelope::from_bytes(&corrupt, Limits::default()),
        Err(Error::Checksum {
            what: "encoded",
            ..
        })
    ));
}

#[test]
fn serde_requires_all_integrity_fields_and_rejects_unknown_envelope_fields() {
    let envelope = Registry::default()
        .encode(LZ4, b"state", Limits::default())
        .unwrap();
    let value = serde_json::to_value(&envelope).unwrap();
    for field in [
        "version",
        "codec",
        "decoded_bytes",
        "encoded_bytes",
        "raw_crc32",
        "encoded_crc32",
        "data",
    ] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Envelope>(missing).is_err());
    }
    let mut extra = value;
    extra["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Envelope>(extra).is_err());
}

#[test]
fn legacy_zlib_requires_complete_stream_checksum_and_exact_length() {
    let registry = Registry::default();
    let raw = fixture();
    let envelope = registry.encode(ZLIB, &raw, Limits::default()).unwrap();
    assert_eq!(
        registry
            .decode_legacy_zlib(&envelope.data, raw.len(), Limits::default())
            .unwrap(),
        raw
    );
    assert!(
        registry
            .decode_legacy_zlib(
                &envelope.data[..envelope.data.len() - 1],
                raw.len(),
                Limits::default()
            )
            .is_err()
    );
    let mut trailing = envelope.data;
    trailing.push(0);
    assert!(matches!(
        registry.decode_legacy_zlib(&trailing, raw.len(), Limits::default()),
        Err(Error::Progress {
            what: "consumed input",
            ..
        })
    ));
}

struct Identity {
    id: &'static str,
    short_consume: bool,
    huge_hint: bool,
}
impl Codec for Identity {
    fn id(&self) -> &str {
        self.id
    }
    fn max_encoded_len(&self, len: usize) -> Result<usize, Error> {
        Ok(if self.huge_hint { usize::MAX } else { len })
    }
    fn encode_into(&self, raw: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        output[..raw.len()].copy_from_slice(raw);
        Ok(raw.len())
    }
    fn decode_into(&self, data: &[u8], output: &mut [u8]) -> Result<Progress, Error> {
        output[..data.len()].copy_from_slice(data);
        Ok(Progress {
            consumed: data.len() - usize::from(self.short_consume),
            written: data.len(),
        })
    }
}

#[test]
fn registered_codecs_are_extensible_bounded_and_cannot_silently_replace_decoders() {
    let mut registry = Registry::default();
    registry
        .register(Box::new(Identity {
            id: "identity-v1",
            short_consume: false,
            huge_hint: false,
        }))
        .unwrap();
    let envelope = registry
        .encode("identity-v1", b"plugin", Limits::default())
        .unwrap();
    assert_eq!(
        registry.decode(&envelope, Limits::default()).unwrap(),
        b"plugin"
    );
    assert!(matches!(
        registry.register(Box::new(Identity {
            id: LZ4,
            short_consume: false,
            huge_hint: false
        })),
        Err(Error::DuplicateCodec { .. })
    ));
    assert_eq!(
        registry
            .decode(
                &registry.encode(LZ4, b"builtin", Limits::default()).unwrap(),
                Limits::default()
            )
            .unwrap(),
        b"builtin"
    );
    registry
        .register(Box::new(Identity {
            id: "bad-consumption-v1",
            short_consume: true,
            huge_hint: false,
        }))
        .unwrap();
    let envelope = registry
        .encode("bad-consumption-v1", b"plugin", Limits::default())
        .unwrap();
    assert!(matches!(
        registry.decode(&envelope, Limits::default()),
        Err(Error::Progress {
            what: "consumed input",
            ..
        })
    ));
    registry
        .register(Box::new(Identity {
            id: "huge-hint-v1",
            short_consume: false,
            huge_hint: true,
        }))
        .unwrap();
    assert!(matches!(
        registry.encode("huge-hint-v1", b"plugin", Limits::default()),
        Err(Error::Limit {
            what: "encoder working buffer",
            ..
        })
    ));
    assert!(matches!(
        registry.encode("unknown-v1", b"plugin", Limits::default()),
        Err(Error::UnknownCodec { .. })
    ));
}

#[test]
fn caller_limits_can_include_a_small_archive_metadata_allowance() {
    let registry = Registry::default();
    let raw = vec![0; (16 << 20) + 16 * 1024];
    assert!(matches!(
        registry.encode(LZ4, &raw, Limits::default()),
        Err(Error::Limit {
            what: "decoded",
            ..
        })
    ));
    let limits = Limits {
        max_decoded_bytes: raw.len(),
        ..Limits::default()
    };
    let envelope = registry.encode(LZ4, &raw, limits).unwrap();
    assert_eq!(registry.decode(&envelope, limits).unwrap().len(), raw.len());
}

struct PreferenceCodec {
    id: &'static str,
    kind: u8,
}
impl Codec for PreferenceCodec {
    fn id(&self) -> &str {
        self.id
    }
    fn max_encoded_len(&self, _: usize) -> Result<usize, Error> {
        Ok(64)
    }
    fn encode_into(&self, _: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        match self.kind {
            0 => {
                output[..64].fill(0);
                Ok(64)
            }
            1 => Err(Error::Plugin {
                codec: self.id.into(),
                operation: "encode",
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "owned plugin failure",
                ),
            }),
            _ => Err(Error::Limit {
                what: "encoded",
                actual: 64,
                limit: 32,
            }),
        }
    }
    fn decode_into(&self, _: &[u8], _: &mut [u8]) -> Result<Progress, Error> {
        Err(Error::Header {
            reason: "not a decoder",
        })
    }
}

#[test]
fn explicit_preferences_fall_back_only_for_observed_encoded_budget_overflow() {
    let mut registry = Registry::default();
    registry
        .register(Box::new(PreferenceCodec {
            id: "too-large-v1",
            kind: 0,
        }))
        .unwrap();
    registry
        .register(Box::new(PreferenceCodec {
            id: "failure-v1",
            kind: 1,
        }))
        .unwrap();
    registry
        .register(Box::new(PreferenceCodec {
            id: "spoof-budget-v1",
            kind: 2,
        }))
        .unwrap();
    registry
        .register(Box::new(Identity {
            id: "fits-v1",
            short_consume: false,
            huge_hint: false,
        }))
        .unwrap();
    let limits = Limits {
        max_encoded_bytes: 32,
        ..Limits::default()
    };
    let envelope = registry
        .encode_preferred(&["too-large-v1", "fits-v1"], b"data", limits)
        .unwrap();
    assert_eq!(envelope.codec, "fits-v1");
    assert_eq!(registry.decode(&envelope, limits).unwrap(), b"data");
    assert!(matches!(
        registry.encode("too-large-v1", b"data", limits),
        Err(Error::Limit {
            what: "encoded",
            ..
        })
    ));
    assert!(matches!(
        registry.encode_preferred(&["failure-v1", "fits-v1"], b"data", limits),
        Err(Error::Plugin { .. })
    ));
    assert!(matches!(
        registry.encode_preferred(&["spoof-budget-v1", "fits-v1"], b"data", limits),
        Err(Error::Limit {
            what: "encoded",
            ..
        })
    ));
    assert!(matches!(
        registry.encode_preferred(&["unknown-v1", "fits-v1"], b"data", limits),
        Err(Error::UnknownCodec { .. })
    ));
    assert!(matches!(
        registry.encode_preferred(&[], b"data", limits),
        Err(Error::NoPreference)
    ));
    assert!(matches!(
        registry.encode_preferred(&["too-large-v1"], b"data", limits),
        Err(Error::Limit {
            what: "encoded",
            ..
        })
    ));
}

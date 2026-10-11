use super::*;

#[test]
fn comments_terminators_and_padding_are_charged_to_work() {
	let input = b"// comment\n{k v}\0 \t\0";
	let exact = EntityLimits {
		work: input.len(),
		..EntityLimits::default()
	};
	assert!(Entities::parse(input, exact).is_ok());
	let short = EntityLimits {
		work: input.len() - 1,
		..exact
	};
	assert!(matches!(
		Entities::parse(input, short),
		Err(EntityError::Budget {
			resource: EntityResource::Work,
			..
		})
	));
}

#[test]
fn comments_words_multiline_quotes_and_empty_records_have_explicit_grammar() {
	let input = b"// first\n{ classname sky_camera // middle\norigin \"1\n2 3\" path maps/a//b empty \"\" }\r\n{}\0 \t\0";
	let entities = parse(input);
	assert_eq!(entities.len(), 2);
	assert_eq!(entities.pair_count(), 4);
	let first = entities.records().next().unwrap();
	assert_eq!(
		first.singleton_ascii(b"origin").unwrap(),
		Some(&b"1\n2 3"[..])
	);
	assert_eq!(
		first.singleton_ascii(b"path").unwrap(),
		Some(&b"maps/a//b"[..])
	);
	assert_eq!(first.singleton_ascii(b"empty").unwrap(), Some(&b""[..]));
	assert_eq!(entities.records().next_back().unwrap().pairs().len(), 0);
	assert!(parse(b" \n// only a comment").is_empty());
	assert!(parse(b"\0 \n\0").is_empty());
	assert!(parse(b"").is_empty());
}

#[test]
fn each_budget_is_enforced_without_truncating_payload_or_tokens() {
	for (input, limits, resource) in [
		(
			&b"{}"[..],
			EntityLimits {
				input_bytes: 1,
				..EntityLimits::default()
			},
			EntityResource::InputBytes,
		),
		(
			&b"{}{}"[..],
			EntityLimits {
				records: 1,
				..EntityLimits::default()
			},
			EntityResource::Records,
		),
		(
			&b"{a b c d}"[..],
			EntityLimits {
				pairs: 1,
				..EntityLimits::default()
			},
			EntityResource::Pairs,
		),
		(
			&b"{abcd x}"[..],
			EntityLimits {
				token_bytes: 3,
				..EntityLimits::default()
			},
			EntityResource::TokenBytes,
		),
		(
			&b"{x \"abcd\"}"[..],
			EntityLimits {
				token_bytes: 3,
				..EntityLimits::default()
			},
			EntityResource::TokenBytes,
		),
		(
			&b"{}"[..],
			EntityLimits {
				work: 1,
				..EntityLimits::default()
			},
			EntityResource::Work,
		),
	] {
		let error = Entities::parse(input, limits).unwrap_err();
		assert!(
			matches!(error, EntityError::Budget { resource: found, .. } if found == resource),
			"{error}"
		);
	}
	assert!(
		Entities::parse(
			b"{}",
			EntityLimits {
				input_bytes: 2,
				records: 1,
				pairs: 0,
				token_bytes: 0,
				work: 2
			}
		)
		.is_ok()
	);
	assert!(
		Entities::parse(
			b"{\"\" \"\"}",
			EntityLimits {
				token_bytes: 0,
				..EntityLimits::default()
			}
		)
		.is_ok()
	);
	assert!(
		Entities::parse(
			b"{}",
			EntityLimits {
				records: 0,
				..EntityLimits::default()
			}
		)
		.is_err()
	);
	assert!(
		Entities::parse(
			b"",
			EntityLimits {
				input_bytes: 0,
				records: 0,
				pairs: 0,
				token_bytes: 0,
				work: 0
			}
		)
		.is_ok()
	);
}

#[test]
fn grammar_errors_do_not_accept_a_partial_record_list() {
	for (input, expected) in [
		(&b"classname worldspawn"[..], EntitySyntax::ExpectedEntity),
		(&b"}"[..], EntitySyntax::UnexpectedClose),
		(&b"{{}}"[..], EntitySyntax::NestedEntity),
		(&b"{key {}}"[..], EntitySyntax::NestedEntity),
		(&b"{key}"[..], EntitySyntax::MissingValue),
		(&b"{key"[..], EntitySyntax::MissingValue),
		(&b"{} {"[..], EntitySyntax::UnterminatedEntity),
		(&b"{\"key\" \"value"[..], EntitySyntax::UnterminatedQuote),
		(&b"{}\0{}"[..], EntitySyntax::InteriorNul),
		(&b"{}\0\x1b"[..], EntitySyntax::InteriorNul),
		(&b"{}\0\x07"[..], EntitySyntax::InteriorNul),
		(&b"{a \"x\0y\"}"[..], EntitySyntax::InteriorNul),
		(&b"// comment\0{}"[..], EntitySyntax::InteriorNul),
	] {
		let error = Entities::parse(input, EntityLimits::default()).unwrap_err();
		assert!(
			matches!(error, EntityError::Syntax { kind, .. } if kind == expected),
			"{input:?}: {error}"
		);
	}
	let error = Entities::parse(b"{}\0{}", EntityLimits::default()).unwrap_err();
	assert_eq!(
		error,
		EntityError::Syntax {
			offset: 3,
			kind: EntitySyntax::InteriorNul
		}
	);
}

#[test]
fn owned_snapshots_can_be_shared_without_native_lifetimes() {
	fn shared<T: Send + Sync>() {}
	shared::<Entities>();
	let source = parse(b"{classname sky_camera}{classname info_player_teamspawn}");
	let copy = source.clone();
	drop(source);
	assert_eq!(
		copy.records().map(Record::index).collect::<Vec<_>>(),
		[0, 1]
	);
	assert_eq!(
		copy.records()
			.next_back()
			.unwrap()
			.singleton_ascii(b"classname")
			.unwrap(),
		Some(&b"info_player_teamspawn"[..])
	);
}

fn parse(bytes: &[u8]) -> Entities {
	Entities::parse(bytes, EntityLimits::default()).unwrap()
}

/// This test consumes the exact independently extracted entity bytes; the
/// external gate verifies their SHA256 dcc25213031843b827cfe7d7b8da069b3224554960e86e791ca2434120de1a37
/// and the source BSP pin before supplying an immutable local path.
#[test]
#[ignore = "requires the explicitly supplied, hash-verified 2Fort entity payload"]
fn pinned_2fort_entity_records() {
	let path = std::env::var_os("SOURCE_BSP_2FORT_ENTITIES")
		.expect("set SOURCE_BSP_2FORT_ENTITIES to the verified local payload");
	let bytes = std::fs::read(path).unwrap();
	assert_eq!(bytes.len(), 314_566);
	let entities = parse(&bytes);
	assert_eq!(entities.len(), 1_097);
	assert_eq!(entities.pair_count(), 14_151);
	let class_count = |class: &[u8]| {
		entities
			.records()
			.filter(|record| record.singleton_ascii(b"classname").unwrap() == Some(class))
			.count()
	};
	assert_eq!(class_count(b"sky_camera"), 1);
	assert_eq!(class_count(b"info_player_teamspawn"), 30);
	assert_eq!(class_count(b"func_respawnroom"), 6);
	assert_eq!(class_count(b"func_areaportal"), 29);
	let room = entities.record(850).unwrap();
	assert_eq!(
		room.singleton_ascii(b"classname").unwrap(),
		Some(&b"func_respawnroom"[..])
	);
	assert_eq!(room.singleton_ascii(b"model").unwrap(), Some(&b"*76"[..]));
	assert_eq!(
		room.singleton_ascii(b"hammerid").unwrap(),
		Some(&b"3714797"[..])
	);
}

#[test]
fn punctuation_tokens_are_data_except_for_record_braces() {
	let entities = parse(b"{a ( b ) c '}");
	let values = entities
		.record(0)
		.unwrap()
		.pairs()
		.map(|pair| pair.value)
		.collect::<Vec<_>>();
	assert_eq!(values, [&b"("[..], &b")"[..], &b"'"[..]]);
}

#[test]
fn quoted_tokens_do_not_implement_json_escapes() {
	let entities = parse(br#"{ key "a\nb\tc" other "x\" }"#);
	let record = entities.record(0).unwrap();
	assert_eq!(
		record.singleton_ascii(b"key").unwrap(),
		Some(&br"a\nb\tc"[..])
	);
	assert_eq!(record.singleton_ascii(b"other").unwrap(), Some(&b"x\\"[..]));
}

#[test]
fn records_preserve_duplicates_arbitrary_bytes_and_literal_backslashes() {
	let input = b"{\"classname\" \"logic_relay\" \"OnTrigger\" \"a\x1bTrigger\x1b\x1b0\x1b-1\" \"OnTrigger\" \"b,Trigger,,0,-1\" \"note\" \"{ // literal \\\\n \xff }\"}\0\0";
	let entities = parse(input);
	assert_eq!(entities.payload(), input);
	assert_eq!(entities.len(), 1);
	assert_eq!(entities.pair_count(), 4);
	let record = entities.record(0).unwrap();
	assert_eq!(
		record.singleton_ascii(b"CLASSNAME").unwrap(),
		Some(&b"logic_relay"[..])
	);
	assert_eq!(
		record.values_ascii(b"ontrigger").collect::<Vec<_>>(),
		[&b"a\x1bTrigger\x1b\x1b0\x1b-1"[..], &b"b,Trigger,,0,-1"[..],]
	);
	assert_eq!(
		record.singleton_ascii(b"note").unwrap(),
		Some(&b"{ // literal \\\\n \xff }"[..])
	);
	assert!(record.singleton_ascii(b"absent").unwrap().is_none());
	assert!(entities.record(1).is_none());
}

#[test]
fn singleton_fields_reject_equal_or_differently_cased_duplicates() {
	for input in [
		&b"{origin \"1 2 3\" Origin \"1 2 3\"}"[..],
		&b"{origin \"1 2 3\" origin \"9 8 7\"}"[..],
	] {
		let entities = parse(input);
		let error = entities
			.record(0)
			.unwrap()
			.singleton_ascii(b"origin")
			.unwrap_err();
		assert_eq!(error.record, 0);
		assert!(error.first < error.second);
		assert_eq!(entities.pair_count(), 2);
	}
}

#[test]
fn source_spans_belong_to_the_original_immutable_snapshot() {
	let mut input = b"  { \"a\" \"xy\" } \0".to_vec();
	let entities = parse(&input);
	input.fill(b'x');
	let record = entities.record(0).unwrap();
	assert_eq!(record.source(), 2..14);
	let pair = record.pairs().next().unwrap();
	assert_eq!(pair.key_source, 5..6);
	assert_eq!(pair.value_source, 9..11);
	assert_eq!(pair.key, b"a");
	assert_eq!(pair.value, b"xy");
	assert_eq!(&entities.payload()[record.source()], b"{ \"a\" \"xy\" }");
}

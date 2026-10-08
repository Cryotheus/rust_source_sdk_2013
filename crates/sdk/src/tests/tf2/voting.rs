//! Tests of `crate::tf2::voting`: decoding TF2's vote events from their keys.

use super::*;

#[test]
fn accepted_ballot_keeps_entity_index_and_vote_id() {
	assert_eq!(
		parse(
			c"vote_cast",
			&[
				(c"voteidx", 42),
				(c"entityid", 7),
				(c"vote_option", 1),
				(c"team", 3)
			]
		)
		.unwrap(),
		Some(VoteEvent::Cast {
			vote_id: 42,
			entity_index: 7,
			choice: VoteChoice::NO,
			team: 3
		})
	);
}

/// Decodes the event `name` whose integers are `fields`, and whose strings
/// are the options `Yes` and `No`.
fn parse(name: &CStr, fields: &[(&CStr, c_int)]) -> Result<Option<VoteEvent>, VoteEventError> {
	decode(
		name,
		|key| {
			fields
				.iter()
				.find(|(name, _)| *name == key)
				.map(|(_, n)| *n)
		},
		|key| match key.to_bytes() {
			b"option1" => Some(c"Yes".to_owned()),
			b"option2" => Some(c"No".to_owned()),
			_ => None,
		},
	)
}

#[test]
fn rejects_missing_and_invalid_fields_without_defaulting_to_yes() {
	assert_eq!(parse(c"other_event", &[]).unwrap(), None);
	assert_eq!(parse(c"vote_cast", &[]), Err(VoteEventError("voteidx")));
	for count in [-1, 0, 1, 6] {
		assert_eq!(
			parse(c"vote_options", &[(c"voteidx", 1), (c"count", count)]),
			Err(VoteEventError("count"))
		);
	}
	for choice in [-1, 5, 256] {
		assert_eq!(
			parse(
				c"vote_cast",
				&[
					(c"voteidx", 0),
					(c"entityid", 1),
					(c"vote_option", choice),
					(c"team", 0)
				]
			),
			Err(VoteEventError("vote_option"))
		);
	}
}

#[test]
fn starts_keep_their_vote_id_and_option_order() {
	assert_eq!(
		parse(c"vote_options", &[(c"voteidx", 17), (c"count", 2)]).unwrap(),
		Some(VoteEvent::Started {
			vote_id: 17,
			options: vec![c"Yes".to_owned(), c"No".to_owned()]
		})
	);
}

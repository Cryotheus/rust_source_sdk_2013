//! Tests of TF2's custom player attributes through a fake player, whose native
//! attribute methods keep its attributes by name.

use super::*;
use crate::test_support::server::mock_server;
use crate::test_support::tf2::player::{FakePlayer, Method, Value};
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::tf2::script_binding::{FLOAT, STRING, VOID};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::rc::Rc;

/// The native attribute methods of fake players.
const METHODS: &[Method] = &[
	Method::new(c"AddCustomAttribute", VOID, &[STRING, FLOAT, FLOAT]),
	Method::new(c"GetCustomAttribute", FLOAT, &[STRING, FLOAT]),
];

#[test]
fn catalog_attributes_are_set_on_players_within_their_bounds() {
	let fake = FakePlayer::new(METHODS, &[]);
	let stored = Rc::new(RefCell::new(HashMap::<CString, f32>::new()));
	let game = stored.clone();

	fake.answer(
		move |_, method, arguments| match (method.to_bytes(), arguments) {
			(
				b"AddCustomAttribute",
				[Value::String(name), Value::Float(value), Value::Float(_)],
			) => {
				game.borrow_mut().insert(name.clone(), *value);
				Some(Value::Void)
			}

			(b"GetCustomAttribute", [Value::String(name), Value::Float(absent)]) => Some(
				Value::Float(game.borrow().get(name).copied().unwrap_or(*absent)),
			),

			_ => None,
		},
	);

	let server = mock_server(&());
	// SAFETY: The fake player has no attributes for the game to iterate, and
	// the fake game keeps the attributes it is given by name.
	let token = unsafe { trust_shipped_schema(server) };
	let attributes = PlayerAttributes::new(server, fake.entity()).unwrap();
	let factor = |factor| Multiplier::new(factor).unwrap();

	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_FORCE_REDUCTION, factor(1.5), None),
		Err(AttributeError::OutOfDomain)
	);

	for duration in [0.0, -1.0, f32::INFINITY] {
		assert_eq!(
			attributes.set(
				token,
				&catalog::AIRBLAST_VULNERABILITY_MULTIPLIER,
				factor(0.5),
				Some(duration)
			),
			Err(AttributeError::InvalidValue)
		);
	}

	assert!(fake.take_calls().is_empty());

	assert_eq!(
		attributes.set(token, &catalog::DAMAGE_FORCE_REDUCTION, factor(0.25), None),
		Ok(true)
	);
	assert_eq!(
		attributes.set(
			token,
			&catalog::AIRBLAST_VULNERABILITY_MULTIPLIER,
			factor(0.5),
			Some(10.0)
		),
		Ok(true)
	);
	assert_eq!(
		fake.take_calls()
			.into_iter()
			.filter(|(method, _)| *method == c"AddCustomAttribute")
			.map(|(_, arguments)| arguments)
			.collect::<Vec<_>>(),
		[
			vec![
				Value::String(c"damage force reduction".into()),
				Value::Float(0.25),
				Value::Float(DEFAULT_CUSTOM_ATTRIBUTE_DURATION),
			],
			vec![
				Value::String(c"airblast vulnerability multiplier".into()),
				Value::Float(0.5),
				Value::Float(10.0),
			],
		]
	);
	assert_eq!(
		attributes.get(token, c"damage force reduction"),
		Ok(Some(0.25))
	);
	assert_eq!(stored.borrow().len(), 2);

	// A running schema without the name keeps nothing to read back.
	fake.answer(|_, method, _| {
		Some(if method == c"GetCustomAttribute" {
			Value::Float(f32::NAN)
		} else {
			Value::Void
		})
	});
	assert_eq!(
		attributes.set(token, &catalog::MOVE_SPEED_BONUS, factor(1.2), None),
		Ok(false)
	);

	// Definitions of any value type check the float as their type.
	let speed = AnyAttributeDef::Multiplier(catalog::MOVE_SPEED_BONUS);
	let airblast = AnyAttributeDef::Flag(catalog::AIRBLAST_DISABLED);

	assert_eq!(attributes.set_any(token, speed, 1.2, None), Ok(false));
	fake.take_calls();
	assert_eq!(
		attributes.set_any(token, airblast, 0.5, None),
		Err(AttributeError::InvalidValue)
	);
	assert_eq!(
		attributes.set_any(token, speed, 9.0, None),
		Err(AttributeError::OutOfDomain)
	);
	assert!(fake.take_calls().is_empty());

	fake.set_flags(EFL_KILLME);
	assert_eq!(
		attributes.set(token, &catalog::MOVE_SPEED_BONUS, factor(1.2), None),
		Err(AttributeError::MarkedForDeletion)
	);
}

#[test]
fn the_catalog_is_listed_once_and_found_by_name_or_index() {
	for (position, def) in catalog::ALL.iter().enumerate() {
		let others = &catalog::ALL[position + 1..];

		assert!(others.iter().all(|other| other.name() != def.name()));
		assert!(others.iter().all(|other| other.index() != def.index()));
		assert_eq!(catalog::by_index(def.index()), Some(*def));
		assert!(def.min() < def.max());
	}

	assert_eq!(
		catalog::find("DAMAGE BONUS"),
		Some(AnyAttributeDef::Multiplier(catalog::DAMAGE_BONUS))
	);
	assert_eq!(
		catalog::find("move speed bonus").map(|def| def.name()),
		Some(c"move speed bonus")
	);
	assert_eq!(catalog::find("damage bonus "), None);
	assert_eq!(catalog::find("set item tint RGB"), None);
	assert_eq!(catalog::by_index(AttributeIndex::new(142).unwrap()), None);
}

#[test]
fn values_of_any_definition_are_checked_as_its_type() {
	let damage = AnyAttributeDef::Multiplier(catalog::DAMAGE_BONUS);
	let airblast = AnyAttributeDef::Flag(catalog::AIRBLAST_DISABLED);

	assert_eq!(damage.value_kind(), "multiplier");
	assert_eq!(damage.stored(1.5), Ok(1.5));
	assert_eq!(damage.stored(0.5), Err(AttributeError::OutOfDomain));
	assert_eq!(damage.stored(-1.0), Err(AttributeError::InvalidValue));
	assert_eq!(damage.stored(f32::NAN), Err(AttributeError::InvalidValue));
	assert_eq!(airblast.stored(1.0), Ok(1.0));
	assert_eq!(airblast.stored(0.5), Err(AttributeError::InvalidValue));

	let mut set = AttributeSet::new();

	set.insert_any(damage, 2.0).unwrap();
	set.insert(&catalog::DAMAGE_BONUS, Multiplier::new(1.5).unwrap())
		.unwrap();

	assert_eq!(set.len(), 1);
	assert_eq!(
		set.insert_any(airblast, 2.0),
		Err(AttributeError::InvalidValue)
	);
	assert_eq!(set.len(), 1);
	assert_eq!(
		set,
		AttributeSet::new()
			.with(&catalog::DAMAGE_BONUS, Multiplier::new(1.5).unwrap())
			.unwrap()
	);
}

//! Tests of the scan that accepts the schema's attribute lists and flags.

use super::*;
use std::ptr::NonNull;

/// A definition in zeroed memory, with the flags and attribute list of the
/// tests, kept alive with its attributes.
struct Fixture {
	raw: Box<sys::CEconItemDefinition>,
	_attributes: Vec<sys::static_attrib_t>,
}

impl Fixture {
	fn new(index: u16, enabled: bool, attributes: &[u16]) -> Self {
		// SAFETY: The definition is plain data and the tests read only the
		// fields they set; zero is a valid value of each.
		let mut raw: Box<sys::CEconItemDefinition> = unsafe { Box::new(std::mem::zeroed()) };

		let mut elements: Vec<sys::static_attrib_t> = attributes
			.iter()
			.map(|&index| {
				// SAFETY: As above.
				let mut attribute: sys::static_attrib_t = unsafe { std::mem::zeroed() };

				attribute.iDefIndex = index;
				attribute.m_value.asUint32 = 7;
				attribute
			})
			.collect();

		raw.m_nDefIndex = index;
		raw.m_bEnabled = enabled;
		raw.m_vecStaticAttributes.m_Size = elements.len() as c_int;
		raw.m_vecStaticAttributes.m_Memory.m_nAllocationCount = elements.len() as c_int;
		raw.m_vecStaticAttributes.m_Memory.m_pMemory = elements.as_mut_ptr();
		raw.m_vecStaticAttributes.m_pElements = elements.as_mut_ptr();

		Self {
			raw,
			_attributes: elements,
		}
	}

	fn definition(&mut self) -> ItemDefinition<'_> {
		ItemDefinition::new(NonNull::from(&mut *self.raw))
	}
}

#[test]
fn a_consistent_schema_is_counted() {
	let mut plain = Fixture::new(1, true, &[]);
	let mut armed = Fixture::new(2, true, &[10, 20]);
	let mut hidden = Fixture::new(3, false, &[30]);

	hidden.raw.m_bHidden = true;
	hidden.raw.m_bBaseItem = true;

	let definitions = [plain.definition(), armed.definition(), hidden.definition()];
	let stats = scan(&definitions).unwrap();

	assert_eq!(
		stats,
		CatalogStats {
			definitions: 3,
			with_attributes: 2,
			attributes: 3,
			enabled: 2,
			hidden: 1,
			base_items: 1,
			tools: 0,
		}
	);
}

#[test]
fn a_flag_that_is_not_a_boolean_refuses_the_schema() {
	let mut odd = Fixture::new(5, true, &[10]);

	// SAFETY: Writes one byte of the boolean the scan reads as a byte.
	unsafe { (&raw mut odd.raw.m_bHidden).cast::<u8>().write(0x80) };

	let mut fine = Fixture::new(6, true, &[10]);
	let definitions = [fine.definition(), odd.definition()];

	assert_eq!(
		scan(&definitions),
		Err(CatalogFault {
			index: 5,
			reason: FaultReason::Flag(0x80)
		})
	);
}

#[test]
fn a_schema_that_reads_nothing_is_refused() {
	let mut off = Fixture::new(1, false, &[10]);
	let mut bare = Fixture::new(2, true, &[]);
	let mut closed = Fixture::new(3, false, &[]);

	assert_eq!(
		scan(&[off.definition()]).unwrap_err().reason,
		FaultReason::NoneEnabled
	);
	assert_eq!(
		scan(&[bare.definition(), closed.definition()])
			.unwrap_err()
			.reason,
		FaultReason::NoAttributes
	);
}

#[test]
fn a_schema_with_every_definition_enabled_is_refused() {
	let mut one = Fixture::new(1, true, &[10]);
	let mut two = Fixture::new(2, true, &[]);

	assert_eq!(
		scan(&[one.definition(), two.definition()])
			.unwrap_err()
			.reason,
		FaultReason::NoneDisabled
	);
}

#[test]
fn an_attribute_without_an_index_refuses_the_schema() {
	let mut zero = Fixture::new(5, true, &[10, 0]);
	let definitions = [zero.definition()];

	assert_eq!(
		scan(&definitions),
		Err(CatalogFault {
			index: 5,
			reason: FaultReason::AttributeIndex
		})
	);
}

#[test]
fn names_match_through_spaces_underscores_and_typos() {
	let names = vec![(18, c"TF_WEAPON_ROCKETLAUNCHER"), (9, c"Pet Balloonicorn")];
	let rank = |query: &CStr| rank_names(names.clone(), query.to_bytes(), 5);

	assert_eq!(rank(c"rocket launcher")[0].0, 18);
	assert_eq!(rank(c"rocket launchr")[0].0, 18);
	assert_eq!(rank(c"pet baloonicorn")[0].0, 9);
	assert!(rank(c"completely different").is_empty());
	assert!(rank(c"").is_empty());
}

#[test]
fn static_attributes_read_their_index_and_value() {
	let mut armed = Fixture::new(2, true, &[10, 20]);
	let attributes = armed.definition().static_attribute_elements(2);

	assert_eq!(
		attributes,
		[
			StaticAttribute {
				index: 10,
				value: AttributeBits::from_bits(7)
			},
			StaticAttribute {
				index: 20,
				value: AttributeBits::from_bits(7)
			},
		]
	);
}

#[test]
fn vector_headers_must_agree_with_themselves() {
	let good = VectorHeader {
		memory: 0x1000,
		allocation: 4,
		size: 3,
		elements: 0x1000,
	};

	assert_eq!(good.check(8, 128), Ok(()));

	let empty = VectorHeader {
		memory: 0,
		allocation: 0,
		size: 0,
		elements: 0,
	};

	assert_eq!(empty.check(8, 128), Ok(()));

	let cases = [
		(
			VectorHeader { size: 129, ..good },
			FaultReason::AttributeCount(129),
		),
		(
			VectorHeader { size: -1, ..good },
			FaultReason::AttributeCount(-1),
		),
		(
			VectorHeader {
				elements: 0x2000,
				..good
			},
			FaultReason::AttributeMemory,
		),
		(
			VectorHeader {
				memory: 0x1004,
				elements: 0x1004,
				..good
			},
			FaultReason::AttributeElements(3),
		),
		(
			VectorHeader {
				memory: 0,
				elements: 0,
				..good
			},
			FaultReason::AttributeElements(3),
		),
		(
			VectorHeader {
				allocation: 2,
				..good
			},
			FaultReason::AttributeAllocation(2),
		),
	];

	for (header, reason) in cases {
		assert_eq!(header.check(8, 128), Err(reason));
	}
}

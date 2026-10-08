//! Tests of the scan for `TheNavAreas` and of reading its areas.

use super::*;
use crate::util::Section;

const BASE: usize = 0x180000000;
const CODE: usize = BASE + 0x1000;
const MESSAGE: usize = BASE + 0x3000;
const AREAS: usize = BASE + 0x4000;

/// Points the rel32 operand at `operand` of the code at `CODE + offset` to
/// `target`, for an instruction ending right after it.
fn put_relative(bytes: &mut [u8], offset: usize, operand: usize, target: usize) {
	let at = offset + operand;
	let end = CODE + at + 4;
	let delta = i32::try_from(target as i128 - end as i128).unwrap();
	bytes[at..at + 4].copy_from_slice(&delta.to_le_bytes());
}

/// A copy of the loop at `offset` in `bytes`, reading `areas` and printing
/// `message`.
fn put_loop(bytes: &mut [u8], offset: usize, areas: usize, message: usize) {
	for (out, input) in bytes[offset..].iter_mut().zip(UPDATE_LIGHTING) {
		*out = match input {
			SignaturePattern::Exact(byte) => byte,
			SignaturePattern::Any => 0,
		};
	}

	put_relative(bytes, offset, UPDATE_LIGHTING_ELEMENTS, areas);
	put_relative(bytes, offset, UPDATE_LIGHTING_MESSAGE_OPERAND, message);

	for operand in UPDATE_LIGHTING_SIZES {
		put_relative(bytes, offset, operand, areas + 16);
	}
}

/// An image whose code holds one copy of the loop, whose read-only data holds
/// the message, and whose writable data holds the areas.
fn update_lighting() -> Image {
	let mut code = vec![0xcc; 0x200];
	put_loop(&mut code, 0x40, AREAS, MESSAGE);

	let mut rdata = vec![0; 0x100];
	rdata[..UPDATE_LIGHTING_MESSAGE.len()].copy_from_slice(UPDATE_LIGHTING_MESSAGE);

	Image {
		base: BASE,
		sections: vec![
			Section {
				address: CODE,
				bytes: code,
				executable: true,
				writable: false,
			},
			Section {
				address: MESSAGE,
				bytes: rdata,
				executable: false,
				writable: false,
			},
			Section {
				address: AREAS,
				bytes: vec![0; 0x100],
				executable: false,
				writable: true,
			},
		],
	}
}

#[test]
fn finds_the_areas_the_loop_counts() {
	assert_eq!(nav_areas_in(&update_lighting()), Some(AREAS));
}

#[test]
fn every_reference_to_the_areas_must_agree() {
	for operand in UPDATE_LIGHTING_SIZES {
		let mut image = update_lighting();
		put_relative(&mut image.sections[0].bytes, 0x40, operand, AREAS + 24);
		assert_eq!(nav_areas_in(&image), None, "size at {operand}");
	}

	let mut image = update_lighting();
	put_relative(
		&mut image.sections[0].bytes,
		0x40,
		UPDATE_LIGHTING_ELEMENTS,
		AREAS + 8,
	);
	assert_eq!(nav_areas_in(&image), None);
}

#[test]
fn the_message_must_be_the_commands() {
	let mut image = update_lighting();
	image.sections[1].bytes[22] = b'D';
	assert_eq!(nav_areas_in(&image), None);

	// The same text with no terminator is another string.
	let mut image = update_lighting();
	image.sections[1].bytes[UPDATE_LIGHTING_MESSAGE.len() - 1] = b'!';
	assert_eq!(nav_areas_in(&image), None);

	let mut image = update_lighting();
	image.sections[1].executable = true;
	assert_eq!(nav_areas_in(&image), None);
}

#[test]
fn the_areas_must_be_writable_data() {
	let mut image = update_lighting();
	image.sections[2].writable = false;
	assert_eq!(nav_areas_in(&image), None);

	let mut image = update_lighting();
	image.sections[2].executable = true;
	assert_eq!(nav_areas_in(&image), None);

	// The whole vector must fit in the section.
	let mut image = update_lighting();
	image.sections[2]
		.bytes
		.truncate(size_of::<NavAreaVector>() - 1);
	assert_eq!(nav_areas_in(&image), None);
}

#[test]
fn two_copies_of_the_loop_are_ambiguous() {
	let mut image = update_lighting();
	put_loop(&mut image.sections[0].bytes, 0x100, AREAS, MESSAGE);
	assert_eq!(nav_areas_in(&image), None);
}

#[test]
fn elements_are_read_in_place() {
	let mut areas = [0u64; 4];
	let mut elements = [
		(&raw mut areas[0]).cast::<sys::CNavArea>(),
		(&raw mut areas[2]).cast::<sys::CNavArea>(),
	];

	// SAFETY: A zeroed vector is an empty one.
	let mut vector: NavAreaVector = unsafe { std::mem::zeroed() };
	vector.m_Memory.m_pMemory = elements.as_mut_ptr();
	vector.m_Memory.m_nAllocationCount = 2;
	vector.m_Size = 2;
	vector.m_pElements = elements.as_mut_ptr();

	// SAFETY: The vector and its elements outlive the slice.
	let read = unsafe { nav_area_elements(NonNull::from(&mut vector)) };
	assert_eq!(read, elements);

	vector.m_Size = 1;
	// SAFETY: As above.
	assert_eq!(
		unsafe { nav_area_elements(NonNull::from(&mut vector)) },
		&elements[..1]
	);

	// A negative count, as a corrupt vector could hold, reads as empty.
	vector.m_Size = -1;
	// SAFETY: As above.
	assert!(unsafe { nav_area_elements(NonNull::from(&mut vector)) }.is_empty());

	vector.m_Size = 2;
	vector.m_Memory.m_pMemory = std::ptr::null_mut();
	// SAFETY: A null vector reads as empty, whatever its count.
	assert!(unsafe { nav_area_elements(NonNull::from(&mut vector)) }.is_empty());
}

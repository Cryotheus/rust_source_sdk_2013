//! Tests of the ABI details each game decides.

use super::*;
use sdk_raw::entities::{SDK2013_NEXT_BOT_TELEPORT_SLOT, SDK2013_TELEPORT_SLOT, TF2_TELEPORT_SLOT};

#[test]
fn each_game_teleports_through_its_game_dlls_slot() {
	assert_eq!(
		Game::TeamFortress2.teleport_vtable_slot().index(),
		TF2_TELEPORT_SLOT
	);
	assert_eq!(
		Game::SourceSdk2013.teleport_vtable_slot().index(),
		SDK2013_TELEPORT_SLOT
	);
	assert_eq!(
		Game::SourceSdk2013NextBot.teleport_vtable_slot().index(),
		SDK2013_NEXT_BOT_TELEPORT_SLOT
	);
}

thread_local! {
	static COLORED_CALLS: std::cell::RefCell<Vec<(i32, sdk_raw::tier0::SpewColor, String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

unsafe extern "C" fn capture_color_console(
	level: i32,
	color: *const sdk_raw::tier0::SpewColor,
	format: *const std::ffi::c_char,
	mut arguments: ...
) {
	// SAFETY: The wrapper passes one live four-byte Color and one %s vararg.
	let entry = unsafe {
		(
			level,
			*color,
			CStr::from_ptr(format).to_string_lossy().into_owned(),
			CStr::from_ptr(arguments.next_arg::<*const std::ffi::c_char>())
				.to_string_lossy()
				.into_owned(),
		)
	};
	COLORED_CALLS.with(|calls| calls.borrow_mut().push(entry));
}

#[test]
fn native_colored_console_abi_keeps_color_separate_and_text_literal() {
	use sdk_raw::tier0::{SpewColor, color_print_through};
	assert_eq!(std::mem::size_of::<SpewColor>(), 4);
	assert_eq!(std::mem::align_of::<SpewColor>(), 1);
	let color = SpewColor {
		r: 0,
		g: 0,
		b: 255,
		a: 255,
	};
	COLORED_CALLS.with(|calls| calls.borrow_mut().clear());
	// SAFETY: The mock has the exported C ABI and validates its exact arguments.
	unsafe { color_print_through(capture_color_console, color, c"雪 literal %s %n 100%\n") };
	COLORED_CALLS.with(|calls| {
		assert_eq!(
			*calls.borrow(),
			[(
				0,
				color,
				"%s".to_owned(),
				"雪 literal %s %n 100%\n".to_owned()
			)]
		)
	});
}

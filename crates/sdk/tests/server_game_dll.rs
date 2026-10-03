//! Tests of the game's answers through its main interface (`IServerGameDLL`).

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tier0::MAX_PATH;
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::ServerGameDll;
use source_sdk_2013::interfaces::server_game_dll::LevelProvision;
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

/// `IServerGameDLL::CanProvideLevel`, which resolves `workshop/123` in place.
unsafe extern "C" fn can_provide_level(
	_: *mut sys::IServerGameDLL,
	name: *mut c_char,
	capacity: c_int,
) -> sys::IServerGameDLL_eCanProvideLevelResult {
	assert_eq!(capacity as usize, MAX_PATH);

	// SAFETY: The wrapper passes a NUL-terminated name in a buffer of
	// `capacity` bytes, which the resolved name fits.
	unsafe {
		if CStr::from_ptr(name) != c"workshop/123" {
			return sys::IServerGameDLL_eCanProvideLevelResult_eCanProvideLevel_CannotProvide;
		}

		ptr::copy_nonoverlapping(c"workshop/cp_example.ugc123".as_ptr(), name, 27);
	}

	sys::IServerGameDLL_eCanProvideLevelResult_eCanProvideLevel_CanProvide
}

/// `IServerGameDLL::IsManualMapChangeOkay`, which refuses with a reason.
unsafe extern "C" fn manual_map_change(
	_: *mut sys::IServerGameDLL,
	reason: *mut *const c_char,
) -> bool {
	// SAFETY: The wrapper passes a writable pointer for the reason.
	unsafe { reason.write(c"Tournament in progress".as_ptr()) };
	false
}

#[test]
fn methods_convert_the_games_answers() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerGameDLL_GetTickInterval).write(tick_interval);
			(&raw mut (*vtable).IServerGameDLL_Status).write(status);
			(&raw mut (*vtable).IServerGameDLL_GetUserMessageInfo).write(user_message_info);
			(&raw mut (*vtable).IServerGameDLL_CanProvideLevel).write(can_provide_level);
			(&raw mut (*vtable).IServerGameDLL_IsManualMapChangeOkay).write(manual_map_change);
		})
	};

	export(
		Module::GameServer,
		ServerGameDll::VERSION,
		leak(sys::IServerGameDLL {
			vtable_: Box::leak(vtable),
		}),
	);

	let scope = ();
	let game = mock_server(&scope).server_game_dll().unwrap();

	assert_eq!(game.tick_interval(), 0.015);
	assert_eq!(
		game.status().as_c_str(),
		c"Blue Team Wins: 3\nScout       2.5\n"
	);

	let messages = game.user_messages().collect::<Vec<_>>();

	assert_eq!(messages.len(), 2);
	assert_eq!(
		(messages[0].name.as_c_str(), messages[0].size),
		(c"Geiger", Some(1))
	);
	assert_eq!((messages[1].index, messages[1].size), (1, None));

	assert_eq!(
		game.can_provide_level(c"workshop/123"),
		Ok(LevelProvision::CanProvide(
			c"workshop/cp_example.ugc123".to_owned()
		))
	);
	assert_eq!(
		game.can_provide_level(c"ctf_2fort"),
		Ok(LevelProvision::CannotProvide)
	);
	assert!(
		game.can_provide_level(&CString::new("x".repeat(MAX_PATH)).unwrap())
			.is_err()
	);

	assert_eq!(
		game.is_manual_map_change_okay().unwrap_err().to_string(),
		"the game refuses map changes right now: Tournament in progress"
	);
}

/// `IServerGameDLL::Status`, which prints two lines through `printf`-style
/// formats.
unsafe extern "C" fn status(
	_: *mut sys::IServerGameDLL,
	print: Option<unsafe extern "C" fn(*const c_char, ...)>,
) {
	let print = print.unwrap();

	// SAFETY: The wrapper passes a `printf`-style function, and each format's
	// arguments match it.
	unsafe {
		print(c"Blue Team Wins: %d\n".as_ptr(), 3 as c_int);
		print(c"%-8s %6.1f\n".as_ptr(), c"Scout".as_ptr(), 2.5f64);
	}
}

/// `IServerGameDLL::GetTickInterval`.
unsafe extern "C" fn tick_interval(_: *const sys::IServerGameDLL) -> f32 {
	0.015
}

/// `IServerGameDLL::GetUserMessageInfo`, for two messages, the second of
/// variable size.
unsafe extern "C" fn user_message_info(
	_: *mut sys::IServerGameDLL,
	index: c_int,
	name: *mut c_char,
	capacity: c_int,
	size: *mut c_int,
) -> bool {
	let (message, message_size): (&CStr, c_int) = match index {
		0 => (c"Geiger", 1),
		1 => (c"SayText2", -1),
		_ => return false,
	};

	assert!(capacity as usize > message.count_bytes());

	// SAFETY: The wrapper passes a buffer of `capacity` bytes, which the name
	// fits, and a writable size.
	unsafe {
		ptr::copy_nonoverlapping(message.as_ptr(), name, message.count_bytes() + 1);
		size.write(message_size);
	}

	true
}

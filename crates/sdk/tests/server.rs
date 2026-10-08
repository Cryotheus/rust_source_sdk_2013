//! Tests of finding interfaces through the engine's and the game server's
//! interface factories.

use source_sdk_2013::Module;
use source_sdk_2013::test_support::server::{export, mock_server};

#[test]
fn interfaces_resolve_by_exact_version_from_their_own_module() {
	let mut engine = sys::IVEngineServer {
		vtable_: std::ptr::null(),
	};
	let engine_pointer = &raw mut engine;
	let scope = ();

	// Exported by the wrong module, and at a newer version: neither may bind.
	export(Module::GameServer, c"VEngineServer023", engine_pointer);
	export(Module::Engine, c"VEngineServer024", engine_pointer);

	let server = mock_server(&scope);
	let error = server.valve_engine().unwrap_err();

	assert_eq!(error.module(), Module::Engine);
	assert_eq!(error.version(), c"VEngineServer023");

	export(Module::Engine, c"VEngineServer023", engine_pointer);

	assert_eq!(server.valve_engine().unwrap().as_ptr(), engine_pointer);
	assert_eq!(
		server
			.find_interface::<sys::IVEngineServer>(Module::Engine, c"VEngineServer023")
			.unwrap()
			.as_ptr(),
		engine_pointer
	);
	assert!(server.server_game_dll().is_err());
}

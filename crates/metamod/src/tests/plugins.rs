//! Tests of `crate::plugins`: Metamod's plugin list, through a mock
//! `ISmmPluginManager`.

use super::*;
use crate::MetamodVersion;
use crate::sys::plugin_manager::{ISmmPluginManagerVtable, PL_CONSOLE, PL_FILE};
use crate::test_support::smm::MockSmm;
use std::ffi::c_void;

/// A plugin the mock lists.
struct Listed {
	id: PluginId,
	file: &'static CStr,
	status: PlStatus,
	source: PluginId,

	/// What its `QueryRunning` answers: `None` when it runs.
	not_running: Option<&'static CStr>,
}

/// Metamod's plugin manager, listing `plugins`. Its slots other than
/// `Query` and `QueryRunning` are never called.
#[repr(C)]
struct MockManager {
	manager: ISmmPluginManager,
	plugins: Vec<Listed>,
	vtable: Box<ISmmPluginManagerVtable>,
}

impl MockManager {
	fn new(plugins: Vec<Listed>) -> Box<Self> {
		let vtable = Box::new(ISmmPluginManagerVtable {
			load: unreachable_load,
			unload: unreachable_unload,
			pause: unreachable_pause,
			unpause: unreachable_pause,
			unload_all: unreachable_unload_all,
			query,
			query_running,
			query_handle: unreachable_query_handle,
		});

		Box::new(Self {
			manager: ISmmPluginManager {
				vtable: &raw const *vtable,
			},
			plugins,
			vtable,
		})
	}

	fn manager(&self) -> PluginManager {
		// SAFETY: The mock outlives the test's uses of it, on its thread.
		unsafe { PluginManager::from_raw(NonNull::from(&self.manager)) }
	}

	/// The listed plugin with `id`.
	///
	/// # Safety
	///
	/// `this` must be a `MockManager`.
	unsafe fn find<'a>(this: *mut ISmmPluginManager, id: PluginId) -> Option<&'a Listed> {
		// SAFETY: As the caller promises.
		unsafe {
			(*this.cast::<Self>())
				.plugins
				.iter()
				.find(|plugin| plugin.id == id)
		}
	}
}

unsafe extern "C" fn query(
	this: *mut ISmmPluginManager,
	id: PluginId,
	file: *mut *const c_char,
	status: *mut PlStatus,
	source: *mut PluginId,
) -> bool {
	// SAFETY: Every mock manager is a `MockManager`, and the binding passes
	// its variables.
	unsafe {
		let Some(plugin) = MockManager::find(this, id) else {
			return false;
		};

		file.write(plugin.file.as_ptr());
		status.write(plugin.status);
		source.write(plugin.source);
	}

	true
}

unsafe extern "C" fn query_running(
	this: *mut ISmmPluginManager,
	id: PluginId,
	error: *mut c_char,
	capacity: usize,
) -> bool {
	// SAFETY: As for `query`, with the binding's buffer of `capacity` bytes.
	unsafe {
		let reason = match MockManager::find(this, id) {
			Some(Listed {
				not_running: None, ..
			}) => return true,
			Some(Listed {
				not_running: Some(reason),
				..
			}) => reason.to_bytes(),
			None => b"Plugin not valid",
		};

		// As Metamod's `UTIL_Format` does, cut to fit with its NUL.
		let length = reason.len().min(capacity - 1);

		std::ptr::copy_nonoverlapping(reason.as_ptr().cast(), error, length);
		error.add(length).write(0);
	}

	false
}

unsafe extern "C" fn unreachable_load(
	_: *mut ISmmPluginManager,
	_: *const c_char,
	_: PluginId,
	_: *mut bool,
	_: *mut c_char,
	_: usize,
) -> PluginId {
	unreachable!("the mock loads nothing")
}

unsafe extern "C" fn unreachable_unload(
	_: *mut ISmmPluginManager,
	_: PluginId,
	_: bool,
	_: *mut c_char,
	_: usize,
) -> bool {
	unreachable!("the mock unloads nothing")
}

unsafe extern "C" fn unreachable_pause(
	_: *mut ISmmPluginManager,
	_: PluginId,
	_: *mut c_char,
	_: usize,
) -> bool {
	unreachable!("the mock pauses nothing")
}

unsafe extern "C" fn unreachable_unload_all(_: *mut ISmmPluginManager) -> bool {
	unreachable!("the mock unloads nothing")
}

unsafe extern "C" fn unreachable_query_handle(
	_: *mut ISmmPluginManager,
	_: PluginId,
	_: *mut *mut c_void,
) -> bool {
	unreachable!("the mock has no handles")
}

/// Plugins 1, 2 and 5, of which 2 is paused, and 6, which refused to load.
fn listed() -> Vec<Listed> {
	vec![
		Listed {
			id: 1,
			file: c"/srv/tf/addons/first/bin/first.so",
			status: raw::PL_RUNNING,
			source: PL_FILE,
			not_running: None,
		},
		Listed {
			id: 2,
			file: c"/srv/tf/addons/second/bin/second.so",
			status: raw::PL_PAUSED,
			source: PL_CONSOLE,
			not_running: Some(c"waiting for its map"),
		},
		Listed {
			id: 5,
			file: c"/srv/tf/addons/third/bin/third.so",
			status: raw::PL_RUNNING,
			source: 1,
			not_running: None,
		},
		Listed {
			id: 6,
			file: c"/srv/tf/addons/broken/bin/broken.so",
			status: raw::PL_REFUSED,
			source: PL_CONSOLE,
			not_running: Some(c"Plugin not valid"),
		},
	]
}

#[test]
fn plugins_are_listed_by_id_past_gaps() {
	let mock = MockManager::new(listed());
	let plugins = mock.manager().plugins();
	let ids: Vec<PluginId> = plugins.iter().map(|plugin| plugin.id).collect();

	assert_eq!(ids, [1, 2, 5, 6]);

	assert_eq!(
		plugins[1],
		PluginEntry {
			id: 2,
			file: c"/srv/tf/addons/second/bin/second.so".to_owned(),
			state: PluginState::Paused,
			source: PluginSource::Console,
		}
	);

	assert_eq!(plugins[0].source, PluginSource::File);
	assert_eq!(plugins[2].source, PluginSource::Plugin(1));
	assert_eq!(plugins[3].state, PluginState::Refused);
	assert_eq!(mock.manager().query(3), None);
}

#[test]
fn the_search_stops_after_a_long_gap() {
	let reachable = raw::PL_MIN_ID + FREE_RUN;
	let past = reachable + FREE_RUN + 1;

	let plugin = |id: PluginId| Listed {
		id,
		file: c"plugin.so",
		status: raw::PL_RUNNING,
		source: PL_CONSOLE,
		not_running: None,
	};

	let mock = MockManager::new(vec![
		plugin(raw::PL_MIN_ID),
		plugin(reachable),
		plugin(past),
	]);

	let ids: Vec<PluginId> = mock
		.manager()
		.plugins()
		.iter()
		.map(|plugin| plugin.id)
		.collect();

	// 4095 free IDs before `reachable`, and 4096 before `past`.
	assert_eq!(ids, [raw::PL_MIN_ID, reachable]);
	assert!(mock.manager().query(past).is_some());
}

#[test]
fn plugins_say_why_they_do_not_run() {
	let mock = MockManager::new(listed());
	let manager = mock.manager();

	assert_eq!(manager.query_running(1), Ok(()));
	assert_eq!(
		manager.query_running(2),
		Err(c"waiting for its map".to_owned())
	);
	assert_eq!(
		manager.query_running(3),
		Err(c"Plugin not valid".to_owned())
	);
}

#[test]
fn states_keep_their_raw_values_and_names() {
	for raw in -6..=2 {
		assert_eq!(PluginState::from_raw(raw).to_raw(), raw);
	}

	assert_eq!(PluginState::from_raw(raw::PL_REFUSED).name(), "FAILED");
	assert_eq!(PluginState::from_raw(raw::PL_NOT_FOUND).name(), "NOFILE");
	assert_eq!(PluginSource::from_raw(0), PluginSource::Other(0));
}

#[test]
fn metamod_gives_its_plugin_manager() {
	let manager = MockManager::new(listed());

	for version in [MetamodVersion::Stable1226, MetamodVersion::Dev1469] {
		let mut smm = MockSmm::new(version, &[]);

		assert!(smm.api().plugin_manager().is_none());

		smm.plugin_manager = (&raw const manager.manager).cast_mut().cast();

		let found = smm.api().plugin_manager().expect("the mock has one");

		assert_eq!(found.as_ptr(), (&raw const manager.manager).cast_mut());
		assert_eq!(found.query(5).map(|plugin| plugin.id), Some(5));
	}
}

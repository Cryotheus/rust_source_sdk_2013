//! `IVEngineServer`, the engine's services for the game server.

use crate::edicts::Edict;
use crate::net::NetChannel;
use crate::players::UserId;
use sdk_raw::edicts::MAX_EDICTS;
use sdk_raw::players::ABSOLUTE_PLAYER_LIMIT;
use sdk_raw::tier0::MAX_PATH;
use sdk_raw::util::cstr::{copy_cstr, cstring_from_buffer};
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::{self, NonNull};

interface! {
	/// The engine's services for the game server (`IVEngineServer`).
	#[doc(alias = "IVEngineServer")]
	pub struct ValveEngine(sys::IVEngineServer) = Engine c"VEngineServer023";
}

impl<'s> ValveEngine<'s> {
	/// The engine's change-tracking record for an edict.
	#[doc(alias = "GetChangeAccessor")]
	pub(crate) fn change_accessor(
		self,
		edict: Edict<'_>,
	) -> Option<NonNull<sys::IChangeInfoAccessor>> {
		// SAFETY: As for `user_id_of_edict`.
		NonNull::new(unsafe {
			vcall!(self.as_ptr() => IVEngineServer_GetChangeAccessor(edict.as_ptr()))
		})
	}

	/// Queues a change to another level, as the `changelevel` command does.
	///
	/// `landmark` is only used by single-player level transitions.
	#[doc(alias = "ChangeLevel")]
	pub fn change_level(self, map: &CStr, landmark: Option<&CStr>) {
		let landmark = landmark.map_or(ptr::null(), CStr::as_ptr);

		// SAFETY: `Server::new` guarantees the interface is live.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_ChangeLevel(map.as_ptr(), landmark)) };
	}

	/// The value a client reported for one of its user settings, the console
	/// variables marked `FCVAR_USERINFO`, such as `name` or `cl_interp`.
	///
	/// The engine reports an unknown setting, or an edict that no connected
	/// client owns, as empty. Returns `None` if the engine returns null.
	#[doc(alias = "GetClientConVarValue")]
	pub fn client_convar_value(self, client: Edict<'_>, name: &CStr) -> Option<CString> {
		// SAFETY: As for `change_level`. The engine checks the index, and the
		// value is copied at once.
		unsafe {
			copy_cstr(
				vcall!(self.as_ptr() => IVEngineServer_GetClientConVarValue(client.index(), name.as_ptr())),
			)
		}
	}

	/// Prints a message to the console of the client owning an edict.
	#[doc(alias = "ClientPrintf")]
	pub fn client_print(self, client: Edict<'_>, message: &CStr) {
		// SAFETY: As for `change_level`, and the edict is live. The engine
		// ignores edicts that no connected client owns.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_ClientPrintf(client.as_ptr(), message.as_ptr()))
		};
	}

	/// Offsets the crosshair of the client owning an edict, in degrees.
	#[doc(alias = "CrosshairAngle")]
	pub fn crosshair_angle(self, client: Edict<'_>, pitch: f32, yaw: f32) {
		// SAFETY: As for `set_view`.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_CrosshairAngle(client.as_ptr(), pitch, yaw))
		};
	}

	/// Looks up the edict at an entity index.
	///
	/// Returns `None` if the index is outside the edict table or its slot is
	/// free. The engine keeps every player slot's edict in use, even while no
	/// client occupies it.
	#[doc(alias = "PEntityOfEntIndex")]
	#[doc(alias = "INDEXENT")]
	pub fn edict_of_index(self, index: c_int) -> Option<Edict<'s>> {
		// The engine validates the index too, but an out-of-range index never
		// needs to reach it.
		if !(0..MAX_EDICTS).contains(&index) {
			return None;
		}

		// SAFETY: As for `change_level`.
		let edict = NonNull::new(unsafe {
			vcall!(self.as_ptr() => IVEngineServer_PEntityOfEntIndex(index))
		})?;

		// SAFETY: The engine returned an element of its edict table, which only
		// a level change reallocates, and no level changes during `'s`.
		Some(unsafe { Edict::from_raw(edict) })
	}

	/// Finds the edict of the player with a user ID.
	///
	/// Like the game's own `UTIL_PlayerByUserId`, this asks the engine which
	/// client owns the edict of each player slot. Returns `None` if no connected
	/// client has the user ID.
	///
	/// A client's edict resolves as soon as it connects, before its player
	/// entity spawns, and until its disconnection completes, so the edict may
	/// have no entity yet. A lookup scans up to [`ABSOLUTE_PLAYER_LIMIT`] slots.
	pub fn edict_of_user_id(self, user_id: UserId) -> Option<Edict<'s>> {
		(1..=ABSOLUTE_PLAYER_LIMIT)
			.filter_map(|index| self.edict_of_index(index))
			.find(|&edict| !edict.is_free() && self.user_id_of_edict(edict) == Some(user_id))
	}

	/// The path of the game directory, such as `.../tf`.
	#[doc(alias = "GetGameDir")]
	pub fn game_dir(self) -> CString {
		let mut buffer = [0 as c_char; MAX_PATH];

		// SAFETY: As for `change_level`, and the buffer length is passed.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_GetGameDir(buffer.as_mut_ptr(), MAX_PATH as c_int))
		};

		cstring_from_buffer(&buffer)
	}

	/// Whether a map file exists and can be loaded, such as `maps/ctf_2fort.bsp`.
	#[doc(alias = "IsMapValid")]
	pub fn is_map_valid(self, file: &CStr) -> bool {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_IsMapValid(file.as_ptr())) != 0 }
	}

	/// Locks or unlocks the network string tables, and returns whether they
	/// were locked before.
	///
	/// While a level runs, the game unlocks the tables around code that adds
	/// strings, such as spawning a player, and then restores the state this
	/// returned, as the engine's interface asks of every caller.
	/// [`Self::with_unlocked_string_tables`] does both.
	#[doc(alias = "LockNetworkStringTables")]
	pub fn lock_network_string_tables(self, lock: bool) -> bool {
		// SAFETY: As for `change_level`. The game itself sets both states during
		// ordinary play, such as around a player's first spawn.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_LockNetworkStringTables(lock)) }
	}

	/// Writes a line to the server log, as the `log` command does.
	#[doc(alias = "LogPrint")]
	pub fn log_print(self, message: &CStr) {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_LogPrint(message.as_ptr())) };
	}

	/// The net channel of the client owning an edict.
	///
	/// Returns `None` for an edict that no connected client owns, and for fake
	/// clients such as bots and SourceTV, which have no channel.
	#[doc(alias = "GetPlayerNetInfo")]
	pub fn net_channel(self, client: Edict<'_>) -> Option<NetChannel<'s>> {
		// SAFETY: As for `change_level`. The engine checks the index.
		let info = NonNull::new(unsafe {
			vcall!(self.as_ptr() => IVEngineServer_GetPlayerNetInfo(client.index()))
		})?;

		// SAFETY: The engine returns its client's `CNetChan`, whose only base is
		// `INetChannel`, which derives only from `INetChannelInfo`, so both are
		// at the same address. The engine frees a channel when its client
		// disconnects, which nothing safe causes during `'s`.
		Some(unsafe { NetChannel::from_raw(info.cast()) })
	}

	/// The network ID of the client owning an edict, such as a rendered Steam
	/// ID or `BOT`.
	///
	/// Returns `None` for an edict that no client owns, which covers the edict
	/// of every entity that is not a player. The edict of an empty player slot
	/// can still report an ID, such as `STEAM_ID_PENDING`.
	#[doc(alias = "GetPlayerNetworkIDString")]
	pub fn player_network_id(self, edict: Edict<'_>) -> Option<CString> {
		// SAFETY: As for `user_id_of_edict`. The engine renders the ID into a
		// buffer it reuses, so it is copied immediately.
		unsafe {
			copy_cstr(
				vcall!(self.as_ptr() => IVEngineServer_GetPlayerNetworkIDString(edict.as_ptr())),
			)
		}
	}

	/// Queues a command as though it were entered at the server console.
	///
	/// Commands are normally processed on the next frame. Include a command
	/// separator such as a trailing newline when required by the command parser.
	#[doc(alias = "ServerCommand")]
	pub fn server_command(self, command: &CStr) {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_ServerCommand(command.as_ptr())) };
	}

	/// Renders a client's view from another entity, such as a camera, or from
	/// its own player again.
	#[doc(alias = "SetView")]
	pub fn set_view(self, client: Edict<'_>, view: Edict<'_>) {
		// SAFETY: As for `change_level`, and both edicts are live. The engine
		// ignores edicts that no connected client owns.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_SetView(client.as_ptr(), view.as_ptr())) };
	}

	/// The engine's per-frame record of changed network variables.
	#[doc(alias = "GetSharedEdictChangeInfo")]
	pub(crate) fn shared_edict_change_info(self) -> Option<NonNull<sys::CSharedEdictChangeInfo>> {
		// SAFETY: As for `change_level`.
		NonNull::new(unsafe { vcall!(self.as_ptr() => IVEngineServer_GetSharedEdictChangeInfo()) })
	}

	/// Returns the user ID of the player whose client owns an edict.
	///
	/// Returns `None` for an edict that no connected client owns, which covers
	/// the edict of every entity that is not a player.
	#[doc(alias = "GetPlayerUserId")]
	pub fn user_id_of_edict(self, edict: Edict<'_>) -> Option<UserId> {
		// The engine returns -1 for an edict none of its clients own, and can
		// return 0 for the edict of a client slot nobody occupies. Neither is a
		// user ID, and both mean no player owns the edict.
		//
		// SAFETY: As for `change_level`, and the edict is live.
		UserId::from_raw(unsafe {
			vcall!(self.as_ptr() => IVEngineServer_GetPlayerUserId(edict.as_ptr()))
		})
		.ok()
	}

	/// Runs `f` with the network string tables unlocked, then restores the lock
	/// state they had before, even if `f` panics.
	///
	/// Wrap additions made while a level runs, such as
	/// [`NetworkStringTable::add`], in this, as the game does around its own
	/// additions.
	///
	/// [`NetworkStringTable::add`]: crate::interfaces::network_string_tables::NetworkStringTable::add
	#[doc(alias = "LockNetworkStringTables")]
	pub fn with_unlocked_string_tables<R>(self, f: impl FnOnce() -> R) -> R {
		/// Restores the tables' previous lock state when dropped.
		struct Restore<'s> {
			engine: ValveEngine<'s>,
			locked: bool,
		}

		impl Drop for Restore<'_> {
			fn drop(&mut self) {
				self.engine.lock_network_string_tables(self.locked);
			}
		}

		let _restore = Restore {
			engine: self,
			locked: self.lock_network_string_tables(false),
		};

		f()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::ptr::null_mut;

	/// The game directory the mock engine reports.
	const GAME_DIR: &CStr = c"C:/srcds/tf";

	thread_local! {
		static RECEIVED_THIS: Cell<*mut sys::IVEngineServer> = const { Cell::new(null_mut()) };
		static RECEIVED_COMMAND: Cell<*const c_char> = const { Cell::new(ptr::null()) };
	}

	#[test]
	fn methods_forward_the_interface_and_arguments() {
		let vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_ServerCommand).write(record_server_command);
					(&raw mut (*vtable).IVEngineServer_GetGameDir).write(write_game_dir);
				},
			)
		};

		let mut interface = sys::IVEngineServer {
			vtable_: &raw const *vtable,
		};
		let interface_pointer = &raw mut interface;
		let command = c"bot\n";
		let engine = unsafe { ValveEngine::from_raw(NonNull::new(interface_pointer).unwrap()) };

		engine.server_command(command);

		assert_eq!(RECEIVED_THIS.get(), interface_pointer);
		assert_eq!(RECEIVED_COMMAND.get(), command.as_ptr());
		assert_eq!(engine.game_dir().as_c_str(), GAME_DIR);
	}

	unsafe extern "C" fn record_server_command(
		this: *mut sys::IVEngineServer,
		command: *const c_char,
	) {
		RECEIVED_THIS.set(this);
		RECEIVED_COMMAND.set(command);
	}

	unsafe extern "C" fn write_game_dir(
		_: *mut sys::IVEngineServer,
		buffer: *mut c_char,
		length: c_int,
	) {
		let length_with_nul = GAME_DIR.to_bytes_with_nul().len();

		assert_eq!(length, MAX_PATH as c_int);
		unsafe { ptr::copy_nonoverlapping(GAME_DIR.as_ptr(), buffer, length_with_nul) };
	}

	mod edict_lookup {
		use super::*;
		use crate::edicts::test_support::mock_edict;
		use std::cell::RefCell;

		/// A stand-in for the engine's edict table and client list.
		struct MockServer {
			table: *mut sys::edict_t,
			table_len: usize,
			/// The edict and user ID of each client, as `GetPlayerUserId` sees them.
			clients: Vec<(*const sys::edict_t, c_int)>,
			/// Whether `PEntityOfEntIndex` returns free slots, which the engine does not.
			returns_free_slots: bool,
			/// Every index passed to `PEntityOfEntIndex`.
			requested: Vec<c_int>,
		}

		impl MockServer {
			const EMPTY: Self = Self {
				table: null_mut(),
				table_len: 0,
				clients: Vec::new(),
				returns_free_slots: false,
				requested: Vec::new(),
			};
		}

		thread_local! {
			static SERVER: RefCell<MockServer> = const { RefCell::new(MockServer::EMPTY) };
		}

		unsafe extern "C" fn edict_of_index(
			_: *mut sys::IVEngineServer,
			index: c_int,
		) -> *mut sys::edict_t {
			SERVER.with_borrow_mut(|server| {
				server.requested.push(index);

				let Some(slot) = usize::try_from(index)
					.ok()
					.filter(|&slot| slot < server.table_len)
				else {
					return null_mut();
				};

				let edict = unsafe { server.table.add(slot) };
				let free = unsafe { Edict::from_raw(NonNull::new(edict).unwrap()) }.is_free();

				if free && !server.returns_free_slots {
					null_mut()
				} else {
					edict
				}
			})
		}

		fn engine(interface: &mut sys::IVEngineServer) -> ValveEngine<'_> {
			unsafe { ValveEngine::from_raw(NonNull::from(interface)) }
		}

		#[test]
		fn invalid_user_ids_from_the_engine_are_rejected() {
			// Slot 3 is a client slot nobody occupies, which the engine reports as 0.
			let mut table = mock_table(5, |_| false);
			serve(&mut table, &[(1, -5), (2, 70_000), (3, 0), (4, 1)], false);
			let (_vtable, mut interface) = mock_engine();
			let engine = engine(&mut interface);
			let user_id = |index| engine.user_id_of_edict(engine.edict_of_index(index).unwrap());

			assert_eq!(user_id(1), None);
			assert_eq!(user_id(2), None);
			assert_eq!(user_id(3), None);
			assert_eq!(user_id(4), UserId::new(1));
		}

		#[test]
		fn lookup_ignores_free_slots_even_if_the_engine_returns_them() {
			let mut table = mock_table(8, |slot| slot == 2);
			serve(&mut table, &[(1, 2), (2, 4)], true);
			let (_vtable, mut interface) = mock_engine();
			let engine = engine(&mut interface);

			assert_eq!(engine.edict_of_user_id(UserId::new(4).unwrap()), None);
			assert_eq!(
				engine
					.edict_of_user_id(UserId::new(2).unwrap())
					.map(Edict::index),
				Some(1)
			);
		}

		fn mock_engine() -> (
			Box<sys::IVEngineServer__bindgen_vtable>,
			Box<sys::IVEngineServer>,
		) {
			let vtable = unsafe {
				mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(edict_of_index);
						(&raw mut (*vtable).IVEngineServer_GetPlayerUserId).write(player_user_id);
					},
				)
			};

			let interface = Box::new(sys::IVEngineServer {
				vtable_: &raw const *vtable,
			});

			(vtable, interface)
		}

		/// Builds an edict table whose slots are free wherever `free` says so.
		fn mock_table(len: usize, free: impl Fn(usize) -> bool) -> Box<[sys::edict_t]> {
			(0..len)
				.map(|slot| mock_edict(slot as c_int, free(slot)))
				.collect()
		}

		#[test]
		fn out_of_range_indices_never_reach_the_engine() {
			let mut table = mock_table(MAX_EDICTS as usize + 1, |_| false);
			serve(&mut table, &[], false);
			let (_vtable, mut interface) = mock_engine();
			let engine = engine(&mut interface);

			assert_eq!(engine.edict_of_index(-1), None);
			assert_eq!(engine.edict_of_index(MAX_EDICTS), None);
			assert_eq!(engine.edict_of_index(c_int::MAX), None);
			assert!(requested_indices().is_empty());

			assert_eq!(
				engine.edict_of_index(MAX_EDICTS - 1).map(Edict::index),
				Some(MAX_EDICTS - 1)
			);
		}

		unsafe extern "C" fn player_user_id(
			_: *mut sys::IVEngineServer,
			edict: *const sys::edict_t,
		) -> c_int {
			SERVER.with_borrow(|server| {
				server
					.clients
					.iter()
					.find(|&&(client_edict, _)| client_edict == edict)
					.map_or(-1, |&(_, user_id)| user_id)
			})
		}

		fn requested_indices() -> Vec<c_int> {
			SERVER.with_borrow(|server| server.requested.clone())
		}

		/// Serves `table` and `clients`, given as `(slot, user ID)`, to the mock engine.
		fn serve(table: &mut [sys::edict_t], clients: &[(usize, c_int)], returns_free_slots: bool) {
			let base = table.as_mut_ptr();

			SERVER.set(MockServer {
				table: base,
				table_len: table.len(),
				clients: clients
					.iter()
					.map(|&(slot, user_id)| (unsafe { base.add(slot) }.cast_const(), user_id))
					.collect(),
				returns_free_slots,
				requested: Vec::new(),
			});
		}

		#[test]
		fn user_ids_and_player_edicts_round_trip() {
			// Slot 2 is an empty player slot, slots 4 onwards hold other entities,
			// and slot 256 would exceed the engine's player limit.
			let clients = [(1, 2), (3, 7), (255, 9), (256, 11)];
			let mut table = mock_table(300, |slot| slot == 2);
			serve(&mut table, &clients, false);
			let (_vtable, mut interface) = mock_engine();
			let engine = engine(&mut interface);

			for (slot, user_id) in [(1, 2), (3, 7), (255, 9)] {
				let user_id = UserId::new(user_id).unwrap();
				let edict = engine.edict_of_user_id(user_id).unwrap();

				assert_eq!(edict.index(), slot);
				assert_eq!(engine.edict_of_index(slot), Some(edict));
				assert_eq!(engine.user_id_of_edict(edict), Some(user_id));
			}

			assert_eq!(engine.edict_of_user_id(UserId::new(11).unwrap()), None);
			assert_eq!(engine.edict_of_user_id(UserId::new(5).unwrap()), None);
			assert_eq!(
				engine.user_id_of_edict(engine.edict_of_index(0).unwrap()),
				None
			);
			assert_eq!(
				engine.user_id_of_edict(engine.edict_of_index(4).unwrap()),
				None
			);
			assert_eq!(engine.edict_of_index(2), None);
			assert!(
				requested_indices()
					.iter()
					.all(|index| (0..=ABSOLUTE_PLAYER_LIMIT).contains(index))
			);
		}
	}

	mod string_table_lock {
		use super::*;
		use std::cell::RefCell;
		use std::panic::{AssertUnwindSafe, catch_unwind};

		thread_local! {
			/// Whether the mock engine's string tables are locked.
			static LOCKED: Cell<bool> = const { Cell::new(true) };

			/// Every interface and state passed to `LockNetworkStringTables`, in
			/// order.
			static REQUESTS: RefCell<Vec<(*mut sys::IVEngineServer, bool)>> =
				const { RefCell::new(Vec::new()) };
		}

		fn engine(interface: &mut sys::IVEngineServer) -> ValveEngine<'_> {
			unsafe { ValveEngine::from_raw(NonNull::from(interface)) }
		}

		unsafe extern "C" fn lock(this: *mut sys::IVEngineServer, lock: bool) -> bool {
			REQUESTS.with_borrow_mut(|requests| requests.push((this, lock)));
			LOCKED.replace(lock)
		}

		#[test]
		fn locking_forwards_the_state_and_returns_the_previous_one() {
			let (_vtable, mut interface) = mock_engine();
			let pointer = &raw mut *interface;
			let engine = engine(&mut interface);

			assert!(engine.lock_network_string_tables(false));
			assert!(!engine.lock_network_string_tables(false));
			assert!(!engine.lock_network_string_tables(true));
			assert!(LOCKED.get());
			assert_eq!(
				requests(),
				[(pointer, false), (pointer, false), (pointer, true)]
			);
		}

		fn mock_engine() -> (
			Box<sys::IVEngineServer__bindgen_vtable>,
			Box<sys::IVEngineServer>,
		) {
			let vtable = unsafe {
				mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IVEngineServer_LockNetworkStringTables).write(lock);
					},
				)
			};

			let interface = Box::new(sys::IVEngineServer {
				vtable_: &raw const *vtable,
			});

			(vtable, interface)
		}

		fn requests() -> Vec<(*mut sys::IVEngineServer, bool)> {
			REQUESTS.with_borrow(Clone::clone)
		}

		#[test]
		fn unlocked_scopes_leave_unlocked_tables_unlocked() {
			let (_vtable, mut interface) = mock_engine();
			let pointer = &raw mut *interface;
			let engine = engine(&mut interface);
			LOCKED.set(false);

			engine.with_unlocked_string_tables(|| assert!(!LOCKED.get()));

			assert!(!LOCKED.get());
			assert_eq!(requests(), [(pointer, false), (pointer, false)]);
		}

		#[test]
		fn unlocked_scopes_restore_the_lock() {
			let (_vtable, mut interface) = mock_engine();
			let pointer = &raw mut *interface;
			let engine = engine(&mut interface);

			let value = engine.with_unlocked_string_tables(|| {
				assert!(!LOCKED.get());
				7
			});

			assert_eq!(value, 7);
			assert!(LOCKED.get());
			assert_eq!(requests(), [(pointer, false), (pointer, true)]);
		}

		#[test]
		fn unlocked_scopes_restore_the_lock_when_they_panic() {
			let (_vtable, mut interface) = mock_engine();
			let pointer = &raw mut *interface;
			let engine = engine(&mut interface);

			let outcome = catch_unwind(AssertUnwindSafe(|| {
				engine.with_unlocked_string_tables(|| {
					if !LOCKED.get() {
						panic!("the scope panicked");
					}
				});
			}));

			assert!(outcome.is_err());
			assert!(LOCKED.get());
			assert_eq!(requests(), [(pointer, false), (pointer, true)]);
		}
	}
}

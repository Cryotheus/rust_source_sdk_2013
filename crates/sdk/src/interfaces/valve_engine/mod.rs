//! `IVEngineServer`, the engine's services for the game server.

use crate::edicts::Edict;
use crate::math::Vector;
use crate::net::NetChannel;
use crate::players::UserId;
use sdk_raw::edicts::MAX_EDICTS;
use sdk_raw::players::ABSOLUTE_PLAYER_LIMIT;
use sdk_raw::tier0::MAX_PATH;
use sdk_raw::util::cstr::{copy_cstr, cstring_from_buffer};
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

interface! {
	/// The engine's services for the game server (`IVEngineServer`).
	#[doc(alias("IVEngineServer"))]
	pub struct ValveEngine(sys::IVEngineServer) = Engine sdk_raw::interfaces::valve_engine::VERSION;
}

impl<'s> ValveEngine<'s> {
	/// The engine's change-tracking record for an edict.
	#[doc(alias("GetChangeAccessor"))]
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
	#[doc(alias("ChangeLevel"))]
	pub fn change_level(self, map: &CStr, landmark: Option<&CStr>) {
		let landmark = landmark.map_or(ptr::null(), CStr::as_ptr);

		// SAFETY: `Server::new` guarantees the interface is live.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_ChangeLevel(map.as_ptr(), landmark)) };
	}

	/// Whether any part of a box in world space lies in one of the clusters of
	/// `pvs`.
	#[doc(alias("CheckBoxInPVS"))]
	pub fn check_box_in_pvs(self, mins: Vector, maxs: Vector, pvs: &Pvs<'_>) -> bool {
		// The engine would read past the end of an empty set, which holds no
		// cluster anyway.
		if pvs.bits.is_empty() {
			return false;
		}

		let mins = sys::Vector::from(mins);
		let maxs = sys::Vector::from(maxs);

		// SAFETY: As for `change_level`, and the corners are locals. The set holds
		// a bit for each of the level's clusters, and its length is passed.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_CheckBoxInPVS(&mins, &maxs, pvs.bits.as_ptr(), pvs.len_c_int()))
		}
	}

	/// Whether a point lies in one of the clusters of `pvs`. A point outside the
	/// world, or inside its solid parts, lies in none.
	#[doc(alias("CheckOriginInPVS"))]
	pub fn check_origin_in_pvs(self, origin: Vector, pvs: &Pvs<'_>) -> bool {
		// As for `check_box_in_pvs`.
		if pvs.bits.is_empty() {
			return false;
		}

		let origin = sys::Vector::from(origin);

		// SAFETY: As for `check_box_in_pvs`.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_CheckOriginInPVS(&origin, pvs.bits.as_ptr(), pvs.len_c_int()))
		}
	}

	/// The value a client reported for one of its user settings, the console
	/// variables marked `FCVAR_USERINFO`, such as `name` or `cl_interp`.
	///
	/// The engine reports an unknown setting, or an edict that no connected
	/// client owns, as empty. Returns `None` if the engine returns null.
	#[doc(alias("GetClientConVarValue"))]
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
	#[doc(alias("ClientPrintf"))]
	pub fn client_print(self, client: Edict<'_>, message: &CStr) {
		// SAFETY: As for `change_level`, and the edict is live. The engine
		// ignores edicts that no connected client owns.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_ClientPrintf(client.as_ptr(), message.as_ptr()))
		};
	}

	/// The 64-bit Steam ID of the client owning an edict.
	///
	/// A remote client's ID comes from the Steam ticket it connected with, so the
	/// engine has it by the time the game's `ClientConnect` runs, where TF2 reads
	/// it too. Steam checks the ticket afterwards, and the engine disconnects the
	/// client if the check fails.
	///
	/// Returns `None` for an edict that no connected client owns, and for an ID
	/// that `CSteamID::IsValid` would reject, such as the cleared ID of a fake
	/// client the engine does not report to Steam.
	#[doc(alias("GetClientSteamID"))]
	pub fn client_steam_id(self, client: Edict<'_>) -> Option<u64> {
		// SAFETY: As for `change_level`, and the edict is live. The engine checks
		// that a connected client owns it, and returns null otherwise.
		let steam_id =
			unsafe { vcall!(self.as_ptr() => IVEngineServer_GetClientSteamID(client.as_ptr())) };

		// SAFETY: A non-null result points to the ID the engine's client holds,
		// which lives as long as the client and is copied at once, and the union's
		// bits always make a valid `u64`.
		let steam_id = unsafe { steam_id.as_ref()?.m_steamid.m_unAll64Bits };

		is_valid_steam_id(steam_id).then_some(steam_id)
	}

	/// The visibility cluster of the level's map a point lies in, or `None` for a
	/// point outside the world or inside its solid parts.
	#[doc(alias("GetClusterForOrigin"))]
	pub fn cluster_for_origin(self, origin: Vector) -> Option<Cluster> {
		let origin = sys::Vector::from(origin);

		// SAFETY: As for `change_level`, and the position is a local.
		let cluster =
			unsafe { vcall!(self.as_ptr() => IVEngineServer_GetClusterForOrigin(&origin)) };

		(cluster >= 0).then_some(Cluster(cluster))
	}

	/// Offsets the crosshair of the client owning an edict, in degrees.
	#[doc(alias("CrosshairAngle"))]
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
	#[doc(alias("PEntityOfEntIndex", "INDEXENT"))]
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

	/// The number of edicts in use (`GetEntityCount`), one for each networked
	/// entity, of the [`MAX_EDICTS`] the edict table holds. Server-only
	/// entities take none.
	///
	/// The count is optimistic about how many more entities the engine can
	/// network: it reuses a freed edict only once a second has passed since it
	/// was freed, and stops the server with a fatal error when it needs an
	/// edict while the table holds none it can use.
	#[doc(alias("GetEntityCount"))]
	pub fn entity_count(self) -> c_int {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_GetEntityCount()) }
	}

	/// The map `name` names, as `changelevel` and TF2's own map votes find
	/// maps: its canonical name, and how it was found, or `None` if no map has
	/// the name, nor one starting with it.
	///
	/// A map that is [`FoundMap::PossiblyAvailable`] may still be downloaded
	/// as a level changes to it, as workshop maps are.
	#[doc(alias("FindMap"))]
	pub fn find_map(self, name: &CStr) -> Option<(CString, FoundMap)> {
		let name = name.to_bytes();
		let mut buffer = [0 as c_char; MAX_PATH];

		// Room for the name's NUL, which the zeroed buffer already holds.
		if name.len() >= MAX_PATH {
			return None;
		}

		for (to, &from) in buffer.iter_mut().zip(name) {
			*to = from as c_char;
		}

		// SAFETY: As for `change_level`. The engine writes at most the
		// buffer's length, its NUL included.
		let found = unsafe {
			vcall!(self.as_ptr() => IVEngineServer_FindMap(buffer.as_mut_ptr(), MAX_PATH as c_int))
		};

		let found = match found {
			sys::IVEngineServer_eFindMapResult_eFindMap_Found => FoundMap::Exact,
			sys::IVEngineServer_eFindMapResult_eFindMap_FuzzyMatch => FoundMap::Fuzzy,
			sys::IVEngineServer_eFindMapResult_eFindMap_NonCanonical => FoundMap::NonCanonical,
			sys::IVEngineServer_eFindMapResult_eFindMap_PossiblyAvailable => {
				FoundMap::PossiblyAvailable
			}
			_ => return None,
		};

		Some((cstring_from_buffer(&buffer), found))
	}

	/// The path of the game directory, such as `.../tf`.
	#[doc(alias("GetGameDir"))]
	pub fn game_dir(self) -> CString {
		let mut buffer = [0 as c_char; MAX_PATH];

		// SAFETY: As for `change_level`, and the buffer length is passed.
		unsafe {
			vcall!(self.as_ptr() => IVEngineServer_GetGameDir(buffer.as_mut_ptr(), MAX_PATH as c_int))
		};

		cstring_from_buffer(&buffer)
	}

	/// Whether a map file exists and can be loaded, such as `maps/ctf_2fort.bsp`.
	#[doc(alias("IsMapValid"))]
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
	#[doc(alias("LockNetworkStringTables"))]
	pub fn lock_network_string_tables(self, lock: bool) -> bool {
		// SAFETY: As for `change_level`. The game itself sets both states during
		// ordinary play, such as around a player's first spawn.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_LockNetworkStringTables(lock)) }
	}

	/// Writes a line to the server log, as the `log` command does.
	#[doc(alias("LogPrint"))]
	pub fn log_print(self, message: &CStr) {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_LogPrint(message.as_ptr())) };
	}

	/// The net channel of the client owning an edict.
	///
	/// Returns `None` for an edict that no connected client owns, and for fake
	/// clients such as bots and SourceTV, which have no channel.
	#[doc(alias("GetPlayerNetInfo"))]
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

	/// Tells the engine that an edict's [`FL_EDICT_DONTSEND`] flag changed, as
	/// `CBaseEntity::SetTransmitState` does, for a listen server's local client.
	///
	/// [`FL_EDICT_DONTSEND`]: sdk_raw::edicts::FL_EDICT_DONTSEND
	#[doc(alias("NotifyEdictFlagsChange"))]
	pub(crate) fn notify_edict_flags_change(self, edict: Edict<'_>) {
		// SAFETY: As for `change_level`. The engine only reads the index.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_NotifyEdictFlagsChange(edict.index())) };
	}

	/// The network ID of the client owning an edict, such as a rendered Steam
	/// ID or `BOT`.
	///
	/// Returns `None` for an edict that no client owns, which covers the edict
	/// of every entity that is not a player. The edict of an empty player slot
	/// can still report an ID, such as `STEAM_ID_PENDING`.
	#[doc(alias("GetPlayerNetworkIDString"))]
	pub fn player_network_id(self, edict: Edict<'_>) -> Option<CString> {
		// SAFETY: As for `user_id_of_edict`. The engine renders the ID into a
		// buffer it reuses, so it is copied immediately.
		unsafe {
			copy_cstr(
				vcall!(self.as_ptr() => IVEngineServer_GetPlayerNetworkIDString(edict.as_ptr())),
			)
		}
	}

	/// The potentially visible set of a cluster: every cluster that can be seen
	/// from somewhere inside it, itself included.
	#[doc(alias("GetPVSForCluster"))]
	pub fn pvs_for_cluster(self, cluster: Cluster) -> Pvs<'s> {
		self.pvs_for_cluster_index(cluster.0)
	}

	/// The potentially visible set of a cluster, or an empty set for `-1`, as
	/// the engine fills it.
	fn pvs_for_cluster_index(self, cluster: c_int) -> Pvs<'s> {
		let mut bits = vec![0u8; PVS_CAPACITY].into_boxed_slice();

		// SAFETY: As for `change_level`. The buffer holds a bit for each cluster
		// a map can have, as the game's own callers pass, and its length is
		// passed.
		let length = unsafe {
			vcall!(self.as_ptr() => IVEngineServer_GetPVSForCluster(cluster, PVS_CAPACITY as c_int, bits.as_mut_ptr()))
		};

		let length = usize::try_from(length).unwrap_or(0).min(PVS_CAPACITY);

		Pvs {
			bits: bits[..length].into(),
			_level: PhantomData,
		}
	}

	/// The potentially visible set of the cluster a point lies in, as the engine
	/// networks entities to a client whose view is at the point. Empty for a
	/// point outside the world or inside its solid parts.
	pub fn pvs_for_origin(self, origin: Vector) -> Pvs<'s> {
		self.pvs_for_cluster_index(
			self.cluster_for_origin(origin)
				.map_or(-1, |cluster| cluster.0),
		)
	}

	/// Queues a command as though it were entered at the server console.
	///
	/// The engine runs queued commands at the start of the next frame, or when
	/// [`Self::server_execute`] runs them. It rejects text that does not end in
	/// a newline or `;`, printing "Error, bad server command" instead, so end
	/// `command` with one. Several commands can be queued at once, separated by
	/// newlines or `;`.
	#[doc(alias("ServerCommand"))]
	pub fn server_command(self, command: &CStr) {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_ServerCommand(command.as_ptr())) };
	}

	/// Runs the commands queued in the server's command buffer, such as those
	/// [`Self::server_command`] queued, now rather than at the start of the
	/// next frame.
	///
	/// The engine runs its buffer only once at a time, so this does nothing
	/// while the buffer is already running: inside a console command's
	/// callback, for example. A queued `wait` ends the run, leaving the
	/// commands after it for a later frame.
	#[doc(alias("ServerExecute"))]
	pub fn server_execute(self) {
		// SAFETY: As for `change_level`.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_ServerExecute()) };
	}

	/// Renders a client's view from another entity, such as a camera, or from
	/// its own player again.
	#[doc(alias("SetView"))]
	pub fn set_view(self, client: Edict<'_>, view: Edict<'_>) {
		// SAFETY: As for `change_level`, and both edicts are live. The engine
		// ignores edicts that no connected client owns.
		unsafe { vcall!(self.as_ptr() => IVEngineServer_SetView(client.as_ptr(), view.as_ptr())) };
	}

	/// The engine's per-frame record of changed network variables.
	#[doc(alias("GetSharedEdictChangeInfo"))]
	pub(crate) fn shared_edict_change_info(self) -> Option<NonNull<sys::CSharedEdictChangeInfo>> {
		// SAFETY: As for `change_level`.
		NonNull::new(unsafe { vcall!(self.as_ptr() => IVEngineServer_GetSharedEdictChangeInfo()) })
	}

	/// Returns the user ID of the player whose client owns an edict.
	///
	/// Returns `None` for an edict that no connected client owns, which covers
	/// the edict of every entity that is not a player.
	#[doc(alias("GetPlayerUserId"))]
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
	#[doc(alias("LockNetworkStringTables"))]
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

/// Bytes that hold a bit for each cluster a map can have, `MAX_MAP_CLUSTERS`
/// in `public/bspfile.h`, as the game's own callers of `GetPVSForCluster`
/// size their buffers.
const PVS_CAPACITY: usize = 65536 / 8;

/// A visibility cluster of the level's map: leaves of its BSP tree that the
/// map's compiler computed visibility for together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cluster(c_int);

impl Cluster {
	/// The cluster's index, which is also its bit in a [`Pvs`].
	pub const fn index(self) -> usize {
		self.0 as usize
	}
}

/// How [`ValveEngine::find_map`] found a map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(alias("eFindMapResult"))]
pub enum FoundMap {
	/// A map of exactly the name.
	Exact,

	/// A map whose name starts with the name, such as `cp_dustbowl` for
	/// `cp_dust`.
	Fuzzy,

	/// A map the name is another name of, such as
	/// `workshop/cp_qualified_name.ugc1234` for `workshop/1234`.
	NonCanonical,

	/// No map yet, but one the server may be able to download as a level
	/// changes to it.
	PossiblyAvailable,
}

/// A potentially visible set: a bit for each visibility cluster of the level's
/// map, set for the clusters that can be seen from where the set was taken.
///
/// The engine networks an entity to a client only while the entity is in the
/// set of the client's view, or is always sent. A set describes the map of
/// the level it was taken in, so it cannot outlive the call from the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pvs<'s> {
	bits: Box<[u8]>,
	_level: PhantomData<ValveEngine<'s>>,
}

impl Pvs<'_> {
	/// The set's bits, eight clusters to a byte, starting from the lowest bit.
	pub fn as_bytes(&self) -> &[u8] {
		&self.bits
	}

	/// Whether the set holds a cluster.
	pub fn contains(&self, cluster: Cluster) -> bool {
		let index = cluster.index();

		self.bits
			.get(index / 8)
			.is_some_and(|byte| byte & (1 << (index % 8)) != 0)
	}

	/// Whether the set holds no cluster, as for a view outside the world.
	pub fn is_empty(&self) -> bool {
		self.bits.iter().all(|&byte| byte == 0)
	}

	/// The set's length for the engine, which a map's cluster count keeps far
	/// below `c_int::MAX`.
	fn len_c_int(&self) -> c_int {
		self.bits.len() as c_int
	}

	/// Adds every cluster of `other` to this set, as the engine merges the sets
	/// of a client's views.
	pub fn union_with(&mut self, other: &Pvs<'_>) {
		// An empty set can be shorter than one taken from a cluster.
		if other.bits.len() > self.bits.len() {
			let mut bits = vec![0u8; other.bits.len()].into_boxed_slice();

			bits[..self.bits.len()].copy_from_slice(&self.bits);
			self.bits = bits;
		}

		for (byte, other) in self.bits.iter_mut().zip(&other.bits) {
			*byte |= other;
		}
	}
}

/// Whether a 64-bit Steam ID is valid, as `CSteamID::IsValid` decides: its
/// universe and account type are known ones, and those of individual
/// accounts, groups and game servers name an account, the first two in the
/// one instance they use.
fn is_valid_steam_id(steam_id: u64) -> bool {
	// `CSteamID`'s fields, from its lowest bits.
	let account = steam_id as u32;
	let instance = (steam_id >> 32) as u32 & 0xF_FFFF;
	let account_type = (steam_id >> 52) as u32 & 0xF;
	let universe = (steam_id >> 56) as u32;

	// From `k_EUniversePublic` to `k_EUniverseDev`.
	let known_universe = (1..=4).contains(&universe);

	known_universe
		&& match account_type {
			// `k_EAccountTypeIndividual`, in `k_unSteamUserDefaultInstance`.
			1 => account != 0 && instance == 1,

			// `k_EAccountTypeGameServer`, in any instance.
			3 => account != 0,

			// `k_EAccountTypeClan`.
			7 => account != 0 && instance == 0,

			// The other types, up to `k_EAccountTypeAnonUser`.
			2..=10 => true,

			// `k_EAccountTypeInvalid`, and types Steam does not know.
			_ => false,
		}
}

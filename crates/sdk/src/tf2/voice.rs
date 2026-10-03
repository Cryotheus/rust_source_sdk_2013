//! TF2 voice lines: making a player or bot speak a specific line.
//!
//! TF2's voice lines are choreographed scenes, `.vcd` files compiled into
//! `scenes/scenes.image`, between which TF2's response rules
//! (`scripts/talker/*.txt`) choose, such as `scenes/Player/Scout/low/508.vcd`
//! for one of the Scout's thanks. A [`Speaker`] plays one line by its
//! [`ScenePath`], including lines that no voice menu item plays.
//!
//! # Precaching
//!
//! A scene precaches the sounds it speaks as its entity spawns
//! (`PrecacheInstancedScene` in `game/server/sceneentity.cpp`). TF2 precaches
//! every scene its response rules can choose when the level starts, so those
//! lines need no precaching by the plugin. Any other line is precached when
//! it is first played, in the middle of the level: the game precaches its
//! sounds anyway, but warns `Late precache of ...` for each sample the engine
//! reports as not precached yet, and whether clients hear that first play has
//! not been tested. If a line's sound script entry is known, such as
//! `Scout.Thanks01`, precache it when the level starts with
//! [`sound::precache_script_sound`](crate::tf2::sound::precache_script_sound).
//!
//! # Scenes string table
//!
//! Each scene played adds its path to the level's `Scenes` network string
//! table (`PrecacheInstancedScene`), even a path that `scenes.image` lacks,
//! and the path stays there until the level ends. The table holds 8192 paths
//! (`MAX_CHOREO_SCENES_STRINGS` in `game/server/networkstringtable_gamedll.h`),
//! of which TF2's own lines already take thousands. Once it is full, the game
//! can add no more, and a new scene's 13-bit networked index
//! (`m_nSceneStringIndex`) names another scene's path to clients. A missing
//! path is also kept in the game's list of missing-scene warnings for as long
//! as the server runs.
//!
//! So do not play paths taken from untrusted input, such as a line number
//! given in a chat command, as they are: check them against a known set of
//! lines, or limit how many distinct paths can be played.
//!
//! # Bots
//!
//! Bots speak scenes as players do, and the players near them hear the
//! lines. Bots themselves hear nothing, as they have no client.
//!
//! # Audience
//!
//! The game plays a scene's speech to the clients within earshot of the
//! speaker, the speaker's own included, and fades it with distance
//! (`CSceneEntity::DispatchStartSpeak`). Narrowing that audience needs a
//! recipient filter with the layout of the game's `CRecipientFilter`, since
//! `CSceneEntity::SetRecipientFilter` casts the filter it is given to one and
//! copies it, so none is offered yet. To play a line's sound alone to chosen
//! clients, from their own view, broadcast its sound script entry, such as
//! `Scout.Thanks01`, with [`sound::broadcast`](crate::tf2::sound::broadcast).
//!
//! # Unverified
//!
//! On TF2's 64-bit Windows server, [`Speaker::play_scene`] and
//! [`Speaker::play_scene_scripted`] have been observed to play one bot line,
//! the Scout's `508.vcd`, alike: a client hears it and sees the bot's face
//! move. A client also hears a line its own player speaks. Both return the
//! line's length, and [`VoiceError::MissingScene`] for a scene that does not
//! exist. Other scenes, captions, whether clients hear the first play of a
//! late-precached line, and either way of playing a scene on Linux, whose
//! vtables are laid out differently, have not been tested.

use crate::entities::{Entity, data_fields, data_map_class};
use crate::tf2::PlayerClass;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::{Game, Server};
use sdk_raw::util::cstr::borrow_cstr;
use sdk_raw::vcall;
use std::ffi::{CStr, CString};
use std::fmt::Display;
use std::ptr::null_mut;
use std::time::Duration;

/// `LIFE_ALIVE` from `public/const.h`: the `m_lifeState` of a living entity.
const LIFE_ALIVE: u8 = 0;

/// An exclusive bound on the offsets of `CBaseEntity`'s own fields, past which
/// an offset its datamap gives is not trusted.
const MAX_FIELD_OFFSET: usize = 8192;

/// The post-speak delay (`flPostDelay`) scenes are played with. The game adds
/// it only to the time an NPC is marked as speaking
/// (`CSceneEntity::DispatchStartSpeak`), so it would change nothing for TF2's
/// players and bots.
const NO_POST_SPEAK_DELAY: f32 = 0.0;

/// The path of a scene, as TF2 finds it in `scenes/scenes.image`, such as
/// `scenes/Player/Scout/low/508.vcd`.
///
/// A path is checked against what the game keeps of it. The game copies it
/// into a buffer of 128 bytes (`m_szInstanceFilename`, sized by
/// `CChoreoScene::MAX_SCENE_FILENAME`), which would silently truncate a
/// longer one, so a path has at most [`MAX_LEN`](Self::MAX_LEN) bytes. It
/// starts with `scenes/` and ends with `.vcd`, both in any case, and consists
/// of printable ASCII characters other than space.
///
/// TF2 replaces either kind of slash with the platform's separator before it
/// looks a scene up (`CSceneEntity::LoadScene`), and its response rules spell
/// the same scene in different cases, so `/` and `\` are interchangeable, and
/// case does not matter to the lookup. A path is kept as it is given, though,
/// and equality and hashing compare its bytes exactly, so two spellings of
/// one scene are unequal. Spelling paths as TF2's response rules do, as
/// [`voice_line`](Self::voice_line) does, keeps them comparable.
///
/// Each distinct path played stays in the level's `Scenes` string table until
/// the level ends, even a path that `scenes.image` lacks, and the table has
/// room for only so many: check paths built from untrusted input against a
/// known set, as the
/// [module documentation](crate::tf2::voice#scenes-string-table) describes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScenePath(CString);

impl ScenePath {
	/// The longest path, in bytes, that the game keeps whole: 127, leaving
	/// room for the terminator in its 128-byte buffer.
	#[doc(alias = "MAX_SCENE_FILENAME")]
	pub const MAX_LEN: usize = 127;

	/// Checks and copies a path.
	pub fn new(path: &CStr) -> Result<Self, ScenePathError> {
		Self::checked(path.to_owned())
	}

	/// Checks an owned path.
	fn checked(path: CString) -> Result<Self, ScenePathError> {
		let bytes = path.to_bytes();

		if bytes.len() > Self::MAX_LEN {
			return Err(ScenePathError::TooLong { len: bytes.len() });
		}

		if !bytes.iter().all(u8::is_ascii_graphic) {
			return Err(ScenePathError::InvalidCharacter);
		}

		match bytes.split_at_checked(6) {
			Some((directory, [b'/' | b'\\', ..])) if directory.eq_ignore_ascii_case(b"scenes") => {}
			_ => return Err(ScenePathError::NotInScenes),
		}

		match bytes.split_last_chunk::<4>() {
			Some((_, extension)) if extension.eq_ignore_ascii_case(b".vcd") => Ok(Self(path)),
			_ => Err(ScenePathError::NotVcd),
		}
	}

	/// The path of one of a class's voice lines by its name in the class's
	/// directory, as TF2's response rules spell it:
	/// `scenes/Player/<class>/low/<line>.vcd`.
	///
	/// Most lines are numbered, such as the Scout's thanks, 508; some are
	/// named, such as `cm_scout_gamewon_01`. The line's name must consist of
	/// ASCII letters, digits, `_` and `-`. Whether a scene exists by that name
	/// is only known when it is played, and playing it takes an entry of the
	/// level's `Scenes` string table whether it exists or not, so check a
	/// name taken from untrusted input against the lines expected first (see
	/// the [module documentation](crate::tf2::voice#scenes-string-table)).
	pub fn voice_line(class: PlayerClass, line: impl Display) -> Result<Self, ScenePathError> {
		let line = line.to_string();

		if line.is_empty()
			|| !line
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
		{
			return Err(ScenePathError::InvalidLineName);
		}

		let path = format!("scenes/Player/{}/low/{line}.vcd", class.scene_directory());

		// The checked name and the directories have no NUL.
		Self::checked(CString::new(path).map_err(|_| ScenePathError::InvalidLineName)?)
	}

	/// The path as the game takes it.
	pub fn as_c_str(&self) -> &CStr {
		&self.0
	}
}

/// Why a [`ScenePath`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScenePathError {
	/// The path has a character other than printable ASCII, or a space.
	#[error("the scene path has a character other than printable ASCII without spaces")]
	InvalidCharacter,

	/// A voice line's name is empty, or has a character other than ASCII
	/// letters, digits, `_` and `-`.
	#[error("a voice line's name must be ASCII letters, digits, `_` or `-`, and not empty")]
	InvalidLineName,

	/// The path does not start with `scenes/` or `scenes\`.
	#[error("the scene path does not start with `scenes/` or `scenes\\`")]
	NotInScenes,

	/// The path does not end with `.vcd`.
	#[error("the scene path does not end with `.vcd`")]
	NotVcd,

	/// The path is longer than [`ScenePath::MAX_LEN`] bytes.
	#[error("the scene path is {len} bytes long, but the game keeps at most 127")]
	TooLong {
		/// The path's length in bytes, less its terminator.
		len: usize,
	},
}

/// A TF2 player or bot that can be made to speak, within one engine callback.
///
/// Keep the player's [`EntityHandle`](crate::entities::EntityHandle) across
/// callbacks, and make a speaker again in each.
#[derive(Debug, Clone, Copy)]
pub struct Speaker<'s> {
	player: Entity<'s>,
	life_state_offset: usize,
}

impl<'s> Speaker<'s> {
	/// Wraps a player. Fails with [`VoiceError::NotTfPlayer`] unless the
	/// server runs TF2 and `player`'s datamaps include `CTFPlayer`, and with
	/// [`VoiceError::UnsupportedLayout`] if `CBaseEntity`'s datamap lacks a
	/// one-byte `m_lifeState`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, VoiceError> {
		if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
			return Err(VoiceError::NotTfPlayer);
		}

		let life_state_offset = life_state_offset(player).ok_or(VoiceError::UnsupportedLayout)?;

		Ok(Self {
			player,
			life_state_offset,
		})
	}

	/// Fails unless the player can speak a scene now.
	fn check_ready(self) -> Result<(), VoiceError> {
		if self.player.is_marked_for_deletion() {
			Err(VoiceError::MarkedForDeletion)
		} else if !self.is_alive() {
			Err(VoiceError::NotAlive)
		} else {
			Ok(())
		}
	}

	/// Whether the player is alive (`m_lifeState` is `LIFE_ALIVE`), as a
	/// scene requires of its speaker. Players waiting to respawn are not.
	#[doc(alias = "IsAlive")]
	#[doc(alias = "m_lifeState")]
	pub fn is_alive(self) -> bool {
		// SAFETY: `new` found the one-byte field in `CBaseEntity`'s own datamap,
		// which every entity shares through its base, at a plausible offset.
		// The callback keeps the player allocated, and the byte is read without
		// forming a reference, as the game writes it too.
		let state = unsafe {
			self.player
				.as_ptr()
				.byte_add(self.life_state_offset)
				.cast::<u8>()
				.read()
		};

		state == LIFE_ALIVE
	}

	/// Speaks a scene as TF2's response rules do, and returns its length.
	///
	/// This calls the player's `CTFPlayer::PlayScene` with no post-speak delay,
	/// response or recipient filter. The game plays it as a multiplayer scene
	/// (`InstancedScriptedScene` with `bMultiplayer` true, in
	/// `game/server/sceneentity.cpp`): it creates an `instanced_scripted_scene`
	/// entity, which plays the scene in the frames that follow and then removes
	/// itself. The entity is networked, for clients to animate the speaker, if
	/// the scene has flex, expression, gesture or sequence events
	/// (`CSceneEntity::ShouldNetwork`), and the game sends the line's caption to
	/// the players who hear it with captions turned on. On TF2's 64-bit Windows
	/// server, a client has been observed to hear a bot's line and see the
	/// bot's face move; the
	/// [module documentation](crate::tf2::voice#unverified) lists what has not
	/// been tested.
	///
	/// Unlike the voice menu, this is not rate-limited, shows no voice
	/// subtitle, plays no gesture of its own, and does not mark the player as
	/// speaking, so TF2's own lines can talk over it. Each scene takes an edict
	/// while it plays, as each of the game's own voice lines does, and its path
	/// takes an entry of the level's `Scenes` string table until the level
	/// ends, as the
	/// [module documentation](crate::tf2::voice#scenes-string-table) describes.
	///
	/// Creating and spawning the scene entity runs entity-creation and spawn
	/// callbacks synchronously, before this returns: the game's, other
	/// plugins', and this plugin's own, such as hooks of entity creation. They
	/// must keep to the contract of [`Server::new`], which lets them free
	/// entities only through deferred deletion.
	///
	/// # Errors
	///
	/// Fails with [`VoiceError::MarkedForDeletion`] or
	/// [`VoiceError::NotAlive`] before anything is played: the game would play
	/// nothing for a dead speaker, but still report the scene's length. Fails
	/// with [`VoiceError::MissingScene`] if the game reports no length, as for
	/// a scene `scenes.image` does not have. The first time the server misses
	/// a scene, the game warns `Scene '...' missing!`, naming it with its
	/// slashes replaced by the platform's (`MissingSceneWarning`); later misses
	/// of the same scene print only the developer message `... missing from
	/// scenes.image`.
	///
	/// # Taunts
	///
	/// TF2 takes a scene played while it starts a taunt as the taunt's scene
	/// (`CTFPlayer::PlayScene` clears `m_bInitTaunt`), so this must not be
	/// called from a hook inside TF2's taunt code, where the line would
	/// replace the taunt.
	#[doc(alias = "PlayScene")]
	#[doc(alias = "InstancedScriptedScene")]
	pub fn play_scene(self, scene: &ScenePath) -> Result<Duration, VoiceError> {
		self.check_ready()?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` verified the zero-offset CTFPlayer primary base, whose
		// generated TF2 vtable has this entry under the target's ABI. The game
		// copies the path into the scene before returning, and gets no response
		// or filter, which it accepts as null. The scene entity it creates is
		// only ever removed with deferred deletion, in later frames, and the
		// callbacks its creation runs are bound by `Server::new`'s contract.
		let length = unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_PlayScene(
				scene.as_c_str().as_ptr(),
				NO_POST_SPEAK_DELAY,
				null_mut(),
				null_mut(),
			))
		};

		scene_length(length)
	}

	/// Speaks a scene through the `PlayScene` member that `CBaseFlex` exposes
	/// to VScript (`game/server/baseflex.cpp`), and returns its length.
	///
	/// The member's native binding is found by name and checked against the
	/// SDK's signature before it is called, where [`play_scene`] trusts the
	/// generated vtable entry. The member plays a single-player scene
	/// (`bMultiplayer` false): the server runs the speaker's face animation
	/// instead of clients, and networks the scene entity for its flex and
	/// expression events only, not for its gestures
	/// (`CSceneEntity::ShouldNetwork`). On TF2's 64-bit Windows server, a
	/// client has been observed to hear and see one bot line played this way
	/// as it does from [`play_scene`], face movement included; scenes whose
	/// gestures matter may differ.
	///
	/// As with [`play_scene`], the scene's path stays in the level's `Scenes`
	/// string table, and creating the scene entity runs entity-creation and
	/// spawn callbacks synchronously, which must keep to the contract of
	/// [`Server::new`].
	///
	/// # Errors
	///
	/// As for [`play_scene`]. Fails with [`VoiceError::UnsupportedMethod`] if
	/// the player's script class lacks the member or its signature differs
	/// from the SDK's, and with [`VoiceError::Rejected`] if its binding
	/// reports failure.
	///
	/// [`play_scene`]: Self::play_scene
	#[doc(alias = "ScriptPlayScene")]
	pub fn play_scene_scripted(self, scene: &ScenePath) -> Result<Duration, VoiceError> {
		self.check_ready()?;

		// SAFETY: The checked native member plays a scene on this live, living
		// player, as `play_scene` does, with a path the game copies before
		// returning.
		let result = unsafe {
			binding::call(
				self.player,
				c"CBaseFlex",
				c"PlayScene",
				&mut [
					binding::string(scene.as_c_str()),
					binding::float(NO_POST_SPEAK_DELAY),
				],
				binding::FLOAT,
			)?
		};

		// SAFETY: The checked return type selects the float union member.
		scene_length(unsafe { result.__bindgen_anon_1.m_float })
	}

	/// The player who speaks.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}
}

/// Why a [`Speaker`] could not speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VoiceError {
	/// The player is marked for deletion.
	#[error("the player is marked for deletion")]
	MarkedForDeletion,

	/// The game reported no length for the scene, as for one that
	/// `scenes.image` does not have.
	#[error("the scene is not in scenes.image")]
	MissingScene,

	/// The player is not alive, so the scene would not play.
	#[error("the player is not alive")]
	NotAlive,

	/// The entity is not a TF2 player, or the server does not run TF2.
	#[error("voice lines require a TF2 player")]
	NotTfPlayer,

	/// The binding adapter of the player's `PlayScene` member reported
	/// failure.
	#[error("the native PlayScene method rejected its arguments")]
	Rejected,

	/// `CBaseEntity`'s datamap lacks a usable `m_lifeState`.
	#[error("the player's datamap does not describe its life state")]
	UnsupportedLayout,

	/// The player's script class descriptors lack the `PlayScene` member, or
	/// its signature differs from the SDK's.
	#[error("the game does not expose the expected native PlayScene method")]
	UnsupportedMethod,
}

impl From<BindingError> for VoiceError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// The offset of `m_lifeState`, which `CBaseEntity`'s own datamap declares as
/// a one-byte `FIELD_CHARACTER` (`game/server/baseentity.cpp`).
fn life_state_offset(entity: Entity<'_>) -> Option<usize> {
	let map = entity
		.data_maps()
		.find(|map| data_map_class(map) == Some(c"CBaseEntity"))?;
	let field = data_fields(map).iter().find(|field| {
		field.fieldType == sys::_fieldtypes_FIELD_CHARACTER
			// SAFETY: Field names are string literals of the game DLL.
			&& unsafe { borrow_cstr(field.fieldName) } == Some(c"m_lifeState")
	})?;

	if field.fieldSize != 1 || field.fieldSizeInBytes != 1 {
		return None;
	}

	usize::try_from(field.fieldOffset[0])
		.ok()
		.filter(|&offset| offset < MAX_FIELD_OFFSET)
}

/// The length the game reported for a scene it played, or
/// [`VoiceError::MissingScene`] unless it is positive and finite.
fn scene_length(seconds: f32) -> Result<Duration, VoiceError> {
	Duration::try_from_secs_f32(seconds)
		.ok()
		.filter(|length| !length.is_zero())
		.ok_or(VoiceError::MissingScene)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::InterfaceFactory;
	use crate::entities::test_support::{MOCK_EFLAGS_OFFSET, base_entity_fields, data_map, field};
	use crate::server::test_support::mock_server;
	use std::cell::{Cell, RefCell};
	use std::ffi::c_int;
	use std::ffi::{c_char, c_void};
	use std::marker::PhantomData;
	use std::mem::{offset_of, size_of, zeroed};
	use std::ptr::NonNull;

	const PLAY_SCENE: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_PlayScene) / size_of::<usize>();

	const SCRIPT_DESCRIPTION: usize =
		offset_of!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetScriptDesc)
			/ size_of::<usize>();

	thread_local! {
		static DATA_MAP: Cell<*mut sys::datamap_t> = const { Cell::new(null_mut()) };
		static DESCRIPTION: Cell<*mut sys::ScriptClassDesc_t> = const { Cell::new(null_mut()) };
		static LENGTH: Cell<f32> = const { Cell::new(0.0) };
		static PLAYED: RefCell<Vec<(*mut c_void, CString, f32)>> = const { RefCell::new(Vec::new()) };
		static REJECT: Cell<bool> = const { Cell::new(false) };
	}

	/// A player whose fields lie where its datamaps say.
	#[repr(C)]
	struct FakePlayer {
		vtable: *const *const (),
		padding: [usize; 3],
		eflags: c_int,
		more_padding: [u8; 28],
		life_state: u8,
	}

	impl FakePlayer {
		fn new(vtable: &[*const ()]) -> Box<Self> {
			Box::new(Self {
				vtable: vtable.as_ptr(),
				padding: [0; 3],
				eflags: 0,
				more_padding: [0; 28],
				life_state: LIFE_ALIVE,
			})
		}
	}

	unsafe extern "C" fn datamap(_: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		DATA_MAP.get()
	}

	/// The `m_lifeState` declaration of [`FakePlayer`].
	fn life_state_field() -> sys::typedescription_t {
		let mut life_state = field();

		life_state.fieldType = sys::_fieldtypes_FIELD_CHARACTER;
		life_state.fieldName = c"m_lifeState".as_ptr();
		life_state.fieldOffset[0] = offset_of!(FakePlayer, life_state) as c_int;
		life_state.fieldSize = 1;
		life_state.fieldSizeInBytes = 1;
		life_state
	}

	#[test]
	fn living_tf_players_speak_scenes_through_the_generated_vtable() {
		assert_eq!(offset_of!(FakePlayer, eflags), MOCK_EFLAGS_OFFSET);

		let mut vtable = vec![std::ptr::null(); PLAY_SCENE + 1];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[PLAY_SCENE] = play_scene as *const ();

		let player = Box::into_raw(FakePlayer::new(&vtable));
		let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
		let scope = ();
		let server = mock_server(&scope);

		DATA_MAP.set(maps(c"CTFPlayer", Some(life_state_field())));
		PLAYED.take();

		let speaker = Speaker::new(server, entity).unwrap();
		let thanks = ScenePath::voice_line(PlayerClass::Scout, 508).unwrap();

		assert!(speaker.is_alive());
		LENGTH.set(0.8156);
		assert_eq!(
			speaker.play_scene(&thanks),
			Ok(Duration::from_secs_f32(0.8156))
		);
		assert_eq!(
			PLAYED.take(),
			[(
				player.cast(),
				c"scenes/Player/Scout/low/508.vcd".to_owned(),
				0.0
			)]
		);

		// The game reports 0 for a scene it does not have.
		for length in [0.0, -1.0, f32::NAN, f32::INFINITY] {
			LENGTH.set(length);
			assert_eq!(speaker.play_scene(&thanks), Err(VoiceError::MissingScene));
		}

		PLAYED.take();

		// Dead or deleted players are refused before the game is called.
		unsafe { (&raw mut (*player).life_state).write(2) };
		assert!(!speaker.is_alive());
		assert_eq!(speaker.play_scene(&thanks), Err(VoiceError::NotAlive));
		unsafe {
			(&raw mut (*player).life_state).write(LIFE_ALIVE);
			(&raw mut (*player).eflags).write(1);
		}
		assert_eq!(
			speaker.play_scene(&thanks),
			Err(VoiceError::MarkedForDeletion)
		);
		assert!(PLAYED.take().is_empty());
		unsafe { drop(Box::from_raw(player)) };
	}

	/// The datamaps of a `class` deriving from `CBaseEntity`, whose own map has
	/// `life_state` among its fields if given.
	fn maps(
		class: &'static CStr,
		life_state: Option<sys::typedescription_t>,
	) -> *mut sys::datamap_t {
		let mut fields = Vec::from(base_entity_fields());

		fields.extend(life_state);

		let base = data_map(c"CBaseEntity", fields, null_mut());

		data_map(class, vec![], base)
	}

	unsafe extern "C" fn no_interface(_: *const c_char, _: *mut c_int) -> *mut c_void {
		null_mut()
	}

	unsafe extern "C" fn play_scene(
		player: *mut sys::CTFPlayer,
		scene: *const c_char,
		delay: f32,
		response: *mut sys::AI_Response,
		filter: *mut sys::IRecipientFilter,
	) -> f32 {
		assert!(response.is_null());
		assert!(
			filter.is_null(),
			"a filter would be cast to CRecipientFilter"
		);

		let scene = unsafe { CStr::from_ptr(scene) }.to_owned();

		PLAYED.with_borrow_mut(|played| played.push((player.cast(), scene, delay)));
		LENGTH.get()
	}

	unsafe extern "C" fn play_scene_adapter(
		_: sys::ScriptFunctionBindingStorageType_t,
		object: *mut c_void,
		arguments: *mut sys::ScriptVariant_t,
		count: c_int,
		result: *mut sys::ScriptVariant_t,
	) -> bool {
		assert_eq!(count, 2);

		if REJECT.get() {
			return false;
		}

		let (scene, delay) = unsafe {
			(
				CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString).to_owned(),
				(*arguments.add(1)).__bindgen_anon_1.m_float,
			)
		};

		PLAYED.with_borrow_mut(|played| played.push((object, scene, delay)));
		unsafe { result.write(binding::float(LENGTH.get())) };
		true
	}

	#[test]
	fn scene_paths_are_checked_against_what_the_game_keeps() {
		for path in [
			c"scenes/Player/Scout/low/508.vcd",
			c"SCENES\\player\\scout\\low\\508.VCD",
			c"scenes/a.vcd",
		] {
			assert_eq!(ScenePath::new(path).unwrap().as_c_str(), path);
		}

		let longest = CString::new(format!("scenes/{}.vcd", "a".repeat(116))).unwrap();
		let too_long = CString::new(format!("scenes/{}.vcd", "a".repeat(117))).unwrap();

		assert_eq!(longest.as_bytes().len(), ScenePath::MAX_LEN);
		assert!(ScenePath::new(&longest).is_ok());
		assert_eq!(
			ScenePath::new(&too_long),
			Err(ScenePathError::TooLong { len: 128 })
		);

		for path in [c"scenes/a b.vcd", c"scenes/\xC3\xA9.vcd", c"scenes/\t.vcd"] {
			assert_eq!(ScenePath::new(path), Err(ScenePathError::InvalidCharacter));
		}

		for path in [
			c"player/Scout/low/508.vcd",
			c"scenesx/a.vcd",
			c"scenes",
			c"",
		] {
			assert_eq!(ScenePath::new(path), Err(ScenePathError::NotInScenes));
		}

		for path in [c"scenes/a.wav", c"scenes/vcd", c"scenes/"] {
			assert_eq!(ScenePath::new(path), Err(ScenePathError::NotVcd));
		}
	}

	unsafe extern "C" fn script_description(
		_: *mut sys::CBaseEntity,
	) -> *mut sys::ScriptClassDesc_t {
		DESCRIPTION.get()
	}

	#[test]
	fn scripted_scenes_use_the_checked_cbaseflex_member() {
		let mut parameters = [binding::STRING, binding::FLOAT];
		let mut bindings: [sys::ScriptFunctionBinding_t; 1] = unsafe { zeroed() };

		bindings[0].m_desc.m_pszScriptName = c"PlayScene".as_ptr();
		bindings[0].m_desc.m_ReturnType = binding::FLOAT;
		bindings[0].m_desc.m_Parameters = vector(&mut parameters);
		bindings[0].m_flags = 0x01;
		bindings[0].m_pfnBinding = Some(play_scene_adapter);

		let mut flex: sys::ScriptClassDesc_t = unsafe { zeroed() };

		flex.m_pszClassname = c"CBaseFlex".as_ptr();
		flex.m_FunctionBindings = vector(&mut bindings);

		// Changed below only through the pointer `call` reads it by.
		let binding = flex.m_FunctionBindings.m_Memory.m_pMemory;
		let mut player_description: sys::ScriptClassDesc_t = unsafe { zeroed() };

		player_description.m_pszClassname = c"CTFPlayer".as_ptr();
		player_description.m_pBaseDesc = &raw mut flex;

		// The vtable slot `play_scene` would call fails the test if reached.
		let mut vtable = vec![std::ptr::null(); PLAY_SCENE + 1];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
		vtable[SCRIPT_DESCRIPTION] = script_description as *const ();
		vtable[PLAY_SCENE] = sdk_raw::util::mock::unexpected_call as *const ();

		let player = Box::into_raw(FakePlayer::new(&vtable));
		let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
		let scope = ();
		let server = mock_server(&scope);

		DATA_MAP.set(maps(c"CTFPlayer", Some(life_state_field())));
		DESCRIPTION.set(&raw mut player_description);
		PLAYED.take();

		let speaker = Speaker::new(server, entity).unwrap();
		let line = ScenePath::voice_line(PlayerClass::Heavy, "cm_heavy_gamewon_01").unwrap();

		LENGTH.set(2.5);
		assert_eq!(
			speaker.play_scene_scripted(&line),
			Ok(Duration::from_secs_f32(2.5))
		);
		assert_eq!(
			PLAYED.take(),
			[(
				player.cast(),
				c"scenes/Player/Heavy/low/cm_heavy_gamewon_01.vcd".to_owned(),
				0.0
			)]
		);

		LENGTH.set(0.0);
		assert_eq!(
			speaker.play_scene_scripted(&line),
			Err(VoiceError::MissingScene)
		);

		REJECT.set(true);
		assert_eq!(
			speaker.play_scene_scripted(&line),
			Err(VoiceError::Rejected)
		);
		REJECT.set(false);

		unsafe { (&raw mut (*player).life_state).write(1) };
		assert_eq!(
			speaker.play_scene_scripted(&line),
			Err(VoiceError::NotAlive)
		);
		unsafe { (&raw mut (*player).life_state).write(LIFE_ALIVE) };

		// A member whose signature differs is refused before it is called.
		unsafe { (*binding).m_desc.m_ReturnType = binding::VOID };
		assert_eq!(
			speaker.play_scene_scripted(&line),
			Err(VoiceError::UnsupportedMethod)
		);
		DESCRIPTION.set(null_mut());
		assert_eq!(
			speaker.play_scene_scripted(&line),
			Err(VoiceError::UnsupportedMethod)
		);
		PLAYED.take();
		unsafe { drop(Box::from_raw(player)) };
	}

	#[test]
	fn speakers_require_tf_players_with_a_life_state() {
		let mut vtable = vec![std::ptr::null(); PLAY_SCENE + 1];

		vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();

		let player = Box::into_raw(FakePlayer::new(&vtable));
		let entity = unsafe { Entity::from_raw(NonNull::new(player.cast()).unwrap()) };
		let scope = ();
		let server = mock_server(&scope);

		DATA_MAP.set(maps(c"CTFPlayer", Some(life_state_field())));

		let factory = InterfaceFactory::new(no_interface);
		let other_game = unsafe { Server::new(factory, factory, Game::SourceSdk2013, &scope) };

		assert!(matches!(
			Speaker::new(other_game, entity),
			Err(VoiceError::NotTfPlayer)
		));
		assert_eq!(Speaker::new(server, entity).unwrap().player(), entity);

		// TF2's bots derive from `CTFPlayer`, and speak as players do.
		DATA_MAP.set(data_map(
			c"CTFBot",
			vec![],
			maps(c"CTFPlayer", Some(life_state_field())),
		));
		assert!(Speaker::new(server, entity).is_ok());

		DATA_MAP.set(maps(c"CBaseCombatCharacter", Some(life_state_field())));
		assert!(matches!(
			Speaker::new(server, entity),
			Err(VoiceError::NotTfPlayer)
		));

		DATA_MAP.set(maps(c"CTFPlayer", None));
		assert!(matches!(
			Speaker::new(server, entity),
			Err(VoiceError::UnsupportedLayout)
		));

		let mut wide = life_state_field();

		wide.fieldSizeInBytes = 4;

		let mut array = life_state_field();

		array.fieldSize = 2;

		let mut integer = life_state_field();

		integer.fieldType = sys::_fieldtypes_FIELD_INTEGER;

		let mut far = life_state_field();

		far.fieldOffset[0] = MAX_FIELD_OFFSET as c_int;

		for life_state in [wide, array, integer, far] {
			DATA_MAP.set(maps(c"CTFPlayer", Some(life_state)));
			assert!(matches!(
				Speaker::new(server, entity),
				Err(VoiceError::UnsupportedLayout)
			));
		}

		unsafe { drop(Box::from_raw(player)) };
	}

	/// A vector over `values`. Both element pointers come from one
	/// `as_mut_ptr` call, since a second call would invalidate the first.
	fn vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
		let len = c_int::try_from(values.len()).unwrap();
		let elements = values.as_mut_ptr();

		sys::CUtlVector {
			_phantom_0: PhantomData,
			_phantom_1: PhantomData,
			m_Memory: sys::CUtlMemory {
				_phantom_0: PhantomData,
				m_pMemory: elements,
				m_nAllocationCount: len,
				m_nGrowSize: 0,
			},
			m_Size: len,
			m_pElements: elements,
		}
	}

	#[test]
	fn voice_lines_are_built_from_the_class_directory_and_a_checked_name() {
		assert_eq!(
			ScenePath::voice_line(PlayerClass::Scout, 508)
				.unwrap()
				.as_c_str(),
			c"scenes/Player/Scout/low/508.vcd"
		);
		assert_eq!(
			ScenePath::voice_line(PlayerClass::Heavy, "attackMinigun_vocal05")
				.unwrap()
				.as_c_str(),
			c"scenes/Player/Heavy/low/attackMinigun_vocal05.vcd"
		);

		for line in ["", "../508", "508.vcd", "a b", "a\\b", "é"] {
			assert_eq!(
				ScenePath::voice_line(PlayerClass::Spy, line),
				Err(ScenePathError::InvalidLineName)
			);
		}

		assert_eq!(
			ScenePath::voice_line(PlayerClass::Engineer, "a".repeat(100)),
			Err(ScenePathError::TooLong { len: 131 })
		);
	}
}

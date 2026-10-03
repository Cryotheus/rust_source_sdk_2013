//! `IEngineSound`, the server's sound system, and the values a sound is
//! emitted with, from `public/soundflags.h`.
//!
//! [`EngineSound::emit_sound`] plays a precached sample to the clients a
//! [`Recipients`] names. The engine sends each client a sound's parameters in
//! a few bits apiece (`SoundInfo_t::WriteDelta` in `public/soundinfo.h`), so
//! the value types here only hold what survives that encoding, except
//! [`SoundFlags`] built with [`SoundFlags::from_bits_retain`].

use crate::entities::Entity;
use crate::math::Vector;
use crate::user_messages::Recipients;

use sdk_raw::interfaces::engine_sound::{
	CHAN_AUTO, CHAN_BODY, CHAN_ITEM, CHAN_STATIC, CHAN_STREAM, CHAN_VOICE, CHAN_VOICE2,
	CHAN_WEAPON, DEFAULT_SPECIAL_DSP, NO_SPEAKER_ENTITY, PITCH_HIGH, PITCH_LOW, PITCH_NORM,
	SND_CHANGE_PITCH, SND_CHANGE_VOL, SND_DELAY, SND_DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL,
	SND_IGNORE_NAME, SND_IGNORE_PHONEMES, SND_NOFLAGS, SND_SHOULDPAUSE, SND_SPEAKER, SND_STOP,
	SND_STOP_LOOPING, SOUND_FROM_WORLD, VOL_NORM,
};

use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::num::NonZero;
use std::ptr::{self, null_mut};

/// The highest channel the engine's 3-bit encoding of a sound's channel can
/// carry.
const MAX_CHANNEL: c_int = CHAN_VOICE2;

interface! {
	/// The server's sound system (`IEngineSound`).
	#[doc(alias("IEngineSound"))]
	pub struct EngineSound(sys::IEngineSound) = Engine sdk_raw::interfaces::engine_sound::VERSION;
}

/// One of a source's sound channels (`CHAN_*`).
///
/// The engine sends a channel in 3 bits, so the game's channels from
/// `CHAN_VOICE_BASE` (8) up, and `CHAN_REPLACE` (-1), cannot be used.
/// `public/soundflags.h` notes that [`STREAM`](Self::STREAM) and
/// [`STATIC`](Self::STATIC) allocate from the client mixer's static area.
///
/// # Unverified
///
/// The client's mixer, which plays each channel, is not in the SDK. From the
/// channels' and flags' names, a sound on [`AUTO`](Self::AUTO) takes any free
/// channel, and a sound on another dynamic channel replaces the sound the
/// same source plays there, unless emitted with
/// [`SoundFlags::DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL`]. Whether the static
/// area's channels replace sounds by source and channel too is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Channel(c_int);

impl Channel {
	/// `CHAN_AUTO`: any free channel.
	#[doc(alias("CHAN_AUTO"))]
	pub const AUTO: Self = Self(CHAN_AUTO);

	/// `CHAN_BODY`: footsteps and other body sounds.
	#[doc(alias("CHAN_BODY"))]
	pub const BODY: Self = Self(CHAN_BODY);

	/// `CHAN_ITEM`: item pickups and use.
	#[doc(alias("CHAN_ITEM"))]
	pub const ITEM: Self = Self(CHAN_ITEM);

	/// `CHAN_STATIC`: a channel allocated from the static area.
	#[doc(alias("CHAN_STATIC"))]
	pub const STATIC: Self = Self(CHAN_STATIC);

	/// `CHAN_STREAM`: a stream channel allocated from the static or dynamic
	/// area.
	#[doc(alias("CHAN_STREAM"))]
	pub const STREAM: Self = Self(CHAN_STREAM);

	/// `CHAN_VOICE`: speech, as the game plays its voice lines on.
	#[doc(alias("CHAN_VOICE"))]
	pub const VOICE: Self = Self(CHAN_VOICE);

	/// `CHAN_VOICE2`: a second voice channel.
	#[doc(alias("CHAN_VOICE2"))]
	pub const VOICE2: Self = Self(CHAN_VOICE2);

	/// `CHAN_WEAPON`: weapon sounds.
	#[doc(alias("CHAN_WEAPON"))]
	pub const WEAPON: Self = Self(CHAN_WEAPON);

	/// Validates a raw `CHAN_*` value. Returns `None` for any channel the
	/// engine cannot send, outside `CHAN_AUTO` (0) to `CHAN_VOICE2` (7).
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		if raw >= 0 && raw <= MAX_CHANNEL {
			Some(Self(raw))
		} else {
			None
		}
	}

	/// The raw `CHAN_*` value.
	pub const fn to_raw(self) -> c_int {
		self.0
	}
}

/// The pitch a sound plays at, where 100 is unchanged, higher values are
/// higher, and lower values are lower, from 1 to 255.
///
/// `public/engine/IEngineSound.h` calls 70 to 150 the realistic range, and
/// notes that a changed pitch costs the client more to mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pitch(NonZero<u8>);

impl Pitch {
	/// `PITCH_HIGH`: 120.
	#[doc(alias("PITCH_HIGH"))]
	pub const HIGH: Self = Self::from_header(PITCH_HIGH);

	/// `PITCH_LOW`: 95.
	#[doc(alias("PITCH_LOW"))]
	pub const LOW: Self = Self::from_header(PITCH_LOW);

	/// `PITCH_NORM`: 100, the sample's own pitch.
	#[doc(alias("PITCH_NORM"))]
	pub const NORM: Self = Self::from_header(PITCH_NORM);

	/// Validates a pitch. Returns `None` for 0.
	pub const fn new(pitch: u8) -> Option<Self> {
		match NonZero::new(pitch) {
			Some(pitch) => Some(Self(pitch)),
			None => None,
		}
	}

	/// A `PITCH_*` value, which fails to compile in a constant unless it is
	/// from 1 to 255.
	const fn from_header(pitch: c_int) -> Self {
		let narrowed = pitch as u8;

		assert!(narrowed as c_int == pitch);

		match Self::new(narrowed) {
			Some(pitch) => pitch,
			None => panic!("a pitch must not be 0"),
		}
	}

	/// The pitch as a number from 1 to 255.
	pub const fn get(self) -> u8 {
		self.0.get()
	}
}

/// One sound for [`EngineSound::emit_sound`] to play: the parameters of
/// `IEngineSound::EmitSound`.
///
/// [`new`](Self::new) fills in the defaults, which other values can replace
/// with struct update syntax:
///
/// ```
/// # use source_sdk_2013::interfaces::engine_sound::{Channel, SoundEmission, SoundLevel, SoundSource};
/// let emission = SoundEmission {
///     channel: Channel::VOICE,
///     level: SoundLevel::new(95),
///     ..SoundEmission::new(c"vo/scout_thanks01.mp3", SoundSource::World)
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundEmission<'a> {
	/// Where the sound plays from.
	pub source: SoundSource<'a>,

	/// The source's channel the sound plays on.
	pub channel: Channel,

	/// The precached sample, a path under `sound/` such as
	/// `vo/scout_thanks01.mp3`.
	pub sample: &'a CStr,

	/// How loud the sound is at its source.
	pub volume: Volume,

	/// How far the sound carries.
	pub level: SoundLevel,

	/// How the sound changes or stops one already playing.
	pub flags: SoundFlags,

	/// The pitch the sound plays at.
	pub pitch: Pitch,

	/// Where the sound plays, instead of the source's origin.
	pub origin: Option<Vector>,

	/// The game time at which the sound starts, as
	/// [`GlobalVars::current_time`] gives it, or `None` to start now.
	///
	/// The engine sends the delay from now in 13 signed bits of
	/// milliseconds, so it clamps delays to about 4 seconds.
	///
	/// [`GlobalVars::current_time`]: crate::interfaces::player_info_manager::GlobalVars::current_time
	pub sound_time: Option<f32>,

	/// The entity relaying the sound, such as a speaker an `env_microphone`
	/// plays through, usually with [`SoundFlags::SPEAKER`].
	pub speaker: Option<Entity<'a>>,
}

impl<'a> SoundEmission<'a> {
	/// A sample played from `source` with the defaults: [`Channel::AUTO`],
	/// [`Volume::NORM`], [`SoundLevel::NORM`], [`SoundFlags::NONE`], and
	/// [`Pitch::NORM`], with no origin, delay, or speaker.
	pub const fn new(sample: &'a CStr, source: SoundSource<'a>) -> Self {
		Self {
			source,
			channel: Channel::AUTO,
			sample,
			volume: Volume::NORM,
			level: SoundLevel::NORM,
			flags: SoundFlags::NONE,
			pitch: Pitch::NORM,
			origin: None,
			sound_time: None,
			speaker: None,
		}
	}
}

/// Why a sound could not be emitted or stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SoundError {
	/// The origin or start time is infinite or NaN.
	#[error("the sound's origin or start time is not finite")]
	NotFinite,

	/// The source or speaker has no edict, so no client knows it.
	#[error("the entity is not networked, so no client knows it")]
	NotNetworked,
}

bitflags::bitflags! {
	/// The `SND_*` flags a sound is emitted with, from `public/soundflags.h`.
	///
	/// `SND_SPAWNING` has no constant: the game uses it only with the engine,
	/// which never sends it to clients. [`from_bits`](Self::from_bits) returns
	/// `None` for it, and for any bit from `SND_FLAG_BITS_ENCODE` (11) up,
	/// which the engine does not send either.
	/// [`from_bits_retain`](Self::from_bits_retain) keeps such bits, and the
	/// engine is given them as they are.
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct SoundFlags: c_int {
		/// `SND_CHANGE_PITCH`: changes the pitch of the sound already playing.
		#[doc(alias("SND_CHANGE_PITCH"))]
		const CHANGE_PITCH = SND_CHANGE_PITCH;

		/// `SND_CHANGE_VOL`: changes the volume of the sound already playing.
		#[doc(alias("SND_CHANGE_VOL"))]
		const CHANGE_VOLUME = SND_CHANGE_VOL;

		/// `SND_DELAY`: the sound starts after a delay.
		#[doc(alias("SND_DELAY"))]
		const DELAY = SND_DELAY;

		/// `SND_DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL`: plays alongside the
		/// sound already on the channel instead of replacing it.
		#[doc(alias("SND_DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL"))]
		const DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL =
			SND_DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL;

		/// `SND_IGNORE_NAME`: a change or stop applies to every sound of the
		/// source, whatever its sample.
		#[doc(alias("SND_IGNORE_NAME"))]
		const IGNORE_NAME = SND_IGNORE_NAME;

		/// `SND_IGNORE_PHONEMES`: clients ignore the sample's phonemes, the lip
		/// sync data that moves a speaker's mouth.
		#[doc(alias("SND_IGNORE_PHONEMES"))]
		const IGNORE_PHONEMES = SND_IGNORE_PHONEMES;

		/// `SND_NOFLAGS`: no flags, the default.
		#[doc(alias("SND_NOFLAGS"))]
		const NONE = SND_NOFLAGS;

		/// `SND_SHOULDPAUSE`: the sound pauses while the game is paused.
		#[doc(alias("SND_SHOULDPAUSE"))]
		const SHOULD_PAUSE = SND_SHOULDPAUSE;

		/// `SND_SPEAKER`: the sound is replayed through a speaker.
		#[doc(alias("SND_SPEAKER"))]
		const SPEAKER = SND_SPEAKER;

		/// `SND_STOP`: stops the sound.
		#[doc(alias("SND_STOP"))]
		const STOP = SND_STOP;

		/// `SND_STOP_LOOPING`: stops every looping sound of the source.
		#[doc(alias("SND_STOP_LOOPING"))]
		const STOP_LOOPING = SND_STOP_LOOPING;
	}
}

/// How far a sound carries, in decibels (`soundlevel_t`), from 0 to 255.
///
/// Louder sounds fade over longer distances, and [`NONE`](Self::NONE) does not
/// fade at all. The engine's compatibility levels from 256 up, which fade as
/// GoldSrc's did, cannot be expressed.
#[doc(alias("soundlevel_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundLevel(u8);

impl SoundLevel {
	/// `SNDLVL_GUNFIRE`: 140 dB.
	#[doc(alias("SNDLVL_GUNFIRE"))]
	pub const GUNFIRE: Self = Self::from_header(sys::soundlevel_t_SNDLVL_GUNFIRE);

	/// `SNDLVL_IDLE`: 60 dB.
	#[doc(alias("SNDLVL_IDLE"))]
	pub const IDLE: Self = Self::from_header(sys::soundlevel_t_SNDLVL_IDLE);

	/// `SNDLVL_NONE`: 0, heard at the same volume at any distance.
	#[doc(alias("SNDLVL_NONE"))]
	pub const NONE: Self = Self::from_header(sys::soundlevel_t_SNDLVL_NONE);

	/// `SNDLVL_NORM`: 75 dB.
	#[doc(alias("SNDLVL_NORM"))]
	pub const NORM: Self = Self::from_header(sys::soundlevel_t_SNDLVL_NORM);

	/// `SNDLVL_STATIC`: 66 dB.
	#[doc(alias("SNDLVL_STATIC"))]
	pub const STATIC: Self = Self::from_header(sys::soundlevel_t_SNDLVL_STATIC);

	/// `SNDLVL_TALKING`: 80 dB.
	#[doc(alias("SNDLVL_TALKING"))]
	pub const TALKING: Self = Self::from_header(sys::soundlevel_t_SNDLVL_TALKING);

	/// A level in decibels, as the `SNDLVL_<n>dB` constants give them.
	pub const fn new(decibels: u8) -> Self {
		Self(decibels)
	}

	/// A generated `SNDLVL_*` value, which fails to compile in a constant
	/// unless it is from 0 to 255.
	const fn from_header(level: sys::soundlevel_t) -> Self {
		let decibels = level as u8;

		assert!(decibels as sys::soundlevel_t == level);
		Self(decibels)
	}

	/// The level in decibels.
	pub const fn decibels(self) -> u8 {
		self.0
	}
}

/// Where a sound plays from: the entity whose index the engine is given
/// (`iEntIndex`).
///
/// There is no source for each client's own player (`SOUND_FROM_LOCAL_PLAYER`,
/// -1), which `public/engine/IEngineSound.h` calls client-side only.
/// `SoundInfo_t::WriteDelta` sends entity indices up to 31 in 5 unsigned
/// bits, so no client can receive -1. Unless the engine remaps it before
/// sending, which the SDK does not show, clients read entity 31, which is a
/// player on a server with 31 or more player slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundSource<'a> {
	/// A networked entity, which clients play the sound from, at its own
	/// origin unless the emission gives one.
	Entity(Entity<'a>),

	/// The world (`SOUND_FROM_WORLD`, 0), at the emission's origin.
	#[doc(alias("SOUND_FROM_WORLD"))]
	World,
}

impl SoundSource<'_> {
	/// The entity index the engine takes for the source.
	fn to_raw(self) -> Result<c_int, SoundError> {
		match self {
			Self::Entity(entity) => entity
				.edict()
				.map(|edict| edict.index())
				.ok_or(SoundError::NotNetworked),

			Self::World => Ok(SOUND_FROM_WORLD),
		}
	}
}

/// A sound's volume at its source, from silent (0) to the sample's own
/// volume (1).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Volume(f32);

impl Volume {
	/// `VOL_NORM`: the sample's own volume.
	#[doc(alias("VOL_NORM"))]
	pub const NORM: Self = Self(VOL_NORM);

	/// Validates a volume. Returns `None` unless it is from 0 to 1.
	pub const fn new(volume: f32) -> Option<Self> {
		if volume >= 0.0 && volume <= 1.0 {
			Some(Self(volume))
		} else {
			None
		}
	}

	/// The volume as a number from 0 to 1.
	pub const fn get(self) -> f32 {
		self.0
	}
}

impl<'s> EngineSound<'s> {
	/// Plays a precached sample to every client `recipients` names, with the
	/// engine call the game's `CBaseEntity::EmitSound` makes for a sample
	/// path.
	///
	/// The game first offers such a sound to its `env_microphone` entities
	/// (`CEnvMicrophone::OnSoundPlayed`), which can relay it through their
	/// speakers or swallow it. This calls the engine directly, so microphones
	/// never hear the sound, and never relay or swallow it.
	///
	/// The sample is a path under `sound/`, such as `vo/scout_thanks01.mp3`,
	/// which must be [precached](Self::precache_sound). The engine drops a
	/// sample that is not, and prints `SV_StartSound: <sample> not precached`;
	/// [`NetworkStringTables::is_sound_precached`] tells beforehand. Sound
	/// script names, such as `Scout.Thanks01`, are not resolved here. The
	/// engine skips the recipients no client owns, and fake clients.
	///
	/// The emission is checked before anything else, so an empty `recipients`
	/// returns the same errors as any other. Once it passes, an empty
	/// `recipients` returns `Ok` without calling the engine, since no client
	/// would receive the sound.
	///
	/// # Unverified
	///
	/// The engine is given no vector to append the sound's actual origins to
	/// (`pUtlVecOrigins`). Null is the header's default, but every caller in
	/// the game passes a vector. TF2's 64-bit Windows server has been observed
	/// to accept null, both for emissions to fake clients, which it does not
	/// send, and for one a real client played at its source, the client's own
	/// player. Linux servers have not been tested.
	///
	/// [`NetworkStringTables::is_sound_precached`]: crate::interfaces::NetworkStringTables::is_sound_precached
	#[doc(alias("EmitSound"))]
	pub fn emit_sound(
		self,
		recipients: &Recipients,
		emission: &SoundEmission<'_>,
	) -> Result<(), SoundError> {
		let SoundEmission {
			source,
			channel,
			sample,
			volume,
			level,
			flags,
			pitch,
			origin,
			sound_time,
			speaker,
		} = *emission;

		let entity = source.to_raw()?;
		let speaker = match speaker {
			Some(speaker) => SoundSource::Entity(speaker).to_raw()?,
			None => NO_SPEAKER_ENTITY,
		};
		let origin = finite_vector(origin)?;
		let sound_time = match sound_time {
			Some(time) if !time.is_finite() => return Err(SoundError::NotFinite),
			Some(time) => time,
			None => 0.0,
		};

		if recipients.is_empty() {
			return Ok(());
		}

		let filter = recipients.filter();

		// SAFETY: `Server::new` guarantees the interface is live, and binds any
		// code the call reaches, such as other plugins' sound hooks, to free
		// entities only through deferred deletion. The engine reads the filter,
		// sample, and origin during the call, and all of them outlive it. The
		// default special DSP, null direction, and position updates are what the
		// game passes, and `SoundInfo_t::WriteDelta` never sends a direction to
		// clients. The null origins vector is the header's default.
		unsafe {
			vcall!(self.as_ptr() => IEngineSound_EmitSound1(
				filter.as_raw(),
				entity,
				channel.to_raw(),
				sample.as_ptr(),
				volume.get(),
				sys::soundlevel_t::from(level.decibels()),
				flags.bits(),
				c_int::from(pitch.get()),
				DEFAULT_SPECIAL_DSP,
				origin.as_ref().map_or(ptr::null(), ptr::from_ref),
				ptr::null(),
				null_mut(),
				true,
				sound_time,
				speaker,
			));
		}

		Ok(())
	}

	/// What the engine answers when asked whether a sound is in the precache
	/// table.
	///
	/// TF2's 64-bit Windows server has been observed to answer `true` for a
	/// sample it has not precached, which it then refuses to play, so use
	/// [`NetworkStringTables::is_sound_precached`] instead. Linux servers have
	/// not been tested.
	///
	/// [`NetworkStringTables::is_sound_precached`]: crate::interfaces::NetworkStringTables::is_sound_precached
	#[doc(alias("IsSoundPrecached"))]
	pub fn is_sound_precached(self, sample: &CStr) -> bool {
		// SAFETY: As for `precache_sound`.
		unsafe { vcall!(self.as_ptr() => IEngineSound_IsSoundPrecached(sample.as_ptr())) }
	}

	/// Adds a sound to the precache table, which clients load before playing
	/// it. Returns whether the sound could be precached.
	#[doc(alias("PrecacheSound"))]
	pub fn precache_sound(self, sample: &CStr, preload: bool) -> bool {
		// SAFETY: `Server::new` guarantees the interface is live.
		unsafe {
			vcall!(self.as_ptr() => IEngineSound_PrecacheSound(sample.as_ptr(), preload, false))
		}
	}

	/// The length of a sample in seconds, which the engine reads from its
	/// file's header, or `None` if the engine reports no positive, finite
	/// length.
	///
	/// The header notes that MP3 files are not supported, and TF2's 64-bit
	/// Windows server has been observed to report no length for a precached
	/// MP3, `vo/scout_thanks01.mp3`.
	#[doc(alias("GetSoundDuration"))]
	pub fn sound_duration(self, sample: &CStr) -> Option<f32> {
		// SAFETY: As for `precache_sound`. The engine only reads the sample
		// during the call.
		let seconds =
			unsafe { vcall!(self.as_ptr() => IEngineSound_GetSoundDuration(sample.as_ptr())) };

		(seconds.is_finite() && seconds > 0.0).then_some(seconds)
	}

	/// Stops a precached sample playing from a source on a channel, with the
	/// engine call the game's `CBaseEntity::StopSound` makes.
	///
	/// To stop it for only some clients, emit it to them with
	/// [`SoundFlags::STOP`] instead. The engine ignores a sample that is not
	/// precached.
	#[doc(alias("StopSound"))]
	pub fn stop_sound(
		self,
		source: SoundSource<'_>,
		channel: Channel,
		sample: &CStr,
	) -> Result<(), SoundError> {
		let entity = source.to_raw()?;

		// SAFETY: As for `precache_sound`. The engine only reads the sample
		// during the call.
		unsafe {
			vcall!(self.as_ptr() => IEngineSound_StopSound(entity, channel.to_raw(), sample.as_ptr()))
		};

		Ok(())
	}
}

/// Converts an optional vector for the engine, refusing one that is not
/// finite.
fn finite_vector(vector: Option<Vector>) -> Result<Option<sys::Vector>, SoundError> {
	match vector {
		Some(vector) if !vector.is_finite() => Err(SoundError::NotFinite),
		vector => Ok(vector.map(sys::Vector::from)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::edicts::test_support::mock_edict;
	use crate::entities::test_support::{MockEntity, set_networking};
	use crate::user_messages::test_support::recipients;
	use sdk_raw::interfaces::engine_sound::{CHAN_REPLACE, CHAN_USER_BASE, CHAN_VOICE_BASE};
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::cell::{Cell, RefCell};
	use std::ffi::{CString, c_char};
	use std::ptr::NonNull;

	thread_local! {
		static PRECACHED: Cell<bool> = const { Cell::new(true) };
		static EMITTED: RefCell<Vec<Emitted>> = const { RefCell::new(Vec::new()) };
		static STOPPED: RefCell<Vec<(c_int, c_int, CString)>> = const { RefCell::new(Vec::new()) };
		static DURATION: Cell<f32> = const { Cell::new(0.0) };
	}

	/// The arguments `EmitSound` received, with the filter's recipients as the
	/// engine reads them through its vtable.
	#[derive(Debug, Clone, PartialEq)]
	struct Emitted {
		recipients: Vec<c_int>,
		reliable: bool,
		init_message: bool,
		entity: c_int,
		channel: c_int,
		sample: CString,
		volume: f32,
		level: sys::soundlevel_t,
		flags: c_int,
		pitch: c_int,
		special_dsp: c_int,
		origin: Option<Vector>,
		direction_is_null: bool,
		origins_is_null: bool,
		update_positions: bool,
		sound_time: f32,
		speaker: c_int,
	}

	/// A mock of the engine's sound system, which keeps its vtable alive.
	struct MockEngineSound {
		_vtable: Box<sys::IEngineSound__bindgen_vtable>,
		interface: Box<sys::IEngineSound>,
	}

	impl MockEngineSound {
		/// Fills every other slot, including the float-attenuation `EmitSound`,
		/// with a stub that fails the test if called.
		fn new() -> Self {
			// SAFETY: The vtable holds only function pointers.
			let vtable = unsafe {
				mock_vtable::<sys::IEngineSound__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IEngineSound_IsSoundPrecached)
							.write(is_sound_precached);
						(&raw mut (*vtable).IEngineSound_EmitSound1).write(emit_sound);
						(&raw mut (*vtable).IEngineSound_StopSound).write(stop_sound);
						(&raw mut (*vtable).IEngineSound_GetSoundDuration).write(sound_duration);
					},
				)
			};
			let interface = Box::new(sys::IEngineSound {
				vtable_: &raw const *vtable,
			});

			PRECACHED.set(true);
			EMITTED.take();
			STOPPED.take();

			Self {
				_vtable: vtable,
				interface,
			}
		}

		fn engine_sound(&mut self) -> EngineSound<'_> {
			// SAFETY: The mock outlives the borrow.
			unsafe { EngineSound::from_raw(NonNull::from(&mut *self.interface)) }
		}
	}

	#[test]
	fn channels_fit_the_engine_encoding() {
		let channels = [
			Channel::AUTO,
			Channel::WEAPON,
			Channel::VOICE,
			Channel::ITEM,
			Channel::BODY,
			Channel::STREAM,
			Channel::STATIC,
			Channel::VOICE2,
		];

		for (raw, channel) in (0..).zip(channels) {
			assert_eq!(Channel::from_raw(raw), Some(channel));
			assert_eq!(channel.to_raw(), raw);
		}

		for raw in [CHAN_REPLACE, CHAN_VOICE_BASE, CHAN_USER_BASE] {
			assert_eq!(Channel::from_raw(raw), None);
		}
	}

	/// A leaked edict at `index`, for mock entities to report.
	fn edict(index: c_int) -> *mut sys::edict_t {
		Box::into_raw(Box::new(mock_edict(index, false)))
	}

	#[test]
	fn emissions_default_to_the_header_values() {
		let mut mock = MockEngineSound::new();
		let sound = mock.engine_sound();
		let mut speaker = MockEntity::new(9);

		set_networking(null_mut(), edict(9));

		let recipients = recipients(&[2], false);

		sound
			.emit_sound(
				&recipients,
				&SoundEmission::new(c"ambient/alarm.wav", SoundSource::World),
			)
			.unwrap();

		sound
			.emit_sound(
				&recipients,
				&SoundEmission {
					speaker: Some(speaker.entity()),
					flags: SoundFlags::SPEAKER,
					..SoundEmission::new(c"ambient/alarm.wav", SoundSource::World)
				},
			)
			.unwrap();

		EMITTED.with_borrow(|emitted| {
			let defaults = Emitted {
				recipients: vec![2],
				reliable: false,
				init_message: false,
				entity: SOUND_FROM_WORLD,
				channel: 0,
				sample: c"ambient/alarm.wav".to_owned(),
				volume: 1.0,
				level: sys::soundlevel_t_SNDLVL_NORM,
				flags: 0,
				pitch: 100,
				special_dsp: 0,
				origin: None,
				direction_is_null: true,
				origins_is_null: true,
				update_positions: true,
				sound_time: 0.0,
				speaker: -1,
			};

			assert_eq!(
				*emitted,
				[
					defaults.clone(),
					Emitted {
						flags: 1 << 6,
						speaker: 9,
						..defaults
					},
				]
			);
		});
	}

	#[test]
	fn emissions_reach_the_engine_as_its_arguments() {
		let mut mock = MockEngineSound::new();
		let sound = mock.engine_sound();
		let mut source = MockEntity::new(5);

		set_networking(null_mut(), edict(5));

		let emission = SoundEmission {
			channel: Channel::VOICE,
			volume: Volume::new(0.5).unwrap(),
			level: SoundLevel::new(95),
			flags: SoundFlags::STOP_LOOPING | SoundFlags::IGNORE_PHONEMES,
			pitch: Pitch::HIGH,
			origin: Some(Vector::new(1.0, 2.0, 3.0)),
			sound_time: Some(12.5),
			..SoundEmission::new(
				c"vo/scout_thanks01.mp3",
				SoundSource::Entity(source.entity()),
			)
		};

		sound
			.emit_sound(&recipients(&[3, 1], true), &emission)
			.unwrap();

		EMITTED.with_borrow(|emitted| {
			assert_eq!(
				*emitted,
				[Emitted {
					recipients: vec![3, 1],
					reliable: true,
					init_message: false,
					entity: 5,
					channel: 2,
					sample: c"vo/scout_thanks01.mp3".to_owned(),
					volume: 0.5,
					level: 95,
					flags: (1 << 5) | (1 << 8),
					pitch: 120,
					special_dsp: 0,
					origin: Some(Vector::new(1.0, 2.0, 3.0)),
					direction_is_null: true,
					origins_is_null: true,
					update_positions: true,
					sound_time: 12.5,
					speaker: -1,
				}]
			);
		});
	}

	unsafe extern "C" fn emit_sound(
		_: *mut sys::IEngineSound,
		filter: *mut sys::IRecipientFilter,
		entity: c_int,
		channel: c_int,
		sample: *const c_char,
		volume: f32,
		level: sys::soundlevel_t,
		flags: c_int,
		pitch: c_int,
		special_dsp: c_int,
		origin: *const sys::Vector,
		direction: *const sys::Vector,
		origins: *mut sys::CUtlVector<sys::Vector, sys::CUtlMemory<sys::Vector>>,
		update_positions: bool,
		sound_time: f32,
		speaker: c_int,
	) {
		// SAFETY: The wrapper passes a live filter, sample, and origin, which
		// are read as the engine reads them.
		let emitted = unsafe {
			let vtable = (*filter).vtable_;
			let count = ((*vtable).IRecipientFilter_GetRecipientCount)(filter);

			Emitted {
				recipients: (0..count)
					.map(|slot| ((*vtable).IRecipientFilter_GetRecipientIndex)(filter, slot))
					.collect(),
				reliable: ((*vtable).IRecipientFilter_IsReliable)(filter),
				init_message: ((*vtable).IRecipientFilter_IsInitMessage)(filter),
				entity,
				channel,
				sample: CStr::from_ptr(sample).to_owned(),
				volume,
				level,
				flags,
				pitch,
				special_dsp,
				origin: origin.as_ref().map(|&origin| origin.into()),
				direction_is_null: direction.is_null(),
				origins_is_null: origins.is_null(),
				update_positions,
				sound_time,
				speaker,
			}
		};

		EMITTED.with_borrow_mut(|emitted_sounds| emitted_sounds.push(emitted));
	}

	#[test]
	fn invalid_emissions_are_refused_before_the_engine() {
		let mut mock = MockEngineSound::new();
		let sound = mock.engine_sound();
		let mut entity = MockEntity::new(4);
		let sample = c"ui/hint.wav";
		let recipients = recipients(&[1], false);
		let world = SoundEmission::new(sample, SoundSource::World);
		let emit = |emission: SoundEmission<'_>| sound.emit_sound(&recipients, &emission);

		set_networking(null_mut(), null_mut());

		assert_eq!(
			emit(SoundEmission::new(
				sample,
				SoundSource::Entity(entity.entity())
			)),
			Err(SoundError::NotNetworked)
		);
		assert_eq!(
			emit(SoundEmission {
				speaker: Some(entity.entity()),
				..world
			}),
			Err(SoundError::NotNetworked)
		);
		assert_eq!(
			sound.stop_sound(SoundSource::Entity(entity.entity()), Channel::AUTO, sample),
			Err(SoundError::NotNetworked)
		);
		assert_eq!(
			emit(SoundEmission {
				origin: Some(Vector::new(f32::NAN, 0.0, 0.0)),
				..world
			}),
			Err(SoundError::NotFinite)
		);
		assert_eq!(
			emit(SoundEmission {
				origin: Some(Vector::new(0.0, f32::INFINITY, 0.0)),
				..world
			}),
			Err(SoundError::NotFinite)
		);
		assert_eq!(
			emit(SoundEmission {
				sound_time: Some(f32::NAN),
				..world
			}),
			Err(SoundError::NotFinite)
		);

		// Valid, but nobody would hear it.
		assert_eq!(sound.emit_sound(&Recipients::new(), &world), Ok(()));

		EMITTED.with_borrow(|emitted| assert!(emitted.is_empty()));
		STOPPED.with_borrow(|stopped| assert!(stopped.is_empty()));
	}

	unsafe extern "C" fn is_sound_precached(_: *mut sys::IEngineSound, _: *const c_char) -> bool {
		PRECACHED.get()
	}

	#[test]
	fn pitches_exclude_zero() {
		assert_eq!(Pitch::new(0), None);
		assert_eq!(Pitch::new(255).map(Pitch::get), Some(255));
		assert_eq!(Pitch::LOW.get(), 95);
		assert_eq!(Pitch::NORM.get(), 100);
		assert_eq!(Pitch::HIGH.get(), 120);
	}

	unsafe extern "C" fn sound_duration(_: *mut sys::IEngineSound, _: *const c_char) -> f32 {
		DURATION.get()
	}

	#[test]
	fn sound_durations_must_be_positive() {
		let mut mock = MockEngineSound::new();
		let sound = mock.engine_sound();

		DURATION.set(1.5);
		assert_eq!(sound.sound_duration(c"vo/a.wav"), Some(1.5));

		for unknown in [0.0, -1.0, f32::NAN] {
			DURATION.set(unknown);
			assert_eq!(sound.sound_duration(c"vo/a.mp3"), None);
		}
	}

	#[test]
	fn sound_levels_match_the_header() {
		let pairs = [
			(SoundLevel::NONE, sys::soundlevel_t_SNDLVL_NONE),
			(SoundLevel::IDLE, sys::soundlevel_t_SNDLVL_IDLE),
			(SoundLevel::STATIC, sys::soundlevel_t_SNDLVL_STATIC),
			(SoundLevel::NORM, sys::soundlevel_t_SNDLVL_NORM),
			(SoundLevel::TALKING, sys::soundlevel_t_SNDLVL_TALKING),
			(SoundLevel::GUNFIRE, sys::soundlevel_t_SNDLVL_GUNFIRE),
			(SoundLevel::new(95), sys::soundlevel_t_SNDLVL_95dB),
		];

		for (level, raw) in pairs {
			assert_eq!(sys::soundlevel_t::from(level.decibels()), raw);
		}
	}

	unsafe extern "C" fn stop_sound(
		_: *mut sys::IEngineSound,
		entity: c_int,
		channel: c_int,
		sample: *const c_char,
	) {
		// SAFETY: The wrapper passes a live sample.
		let sample = unsafe { CStr::from_ptr(sample) }.to_owned();

		STOPPED.with_borrow_mut(|stopped| stopped.push((entity, channel, sample)));
	}

	#[test]
	fn stops_reach_the_engine() {
		let mut mock = MockEngineSound::new();
		let sound = mock.engine_sound();
		let mut entity = MockEntity::new(7);

		set_networking(null_mut(), edict(7));

		sound
			.stop_sound(
				SoundSource::Entity(entity.entity()),
				Channel::STATIC,
				c"music/loop.wav",
			)
			.unwrap();
		sound
			.stop_sound(SoundSource::World, Channel::VOICE, c"ambient/alarm.wav")
			.unwrap();

		STOPPED.with_borrow(|stopped| {
			assert_eq!(
				*stopped,
				[
					(7, 6, c"music/loop.wav".to_owned()),
					(SOUND_FROM_WORLD, 2, c"ambient/alarm.wav".to_owned()),
				]
			);
		});
	}

	#[test]
	fn volumes_are_finite_fractions() {
		for valid in [0.0, 0.25, 1.0] {
			assert_eq!(Volume::new(valid).map(Volume::get), Some(valid));
		}

		for invalid in [-0.1, 1.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
			assert_eq!(Volume::new(invalid), None);
		}
	}
}

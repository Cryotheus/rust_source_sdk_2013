//! Hand-written ABI of `IEngineSound` that the generated bindings do not
//! describe, and the values sounds are emitted with, from
//! `public/soundflags.h` and `public/engine/IEngineSound.h`.

use std::ffi::{CStr, c_char, c_int};

/// `IEngineSound::EmitSound`'s overload that takes a `soundlevel_t`:
///
/// ```cpp
/// void EmitSound(IRecipientFilter &filter, int iEntIndex, int iChannel,
///     const char *pSample, float flVolume, soundlevel_t iSoundlevel,
///     int iFlags, int iPitch, int iSpecialDSP, const Vector *pOrigin,
///     const Vector *pDirection, CUtlVector<Vector> *pUtlVecOrigins,
///     bool bUpdatePositions, float soundtime, int speakerentity);
/// ```
///
/// MSVC and Itanium order the slots of `EmitSound`'s two overloads
/// differently, so the overload is reached by its generated field,
/// `IEngineSound_EmitSound1`, which has this signature under both ABIs.
#[doc(alias = "EmitSound")]
pub type EmitSoundFn = unsafe extern "C" fn(
	this: *mut sys::IEngineSound,
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
);

// A regeneration that renamed the overloads would fail to compile here,
// instead of passing a sound level as the other overload's float attenuation.
const _: fn(&sys::IEngineSound__bindgen_vtable) -> EmitSoundFn =
	|vtable| vtable.IEngineSound_EmitSound1;

/// `CHAN_AUTO`: any free channel.
pub const CHAN_AUTO: c_int = 0;

/// `CHAN_BODY`: footsteps and other body sounds.
pub const CHAN_BODY: c_int = 4;

/// `CHAN_ITEM`: item pickups and use.
pub const CHAN_ITEM: c_int = 3;

/// `CHAN_REPLACE`, the one negative channel value.
pub const CHAN_REPLACE: c_int = -1;

/// `CHAN_STATIC`: a channel allocated from the static area.
pub const CHAN_STATIC: c_int = 6;

/// `CHAN_STREAM`: a stream channel allocated from the static or dynamic area.
pub const CHAN_STREAM: c_int = 5;

/// `CHAN_USER_BASE`: the first channel allocated to game code.
pub const CHAN_USER_BASE: c_int = CHAN_VOICE_BASE + 128;

/// `CHAN_VOICE`: speech.
pub const CHAN_VOICE: c_int = 2;

/// `CHAN_VOICE_BASE`: the first channel allocated for network voice data.
pub const CHAN_VOICE_BASE: c_int = 8;

/// `CHAN_VOICE2`: a second voice channel.
pub const CHAN_VOICE2: c_int = 7;

/// `CHAN_WEAPON`: weapon sounds.
pub const CHAN_WEAPON: c_int = 1;

/// The special DSP effect sounds are emitted with: the default of
/// `IEngineSound::EmitSound`'s `iSpecialDSP`, which the game's own calls pass.
pub const DEFAULT_SPECIAL_DSP: c_int = 0;

/// The speaker entity of a sound that plays through no speaker: the default
/// of `IEngineSound::EmitSound`'s `speakerentity`.
pub const NO_SPEAKER_ENTITY: c_int = -1;

/// `PITCH_HIGH`: a raised pitch.
pub const PITCH_HIGH: c_int = 120;

/// `PITCH_LOW`: a lowered pitch.
pub const PITCH_LOW: c_int = 95;

/// `PITCH_NORM`: the sample's own pitch.
pub const PITCH_NORM: c_int = 100;

/// `SND_CHANGE_PITCH`: changes the pitch of the sound already playing.
pub const SND_CHANGE_PITCH: c_int = 1 << 1;

/// `SND_CHANGE_VOL`: changes the volume of the sound already playing.
pub const SND_CHANGE_VOL: c_int = 1 << 0;

/// `SND_DELAY`: the sound starts after a delay.
pub const SND_DELAY: c_int = 1 << 4;

/// `SND_DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL`: plays alongside the sound
/// already on the channel instead of replacing it.
pub const SND_DO_NOT_OVERWRITE_EXISTING_ON_CHANNEL: c_int = 1 << 10;

/// `SND_FLAG_BITS_ENCODE`: the number of low flag bits the engine sends to
/// clients.
pub const SND_FLAG_BITS_ENCODE: u32 = 11;

/// `SND_IGNORE_NAME`: a change or stop applies to every sound of the source,
/// whatever its sample.
pub const SND_IGNORE_NAME: c_int = 1 << 9;

/// `SND_IGNORE_PHONEMES`: clients ignore the sample's phonemes.
pub const SND_IGNORE_PHONEMES: c_int = 1 << 8;

/// `SND_NOFLAGS`: no flags.
pub const SND_NOFLAGS: c_int = 0;

/// `SND_SHOULDPAUSE`: the sound pauses while the game is paused.
pub const SND_SHOULDPAUSE: c_int = 1 << 7;

/// `SND_SPAWNING`: the sound is emitted while spawning, a hint between the
/// game and the engine that is never sent to clients.
pub const SND_SPAWNING: c_int = 1 << 3;

/// `SND_SPEAKER`: the sound is replayed through a speaker.
pub const SND_SPEAKER: c_int = 1 << 6;

/// `SND_STOP`: stops the sound.
pub const SND_STOP: c_int = 1 << 2;

/// `SND_STOP_LOOPING`: stops every looping sound of the source.
pub const SND_STOP_LOOPING: c_int = 1 << 5;

/// `SOUND_FROM_LOCAL_PLAYER`: the entity index of each client's own player,
/// which the header calls client-side only.
pub const SOUND_FROM_LOCAL_PLAYER: c_int = -1;

/// `SOUND_FROM_WORLD`: the entity index of the world.
pub const SOUND_FROM_WORLD: c_int = 0;

/// The version string `IEngineSound` is exported and requested under.
///
/// This is `IENGINESOUND_SERVER_INTERFACE_VERSION` from
/// `public/engine/IEngineSound.h`.
#[doc(alias = "IENGINESOUND_SERVER_INTERFACE_VERSION")]
pub const VERSION: &CStr = c"IEngineSoundServer003";

/// `VOL_NORM`: the sample's own volume.
pub const VOL_NORM: f32 = 1.0;

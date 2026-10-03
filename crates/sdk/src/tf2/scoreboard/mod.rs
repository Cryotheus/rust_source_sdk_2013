//! TF2's scoreboard: display overrides kept applied around each game frame,
//! and the few operations that change the game's own scoring state.
//!
//! # Where clients read the scoreboard
//!
//! TF2 clients build the scoreboard from two kinds of networked entities:
//!
//! - `tf_player_manager` (`CTFPlayerResource`), which holds an array per
//!   column with an element per player slot. For each slot with a connected
//!   player, the game recomputes the elements from its own state: most at
//!   every think, ten times a second (`player_resource.cpp:94-101`), the
//!   damage, healing, support and credit columns at most once a second
//!   (`tf_player_resource.cpp:218-230`), and the ping every 2 seconds
//!   (`player_resource.cpp:139-147`). A value written once is overwritten at
//!   the column's next update. Slots without a connected player keep their
//!   values, apart from the connection fields (`player_resource.cpp:162-168`).
//! - `tf_team` (`CTFTeam`), whose score and flag captures the game only
//!   assigns when they change. They are the game's real state.
//!
//! The game reads two of the player columns back. When a player's
//! `m_iTotalScore` changes, it reports the difference to the item servers as
//! Strange "Points Scored" progress, or in Mann vs. Machine to its statistics
//! (`tf_player_resource.cpp:243-258`); autobalance, team scrambles and match
//! results read it as well. The game also averages each new ping with the
//! previous one (`player_resource.cpp:146`).
//!
//! # Display overrides
//!
//! A [`Scoreboard`] keeps what clients see instead of the game's values, and
//! brackets each game frame:
//!
//! 1. [`Scoreboard::before_frame`] writes the game's own values back, so
//!    nothing the game does during the frame sees an override.
//! 2. The game's frame runs, and may change its values.
//! 3. [`Scoreboard::after_frame`] records the game's new values and writes the
//!    overrides again, before the engine sends the frame to clients.
//!
//! Only what clients must receive anew is marked changed for the engine's
//! networking: the engine records at most 19 changed variables per entity and
//! frame, past which it compares the whole entity, which for the player
//! resource is thousands of variables.
//!
//! With Metamod:Source, `MetamodApi::hook_game_frame` and
//! `MetamodApi::hook_game_frame_post` from the `metamod_source` crate run
//! callbacks at those two points.
//!
//! Code that runs between frames, such as console commands, other plugins, and
//! map scripts reacting to either, sees the overrides, and the game keeps what
//! such code computes from them:
//!
//! - The game adds to a team's score and flag captures in place
//!   (`team.cpp:281-284`, `tf_team.h:54`). An increment made between frames,
//!   such as a round won through `mp_forcewin`, or through another plugin or a
//!   script ending the round or firing `tf_gamerules`' `AddRedTeamScore` input,
//!   is added to the value shown, and the sum becomes the game's own
//!   (`teamplayroundbased_gamerules.cpp:2360-2364`). Call
//!   [`Scoreboard::restore_all`] before causing such a change.
//! - When `mvm_wave_complete` fires, the player resource updates every slot at
//!   once (`tf_player_resource.cpp:70-81`). The game fires it during frames
//!   (`tf_populators.cpp:1928`), but should another plugin or a script fire it
//!   between frames, the change in each overridden `m_iTotalScore` is computed
//!   against the override, and reported as above.
//!
//! The column holding each player's team (`m_iTeam`) cannot be overridden, as
//! the game reads it between frames when a vote is called, nor can the columns
//! clients identify players by.
//!
//! Clients' achievement logic reads the overridden columns as well: at the end
//! of a round, an achievement compares the teammates' `m_iTotalScore`
//! (`achievements_tf.cpp:832-860`), a Spy achievement checks the victim's
//! `m_iActiveDominations` (`achievements_tf_spy.cpp:379`), and a Medic
//! achievement checks that no teammate's `m_iPlayerClass` is the Medic
//! (`achievements_tf_medic.cpp:326-342`). Overrides can therefore grant or
//! withhold real Steam achievements.
//!
//! # Unverified
//!
//! On TF2's 64-bit Windows server, a retail client's scoreboard has been
//! observed to show [`Override::Fixed`] values of a player's score and kills,
//! and of BLU's score, steadily and without flicker. They held after a kill
//! raised the game's values, which [`Scoreboard::real_player_stat`] reported,
//! and clearing the overrides showed the game's values again. With bots, a
//! plugin calling [`Scoreboard::restore_all`] on pause and unload wrote the
//! game's values back, and the overrides returned on unpause.
//!
//! Linux GNU servers, the other columns, [`Override::Offset`], team flag
//! captures, the [game state](#game-state) functions, the achievement effects
//! above, Mann vs. Machine, SourceTV and demos, and clients under packet loss
//! or lag have not been observed live.
//!
//! # Game state
//!
//! [`set_team_score`], [`set_team_flag_captures`], [`set_frags`],
//! [`set_deaths`] and [`reset_scores`] change what the game itself keeps, which
//! every reader then uses, with the consequences each documents. While a
//! [`Scoreboard`] overrides a team's number, set it through
//! [`Scoreboard::set_real_team_stat`] instead.

#[cfg(test)]
#[path = "../../tests/tf2/scoreboard.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::datatables::{NetPropError, PropFlags, PropKind, SendProp, ServerClass, Storage};
use crate::edicts::Edict;
use crate::entities::{Entity, EntityHandle};
use crate::interfaces::{ServerGameDll, ServerTools, ValveEngine};
use crate::players::UserId;
use crate::tf2::PlayerClass;
use crate::{Game, InterfaceError, Server};
use sdk_raw::edicts::MAX_CHANGE_OFFSETS;

use sdk_raw::tf2::scoreboard::{
	ELEMENT_SIZE, KILL_STREAK, STREAKS_PER_SLOT, TF_TEAM_BLUE, TF_TEAM_RED,
};

use sdk_raw::vcall;
use std::collections::BTreeMap;
use std::ffi::{CStr, c_int};
use std::ops::RangeInclusive;

/// An exclusive bound on the datamap offsets trusted for a player's fields.
const MAX_FIELD_OFFSET: usize = 1 << 16;

/// The class name of TF2's `CTFPlayerResource` (`tf_player_resource.cpp:52`).
const RESOURCE_CLASS_NAME: &CStr = c"tf_player_manager";

/// The class name of TF2's `CTFTeam` (`tf_team.cpp:61`).
const TEAM_CLASS_NAME: &CStr = c"tf_team";

/// Where a player column's array lives in the player resource, as resolved
/// from the resource's server class.
#[derive(Debug, Clone, Copy)]
struct ArrayLayout {
	/// Bytes from the start of the entity to the first element.
	first: usize,

	/// The number of elements, each [`ELEMENT_SIZE`] bytes after the last.
	len: usize,

	/// The values an element can be networked with.
	range: NetRange,
}

impl ArrayLayout {
	/// Resolves `field`'s array in `class`, checking that it is a contiguous
	/// array of `int`s that holds at least player slot 1.
	fn resolve(
		dll: ServerGameDll<'_>,
		class: ServerClass<'_>,
		field: PlayerField,
	) -> Result<Self, ScoreboardError> {
		let needed = field.element(1).map_or(usize::MAX, |element| element + 1);
		let unexpected = || ScoreboardError::UnexpectedLayout {
			name: field.display_name(),
			needed,
		};

		let array = dll.net_prop(class, field.name())?;
		let len = array
			.element_count()
			.filter(|&len| len >= needed)
			.ok_or_else(unexpected)?;
		let first = array.element(0)?;

		for index in 0..len {
			let element = array.element(index)?;
			let expected = index
				.checked_mul(ELEMENT_SIZE)
				.and_then(|bytes| first.offset().checked_add(bytes));

			if element.prop().kind() != PropKind::Int
				|| !element.storage().is_compatible(Storage::I32)
				|| Some(element.offset()) != expected
			{
				return Err(unexpected());
			}
		}

		Ok(Self {
			first: first.offset(),
			len,
			range: encodable_range(first.prop()),
		})
	}

	/// Checks that the array holds [`STREAKS_PER_SLOT`] elements for each of
	/// `slots` player slots, as `m_iStreaks` must for the bindings' streak
	/// numbering to address the right element.
	fn grouped(self, slots: usize) -> Result<Self, ScoreboardError> {
		if slots.checked_mul(STREAKS_PER_SLOT) == Some(self.len) {
			Ok(self)
		} else {
			Err(ScoreboardError::UnexpectedStreaks {
				len: self.len,
				slots,
			})
		}
	}

	/// Bytes from the start of the entity to `field`'s element for `slot`, or
	/// `None` if the array does not reach it.
	fn offset(self, field: PlayerField, slot: usize) -> Option<usize> {
		let element = field.element(slot).filter(|&element| element < self.len)?;

		// `resolve` checked that every element's offset is this.
		Some(self.first + element * ELEMENT_SIZE)
	}
}

/// The variables of one entity marked changed during one pass, sent to the
/// engine's change tracking together.
#[derive(Debug)]
struct Changes {
	/// Whether to mark the whole entity changed, whatever `offsets` holds.
	full: bool,

	/// The offsets of the variables to mark changed, without duplicates.
	offsets: Vec<usize>,
}

impl Changes {
	/// No changes, or the whole entity if `full`.
	const fn new(full: bool) -> Self {
		Self {
			full,
			offsets: Vec::new(),
		}
	}

	/// Tells the engine about the changes to `edict`'s entity.
	///
	/// More offsets than the engine records per frame, or one it cannot
	/// record, mark the whole entity changed at once, as recording them one by
	/// one would end up doing.
	fn flush(self, engine: ValveEngine<'_>, edict: Edict<'_>) {
		let offsets: Option<Vec<u16>> = self
			.offsets
			.iter()
			.map(|&offset| u16::try_from(offset).ok())
			.collect();

		match offsets {
			Some(offsets) if !self.full && offsets.len() <= usize::from(MAX_CHANGE_OFFSETS) => {
				for offset in offsets {
					edict.state_changed(engine, offset);
				}
			}

			_ => edict.full_state_changed(engine),
		}
	}

	/// Marks the variable at `offset` changed.
	fn mark(&mut self, offset: usize) {
		if !self.offsets.contains(&offset) {
			self.offsets.push(offset);
		}
	}
}

/// The interfaces the scoreboard uses within one callback.
#[derive(Debug, Clone, Copy)]
struct Context<'s> {
	dll: ServerGameDll<'s>,
	engine: ValveEngine<'s>,
	tools: ServerTools<'s>,
}

impl<'s> Context<'s> {
	fn new(server: Server<'s>) -> Result<Self, InterfaceError> {
		Ok(Self {
			dll: server.server_game_dll()?,
			engine: server.valve_engine()?,
			tools: server.server_tools()?,
		})
	}

	/// The user ID of the client that owns player slot `slot` now.
	fn owner(self, slot: usize) -> Option<UserId> {
		let edict = self.engine.edict_of_index(c_int::try_from(slot).ok()?)?;

		self.engine.user_id_of_edict(edict)
	}
}

/// One overridable variable: what clients should see, and what the store
/// knows of the game's value and of what it wrote.
#[derive(Debug, Clone, Copy, Default)]
struct FieldState {
	/// What clients should see instead of the game's value.
	overridden: Option<Override>,

	/// The game's own value, as the last frame left it.
	real: Option<i32>,

	/// What the store last wrote in place of `real`, which clients receive
	/// unless something else wrote the variable since.
	applied: Option<i32>,

	/// Whether [`Self::before_frame`] wrote `real` back over `applied` for the
	/// current frame, so the variable holds the game's value after it.
	restored: bool,
}

impl FieldState {
	/// Records the game's value after its frame and, if `apply`, writes the
	/// override. Without an override to write, the game's value is written
	/// back where the store wrote another.
	///
	/// If [`Self::before_frame`] did not write the game's value back for this
	/// frame, the variable may still hold what was applied, which is then taken
	/// to mean the game did not change it.
	fn after_frame(
		&mut self,
		variable: IntField<'_>,
		range: NetRange,
		changes: &mut Changes,
		always_flag: bool,
		apply: bool,
	) {
		let restored = std::mem::take(&mut self.restored);

		if !self.is_active() {
			return;
		}

		let memory = variable.read();
		let real = match (self.applied, self.real) {
			(Some(applied), Some(real)) if !restored && memory == applied => real,
			_ => memory,
		};

		match self.overridden.filter(|_| apply) {
			Some(value) => {
				let shown = value.resolve(real, range);

				if memory != shown {
					variable.write(shown);
				}

				if self.applied != Some(shown) || always_flag {
					changes.mark(variable.offset);
				}

				self.real = Some(real);
				self.applied = Some(shown);
			}

			None => {
				if self.applied.take().is_some() {
					if memory != real {
						variable.write(real);
					}

					changes.mark(variable.offset);
				}

				if self.overridden.is_some() {
					self.real = Some(real);
				}
			}
		}
	}

	/// Writes the game's value back before its frame, where the variable
	/// still holds what was applied.
	///
	/// Returns whether the value was written back without marking it changed,
	/// which leaves clients with the override only if [`Self::after_frame`]
	/// applies it again before the engine sends the frame.
	fn before_frame(
		&mut self,
		variable: IntField<'_>,
		changes: &mut Changes,
		always_flag: bool,
	) -> bool {
		self.restored = false;

		let Some(applied) = self.applied else {
			return false;
		};

		let memory = variable.read();

		if memory == applied {
			if let Some(real) = self.real
				&& real != memory
			{
				variable.write(real);
			}

			if self.overridden.is_some() && !always_flag {
				self.restored = true;
				return true;
			}

			changes.mark(variable.offset);
		} else {
			// Something else wrote the variable since it was applied, which the
			// game's logic takes as its own value from now on.
			self.real = Some(memory);

			if self.overridden.is_none() || always_flag {
				changes.mark(variable.offset);
			}
		}

		self.applied = None;
		false
	}

	/// Drops what the store knows of an entity that is gone.
	const fn forget_memory(&mut self) {
		self.real = None;
		self.applied = None;
		self.restored = false;
	}

	/// Whether the variable is overridden, or still holds what was applied.
	const fn is_active(&self) -> bool {
		self.overridden.is_some() || self.applied.is_some()
	}

	/// Drops the game's value once nothing needs it.
	const fn prune(&mut self) {
		if !self.is_active() {
			self.real = None;
		}
	}

	/// Writes the game's value back where the variable holds what was
	/// applied, and marks it changed, keeping the override.
	fn restore(&mut self, variable: IntField<'_>, changes: &mut Changes) {
		self.restored = false;

		let Some(applied) = self.applied.take() else {
			return;
		};

		let memory = variable.read();

		if memory != applied {
			self.real = Some(memory);
		} else if let Some(real) = self.real
			&& real != memory
		{
			variable.write(real);
		}

		changes.mark(variable.offset);
	}
}

/// An `int` member of a live entity, at an offset resolved for the entity's
/// class.
#[derive(Debug, Clone, Copy)]
struct IntField<'s> {
	entity: Entity<'s>,
	offset: usize,
}

impl<'s> IntField<'s> {
	/// # Safety
	///
	/// `offset` must be where an `int` member lives in every entity of
	/// `entity`'s class, as a send table or datamap of that class gives it.
	const unsafe fn new(entity: Entity<'s>, offset: usize) -> Self {
		Self { entity, offset }
	}

	fn read(self) -> i32 {
		// SAFETY: `new`'s contract places an `int` at the offset, within the
		// entity, which stays allocated for `'s`. It is read without forming a
		// reference, since the game writes it through its own pointers.
		unsafe {
			self.entity
				.as_ptr()
				.byte_add(self.offset)
				.cast::<i32>()
				.read_unaligned()
		}
	}

	fn write(self, value: i32) {
		// SAFETY: As for `read`. The game writes its members the same way, on
		// the main thread.
		unsafe {
			self.entity
				.as_ptr()
				.byte_add(self.offset)
				.cast::<i32>()
				.write_unaligned(value);
		}
	}
}

/// What [`Scoreboard::level_shutdown`] does with the overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LevelPolicy {
	/// Keep the overrides for the next level. Each player's still applies to
	/// them while their user ID owns the same player slot, and is dropped at
	/// the first [`Scoreboard::after_frame`] where it does not, as while no
	/// client is in the slot.
	KeepOverrides,

	/// Drop every override.
	ClearOverrides,
}

/// The values a networked integer can hold, from [`encodable_range`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetRange {
	min: i32,
	max: i32,
}

impl NetRange {
	/// Checks that `value` can be networked as the variable `name`.
	fn check(self, name: &'static str, value: i32) -> Result<(), ScoreboardError> {
		if (self.min..=self.max).contains(&value) {
			Ok(())
		} else {
			Err(ScoreboardError::OutOfRange {
				name,
				value,
				min: self.min,
				max: self.max,
			})
		}
	}

	/// The value in the range nearest to `value`.
	fn clamp(self, value: i64) -> i32 {
		saturate(value.clamp(i64::from(self.min), i64::from(self.max)))
	}

	const fn inclusive(self) -> RangeInclusive<i32> {
		self.min..=self.max
	}
}

/// What clients see in place of the game's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Override {
	/// This value. Setting it fails with [`ScoreboardError::OutOfRange`] if the
	/// variable cannot network it.
	Fixed(i32),

	/// The game's value plus this, saturated to what the variable can
	/// network, and following the game's value as it changes.
	Offset(i32),
}

impl Override {
	/// The value shown for the game's `real` value.
	fn resolve(self, real: i32, range: NetRange) -> i32 {
		match self {
			Self::Fixed(value) => range.clamp(i64::from(value)),
			Self::Offset(offset) => range.clamp(i64::from(real) + i64::from(offset)),
		}
	}
}

/// How far the store has come through the current frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
	/// [`Scoreboard::before_frame`] has not run since the store was created,
	/// restored, or its level shut down, so [`Scoreboard::after_frame`] applies
	/// nothing: no override is applied that a game frame could see.
	#[default]
	Idle,

	/// [`Scoreboard::before_frame`] ran last. `unflagged` is whether it wrote
	/// values back without marking them changed, which only the following
	/// [`Scoreboard::after_frame`] makes right for clients.
	Restored { unflagged: bool },

	/// [`Scoreboard::after_frame`] ran last, after a
	/// [`Scoreboard::before_frame`].
	Applied,
}

/// A player column the store can override: a [`PlayerStat`], the class, or
/// whether the player is alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerField {
	Stat(PlayerStat),
	Class,
	Alive,
}

impl PlayerField {
	/// Every column, in the order of [`Self::index`].
	const ALL: [Self; 16] = [
		Self::Stat(PlayerStat::Score),
		Self::Stat(PlayerStat::Kills),
		Self::Stat(PlayerStat::Deaths),
		Self::Stat(PlayerStat::Ping),
		Self::Stat(PlayerStat::Dominations),
		Self::Stat(PlayerStat::Killstreak),
		Self::Stat(PlayerStat::Damage),
		Self::Stat(PlayerStat::BossDamage),
		Self::Stat(PlayerStat::Healing),
		Self::Stat(PlayerStat::DamageAssist),
		Self::Stat(PlayerStat::HealingAssist),
		Self::Stat(PlayerStat::DamageBlocked),
		Self::Stat(PlayerStat::BonusPoints),
		Self::Stat(PlayerStat::CurrencyCollected),
		Self::Class,
		Self::Alive,
	];

	/// The networked variable's name, as an `&str` for errors.
	fn display_name(self) -> &'static str {
		self.name().to_str().unwrap_or_default()
	}

	/// The index of `slot`'s element in the column's array.
	fn element(self, slot: usize) -> Option<usize> {
		match self {
			Self::Stat(PlayerStat::Killstreak) => {
				slot.checked_mul(STREAKS_PER_SLOT)?.checked_add(KILL_STREAK)
			}

			_ => Some(slot),
		}
	}

	/// The column's position in [`Self::ALL`] and in each slot's fields.
	const fn index(self) -> usize {
		match self {
			Self::Stat(stat) => stat as usize,
			Self::Class => PlayerStat::ALL.len(),
			Self::Alive => PlayerStat::ALL.len() + 1,
		}
	}

	/// The networked array's name in `DT_TFPlayerResource`.
	const fn name(self) -> &'static CStr {
		match self {
			Self::Stat(stat) => stat.name(),
			Self::Class => c"m_iPlayerClass",
			Self::Alive => c"m_bAlive",
		}
	}
}

/// What the store keeps for one player slot.
#[derive(Debug, Default)]
struct PlayerSlot {
	/// The client whose overrides these are, while they apply.
	owner: Option<UserId>,

	/// Each column, in [`PlayerField::ALL`]'s order.
	fields: [FieldState; PlayerField::ALL.len()],
}

impl PlayerSlot {
	/// Drops the owner and its overrides. What was applied is still written
	/// back by the next frame callback.
	fn disown(&mut self) {
		self.owner = None;

		for field in &mut self.fields {
			field.overridden = None;
		}
	}

	fn forget_memory(&mut self) {
		self.fields.iter_mut().for_each(FieldState::forget_memory);
	}

	/// Drops what is no longer needed, and returns whether anything is left.
	fn prune(&mut self) -> bool {
		self.fields.iter_mut().for_each(FieldState::prune);
		self.fields.iter().any(FieldState::is_active)
	}
}

/// A player column of numbers, from `CTFPlayerResource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerStat {
	/// The scoreboard's "Score" column: `m_iTotalScore`, the points the game
	/// works out from the player's statistics. Clients' achievement logic
	/// compares it between teammates, as the module documentation describes.
	#[doc(alias("m_iTotalScore"))]
	Score,

	/// Kills in the statistics panel of the player selected on the
	/// scoreboard: `m_iScore`, a copy of the player's frag count.
	#[doc(alias("m_iScore"))]
	Kills,

	/// Deaths in the statistics panel: `m_iDeaths`.
	#[doc(alias("m_iDeaths"))]
	Deaths,

	/// The ping column: `m_iPing`. Clients show "BOT" for bots whatever it
	/// holds.
	#[doc(alias("m_iPing"))]
	Ping,

	/// The number of players the player dominates, shown as an icon:
	/// `m_iActiveDominations`. A client's Spy achievement checks it for the
	/// victims of backstabs, as the module documentation describes.
	#[doc(alias("m_iActiveDominations"))]
	Dominations,

	/// The killstreak count: the `kTFStreak_Kills` element of the player's
	/// group in `m_iStreaks`. Clients show it only for players whose entity
	/// they have, holding a weapon.
	#[doc(alias("m_iStreaks", "kTFStreak_Kills"))]
	Killstreak,

	/// Damage dealt, in the statistics panel and Mann vs. Machine's
	/// scoreboard: `m_iDamage`.
	#[doc(alias("m_iDamage"))]
	Damage,

	/// Damage to bosses, Mann vs. Machine's "tank" column: `m_iDamageBoss`.
	#[doc(alias("m_iDamageBoss"))]
	BossDamage,

	/// Mann vs. Machine's healing column: `m_iHealing`.
	#[doc(alias("m_iHealing"))]
	Healing,

	/// Damage assisted, part of Mann vs. Machine's "support" column:
	/// `m_iDamageAssist`.
	#[doc(alias("m_iDamageAssist"))]
	DamageAssist,

	/// Healing assisted, part of the "support" column: `m_iHealingAssist`.
	#[doc(alias("m_iHealingAssist"))]
	HealingAssist,

	/// Damage blocked, part of the "support" column: `m_iDamageBlocked`.
	#[doc(alias("m_iDamageBlocked"))]
	DamageBlocked,

	/// Bonus points, each counted as 25 in the "support" column:
	/// `m_iBonusPoints`.
	#[doc(alias("m_iBonusPoints"))]
	BonusPoints,

	/// Mann vs. Machine's credits column: `m_iCurrencyCollected`.
	#[doc(alias("m_iCurrencyCollected"))]
	CurrencyCollected,
}

impl PlayerStat {
	/// Every column.
	///
	/// When a Mann vs. Machine wave completes, clients add the damage,
	/// healing, support and credit columns they were last sent to running
	/// totals (`tf_hud_mann_vs_machine_scoreboard.cpp:145-167`), so overrides
	/// shown then stay in those totals.
	pub const ALL: [Self; 14] = [
		Self::Score,
		Self::Kills,
		Self::Deaths,
		Self::Ping,
		Self::Dominations,
		Self::Killstreak,
		Self::Damage,
		Self::BossDamage,
		Self::Healing,
		Self::DamageAssist,
		Self::HealingAssist,
		Self::DamageBlocked,
		Self::BonusPoints,
		Self::CurrencyCollected,
	];

	/// The networked array's name in `DT_TFPlayerResource`, such as
	/// `m_iTotalScore`.
	pub const fn name(self) -> &'static CStr {
		match self {
			Self::Score => c"m_iTotalScore",
			Self::Kills => c"m_iScore",
			Self::Deaths => c"m_iDeaths",
			Self::Ping => c"m_iPing",
			Self::Dominations => c"m_iActiveDominations",
			Self::Killstreak => c"m_iStreaks",
			Self::Damage => c"m_iDamage",
			Self::BossDamage => c"m_iDamageBoss",
			Self::Healing => c"m_iHealing",
			Self::DamageAssist => c"m_iDamageAssist",
			Self::HealingAssist => c"m_iHealingAssist",
			Self::DamageBlocked => c"m_iDamageBlocked",
			Self::BonusPoints => c"m_iBonusPoints",
			Self::CurrencyCollected => c"m_iCurrencyCollected",
		}
	}
}

/// What the store keeps of the player resource it last found.
#[derive(Debug)]
struct ResourceCache {
	handle: EntityHandle,

	/// The address of the resource's server class, which `fields` describes.
	class: usize,

	/// Each column's array, in [`PlayerField::ALL`]'s order.
	fields: [Result<ArrayLayout, ScoreboardError>; PlayerField::ALL.len()],
}

impl ResourceCache {
	/// Resolves every column of the resource `entity`, whose server class is
	/// `class`.
	///
	/// `m_iStreaks` must also hold a group for each element of
	/// `m_iTotalScore`, so it is unusable whenever that is.
	fn resolve(dll: ServerGameDll<'_>, entity: Entity<'_>, class: ServerClass<'_>) -> Self {
		let mut fields = PlayerField::ALL.map(|field| ArrayLayout::resolve(dll, class, field));
		let slots = fields[PlayerField::Stat(PlayerStat::Score).index()]
			.as_ref()
			.map(|layout| layout.len)
			.map_err(Clone::clone);
		let streaks = &mut fields[PlayerField::Stat(PlayerStat::Killstreak).index()];

		if let Ok(&layout) = streaks.as_ref() {
			*streaks = slots.and_then(|slots| layout.grouped(slots));
		}

		Self {
			handle: entity.handle(),
			class: class.as_ptr().addr(),
			fields,
		}
	}

	/// The offset of `field`'s element for `slot`, and the values it holds.
	fn locate(&self, field: PlayerField, slot: usize) -> Option<(usize, NetRange)> {
		let layout = self.fields[field.index()].as_ref().ok()?;

		Some((layout.offset(field, slot)?, layout.range))
	}
}

/// Where a team's variable lives, as resolved from the team's server class.
#[derive(Debug, Clone, Copy)]
struct Scalar {
	offset: usize,
	range: NetRange,
}

impl Scalar {
	fn resolve(
		dll: ServerGameDll<'_>,
		class: ServerClass<'_>,
		stat: TeamStat,
	) -> Result<Self, ScoreboardError> {
		let variable = dll.net_prop(class, stat.name())?;

		Ok(Self {
			offset: variable.offset(),
			range: int_range(variable.prop(), variable.storage(), stat.display_name())?,
		})
	}
}

/// Scoreboard overrides, kept by the plugin between callbacks.
///
/// Set what clients see with the `set_player_*` methods and
/// [`Self::set_team_stat`], and read back what is overridden with the
/// `*_override` methods. Each override lasts until it is
/// cleared, or for a player until their client disconnects or their user ID
/// no longer owns the slot. The store holds no engine pointer: each call finds
/// the entities again by handle, and checks them against the server class
/// their layout was resolved from.
///
/// # Driving it
///
/// - Call [`Self::before_frame`] before every `IServerGameDLL::GameFrame`,
///   and [`Self::after_frame`] after it. Without the second, overrides never
///   reach clients; without the first, the game computes its score changes
///   against overrides. `after_frame` applies nothing until `before_frame`
///   has run since the store was created, restored, or its level shut down,
///   so the order in which the two hooks start does not matter: Metamod 2.0
///   can start either a few frames late. A frame whose `after_frame` is
///   missed is recovered by the next `before_frame`; a frame whose
///   `before_frame` is missed after that runs with the overrides in place.
/// - Call [`Self::restore_all`] when the plugin pauses or unloads, before its
///   hooks stop. Otherwise the overrides stay in the game's memory, and the
///   game reports the difference between them and the real score as Strange
///   "Points Scored" progress, or as Mann vs. Machine statistics. Dropping the
///   store does not restore anything.
/// - Call [`Self::forget_player`] when a player disconnects
///   ([`GameEventId::PlayerDisconnect`]), and [`Self::level_shutdown`] when
///   the level ends. Neither touches an entity.
///
/// None of these run game code: they read and write the variables, and use
/// the engine's change tracking, so they free no entity. Calling the frame
/// callbacks at other times is safe, but lets game logic see overrides.
///
/// [`GameEventId::PlayerDisconnect`]: crate::tf2::game_events::GameEventId::PlayerDisconnect
#[doc(alias("CTFPlayerResource", "tf_player_manager"))]
#[must_use = "overrides only reach clients while the store's frame callbacks run"]
#[derive(Debug, Default)]
pub struct Scoreboard {
	/// Whether every write is marked changed, as a fallback and for debugging.
	always_flag: bool,
	phase: Phase,

	/// Each player slot with overrides, or with values still to write back,
	/// by entity index.
	players: BTreeMap<usize, PlayerSlot>,
	resource: Option<ResourceCache>,
	team_caches: [Option<TeamCache>; ScoringTeam::ALL.len()],
	teams: [TeamSlot; ScoringTeam::ALL.len()],
	_not_thread_safe: NotThreadSafe,
}

impl Scoreboard {
	/// An empty store, which overrides nothing.
	pub fn new() -> Self {
		Self::default()
	}

	/// Records the game's values after its frame and writes the overrides,
	/// marking changed only what clients must receive anew. Call it after
	/// every `IServerGameDLL::GameFrame`.
	///
	/// Until [`Self::before_frame`] has run since the store was created,
	/// [`Self::restore_all`] or [`Self::level_shutdown`], this only records the
	/// game's values, and writes them back where an override was applied, so
	/// that no game frame runs with an override in place.
	///
	/// A player's overrides are dropped here once their user ID no longer owns
	/// their slot, as when another client takes the slot or no client is in it.
	/// Overrides kept by `level_shutdown` with [`LevelPolicy::KeepOverrides`]
	/// apply once the next level's player resource and teams exist.
	pub fn after_frame(&mut self, server: Server<'_>) {
		let always_flag = self.always_flag;
		let apply = self.phase != Phase::Idle;
		let phase = if apply { Phase::Applied } else { Phase::Idle };

		if !self.has_active() {
			self.phase = phase;
			return;
		}

		// Without the interfaces nothing is applied, so the phase stays as it
		// is, and the next `before_frame` recovers what this one would have.
		let Ok(context) = Context::new(server) else {
			return;
		};

		self.phase = phase;

		for (&slot, state) in &mut self.players {
			if let Some(owner) = state.owner
				&& context.owner(slot) != Some(owner)
			{
				state.disown();
			}
		}

		self.visit(
			context,
			false,
			FieldState::is_active,
			|field, variable, range, changes| {
				field.after_frame(variable, range, changes, always_flag, apply);
			},
		);

		self.prune();
	}

	/// Writes the game's own values back before its frame, where the
	/// variables still hold the overrides, so the frame's game logic sees
	/// them. Call it before every `IServerGameDLL::GameFrame`.
	///
	/// If [`Self::after_frame`] did not run after the previous call, the
	/// values written back then never reached clients consistently, so the
	/// entities are marked changed as a whole.
	pub fn before_frame(&mut self, server: Server<'_>) {
		let heal = self.phase == Phase::Restored { unflagged: true };
		let always_flag = self.always_flag;
		let mut unflagged = false;

		for field in self.fields_mut() {
			field.restored = false;
		}

		if self.has_applied()
			&& let Ok(context) = Context::new(server)
		{
			self.visit(
				context,
				heal,
				|field| field.applied.is_some(),
				|field, variable, _, changes| {
					unflagged |= field.before_frame(variable, changes, always_flag);
				},
			);
		}

		self.phase = Phase::Restored { unflagged };
		self.prune();
	}

	/// Drops every override. Values applied are written back by the next
	/// frame callback or [`Self::restore_all`].
	pub fn clear_all_overrides(&mut self) {
		for slot in self.players.values_mut() {
			slot.disown();
		}

		for team in &mut self.teams {
			for field in &mut team.fields {
				field.overridden = None;
			}
		}

		self.prune();
	}

	/// Drops the override of whether `player` is alive.
	pub fn clear_player_alive(&mut self, player: UserId) {
		self.clear_player_field(player, PlayerField::Alive);
	}

	/// Drops the override of `player`'s class.
	pub fn clear_player_class(&mut self, player: UserId) {
		self.clear_player_field(player, PlayerField::Class);
	}

	fn clear_player_field(&mut self, player: UserId, field: PlayerField) {
		if let Some(slot) = self.slot_of(player)
			&& let Some(state) = self.players.get_mut(&slot)
		{
			state.fields[field.index()].overridden = None;
		}

		self.prune();
	}

	/// Drops the override of one of `player`'s columns. The game's value is
	/// written back by the next frame callback.
	pub fn clear_player_stat(&mut self, player: UserId, stat: PlayerStat) {
		self.clear_player_field(player, PlayerField::Stat(stat));
	}

	/// Drops the override of one of a team's numbers.
	pub fn clear_team_stat(&mut self, team: ScoringTeam, stat: TeamStat) {
		self.teams[team.index()].fields[stat.index()].overridden = None;
		self.prune();
	}

	/// Every variable the store keeps.
	fn fields(&self) -> impl Iterator<Item = &FieldState> {
		self.players
			.values()
			.flat_map(|slot| &slot.fields)
			.chain(self.teams.iter().flat_map(|team| &team.fields))
	}

	/// Every variable the store keeps, to change.
	fn fields_mut(&mut self) -> impl Iterator<Item = &mut FieldState> {
		self.players
			.values_mut()
			.flat_map(|slot| &mut slot.fields)
			.chain(self.teams.iter_mut().flat_map(|team| &mut team.fields))
	}

	/// Drops every override of `player`, as when their client disconnects.
	///
	/// Call it for `player_disconnect`, so that the slot's next client does not
	/// start with them; [`Self::after_frame`] also drops them once the user ID
	/// no longer owns the slot. This touches no entity: what was applied is
	/// written back by the next frame callback or [`Self::restore_all`].
	pub fn forget_player(&mut self, player: UserId) {
		if let Some(slot) = self.slot_of(player)
			&& let Some(state) = self.players.get_mut(&slot)
		{
			state.disown();
		}

		self.prune();
	}

	/// Whether anything is overridden, or still holds what was applied.
	fn has_active(&self) -> bool {
		self.fields().any(FieldState::is_active)
	}

	/// Whether any variable still holds what was applied.
	fn has_applied(&self) -> bool {
		self.fields().any(|field| field.applied.is_some())
	}

	/// Forgets the level's entities and resolved layouts, and what was applied
	/// to them, without touching them, since the engine frees them with the
	/// level. Call it when the level shuts down.
	///
	/// Overrides are kept or dropped by `policy`.
	pub fn level_shutdown(&mut self, policy: LevelPolicy) {
		self.phase = Phase::Idle;
		self.resource = None;
		self.team_caches = Default::default();
		self.fields_mut().for_each(FieldState::forget_memory);

		if policy == LevelPolicy::ClearOverrides {
			self.clear_all_overrides();
		}

		self.prune();
	}

	/// The user IDs of the players with an override, each once.
	///
	/// The store drops overrides without being asked when a player's user ID
	/// no longer owns their slot, when the running game no longer lays out a
	/// column as before, and at [`Self::level_shutdown`] by its policy; this
	/// and the other `*_override` methods tell what is left.
	pub fn overridden_players(&self) -> impl Iterator<Item = UserId> + '_ {
		self.players
			.values()
			.filter(|slot| slot.fields.iter().any(|field| field.overridden.is_some()))
			.filter_map(|slot| slot.owner)
	}

	/// Whether `player` is shown as alive, if the store overrides it.
	pub fn player_alive_override(&self, player: UserId) -> Option<bool> {
		match self.player_field_override(player, PlayerField::Alive)? {
			Override::Fixed(alive) => Some(alive != 0),
			Override::Offset(_) => None,
		}
	}

	/// The class `player` is shown as, if the store overrides it.
	pub fn player_class_override(&self, player: UserId) -> Option<PlayerClass> {
		match self.player_field_override(player, PlayerField::Class)? {
			Override::Fixed(class) => PlayerClass::from_raw(class),
			Override::Offset(_) => None,
		}
	}

	fn player_field_override(&self, player: UserId, field: PlayerField) -> Option<Override> {
		let slot = self.players.get(&self.slot_of(player)?)?;

		slot.fields[field.index()].overridden
	}

	/// The override of one of `player`'s columns, if any.
	pub fn player_override(&self, player: UserId, stat: PlayerStat) -> Option<Override> {
		self.player_field_override(player, PlayerField::Stat(stat))
	}

	/// The values the running game can network for a player column, from
	/// its networked variable's bit count and flags.
	///
	/// [`Override::Fixed`] values must lie within it, and [`Override::Offset`]
	/// results are saturated to it.
	pub fn player_stat_range(
		&mut self,
		server: Server<'_>,
		stat: PlayerStat,
	) -> Result<RangeInclusive<i32>, ScoreboardError> {
		check_game(server)?;

		let context = Context::new(server)?;
		let (_, _, cache) = resource_entity(&mut self.resource, &mut self.players, context)?;
		let layout = cache.fields[PlayerField::Stat(stat).index()].clone()?;

		Ok(layout.range.inclusive())
	}

	/// Drops what is no longer needed.
	fn prune(&mut self) {
		self.players.retain(|_, slot| slot.prune());

		for team in &mut self.teams {
			team.fields.iter_mut().for_each(FieldState::prune);
		}
	}

	/// The game's own value of one of `player`'s columns while it is
	/// overridden, as [`Self::after_frame`] last found it.
	///
	/// Returns `None` if the column is not overridden, or has not been through
	/// `after_frame` since it was.
	pub fn real_player_stat(&self, player: UserId, stat: PlayerStat) -> Option<i32> {
		let slot = self.players.get(&self.slot_of(player)?)?;

		slot.fields[PlayerField::Stat(stat).index()].real
	}

	/// The game's own value of one of a team's numbers while it is
	/// overridden, as [`Self::after_frame`] last found it.
	pub fn real_team_stat(&self, team: ScoringTeam, stat: TeamStat) -> Option<i32> {
		self.teams[team.index()].fields[stat.index()].real
	}

	/// Writes every overridden variable back to the game's own value, and
	/// marks it changed, keeping the overrides: [`Self::after_frame`] applies
	/// them again once [`Self::before_frame`] has run.
	///
	/// Call it when the plugin pauses or unloads, while its hooks still run, so
	/// the game never computes its score changes against an override, and
	/// before causing the game to add to a team's number between frames.
	pub fn restore_all(&mut self, server: Server<'_>) {
		self.phase = Phase::Idle;

		if self.has_applied()
			&& let Ok(context) = Context::new(server)
		{
			self.visit(
				context,
				false,
				|field| field.applied.is_some(),
				|field, variable, _, changes| field.restore(variable, changes),
			);
		}

		self.prune();
	}

	/// Marks every value the store writes changed, not only those clients
	/// must receive anew.
	///
	/// This is for diagnosing the engine's change tracking, and as a fallback
	/// should values the store writes back ever reach clients. It makes the
	/// engine compare the whole player resource most frames.
	pub const fn set_always_flag(&mut self, always: bool) {
		self.always_flag = always;
	}

	/// Shows `player` as alive or dead, from the next [`Self::after_frame`]
	/// until cleared or the player disconnects.
	///
	/// Clients show a feigning Spy, or a Halloween ghost, as dead whatever this
	/// holds.
	///
	/// Fails as [`Self::set_player_stat`] does.
	#[doc(alias("m_bAlive"))]
	pub fn set_player_alive(
		&mut self,
		server: Server<'_>,
		player: UserId,
		alive: bool,
	) -> Result<(), ScoreboardError> {
		self.set_player_field(
			server,
			player,
			PlayerField::Alive,
			Override::Fixed(i32::from(alive)),
		)
	}

	/// Shows `class` as `player`'s class, from the next [`Self::after_frame`]
	/// until cleared or the player disconnects. Clients only show the classes
	/// of their teammates, and a client's Medic achievement checks them, as
	/// the module documentation describes.
	///
	/// Fails as [`Self::set_player_stat`] does.
	#[doc(alias("m_iPlayerClass"))]
	pub fn set_player_class(
		&mut self,
		server: Server<'_>,
		player: UserId,
		class: PlayerClass,
	) -> Result<(), ScoreboardError> {
		self.set_player_field(
			server,
			player,
			PlayerField::Class,
			Override::Fixed(class.to_raw()),
		)
	}

	fn set_player_field(
		&mut self,
		server: Server<'_>,
		owner: UserId,
		field: PlayerField,
		value: Override,
	) -> Result<(), ScoreboardError> {
		check_game(server)?;

		let context = Context::new(server)?;

		// `edict_of_user_id` only looks through the player slots.
		let slot = context
			.engine
			.edict_of_user_id(owner)
			.and_then(|edict| usize::try_from(edict.index()).ok())
			.ok_or(ScoreboardError::NotConnected)?;

		let (_, _, cache) = resource_entity(&mut self.resource, &mut self.players, context)?;
		let layout = cache.fields[field.index()].clone()?;

		if layout.offset(field, slot).is_none() {
			return Err(ScoreboardError::UnexpectedLayout {
				name: field.display_name(),
				needed: field
					.element(slot)
					.map_or(usize::MAX, |element| element + 1),
			});
		}

		if let Override::Fixed(value) = value {
			layout.range.check(field.display_name(), value)?;
		}

		for (&other, state) in &mut self.players {
			if other != slot && state.owner == Some(owner) {
				state.disown();
			}
		}

		let state = self.players.entry(slot).or_default();

		if state.owner != Some(owner) {
			state.disown();
			state.owner = Some(owner);
		}

		state.fields[field.index()].overridden = Some(value);
		Ok(())
	}

	/// Overrides one of `player`'s columns for their current connection, from
	/// the next [`Self::after_frame`] until cleared or the player disconnects.
	///
	/// Fails if the server does not run TF2, an interface is unavailable, no
	/// connected client has the user ID `player`, no player resource exists,
	/// the running game lays out the column differently, or a
	/// [`Override::Fixed`] value is outside [`Self::player_stat_range`].
	pub fn set_player_stat(
		&mut self,
		server: Server<'_>,
		player: UserId,
		stat: PlayerStat,
		value: Override,
	) -> Result<(), ScoreboardError> {
		self.set_player_field(server, player, PlayerField::Stat(stat), value)
	}

	/// Sets one of a team's numbers in the game's own state, with the
	/// consequences [`set_team_score`] and [`set_team_flag_captures`] document,
	/// marks it changed for clients, and records it as the game's value. An
	/// override of the number stays.
	///
	/// Use this instead of those functions while the store overrides the
	/// number. The store tells the game's writes from its own by value, so it
	/// takes a value they write that equals the override applied for its own,
	/// and writes the previous value back at the next [`Self::before_frame`].
	///
	/// Fails if the server does not run TF2, an interface is unavailable, the
	/// team has no entity, the running game does not network the number as an
	/// `int`, or `value` is outside [`Self::team_stat_range`].
	#[doc(alias("SetScore", "SetFlagCaptures"))]
	pub fn set_real_team_stat(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
		value: i32,
	) -> Result<(), ScoreboardError> {
		check_game(server)?;

		let context = Context::new(server)?;
		let index = team.index();
		let (entity, edict, cache) = team_entity(
			&mut self.team_caches[index],
			&mut self.teams[index],
			context,
			team,
		)?;
		let scalar = cache.fields[stat.index()].clone()?;

		scalar.range.check(stat.display_name(), value)?;

		// SAFETY: The offset was resolved from the send table of the class
		// `team_entity` checked the entity still has. The game's own setters
		// assign the score and flag captures any `int` they are given, and the
		// value fits what the variable networks.
		let variable = unsafe { IntField::new(entity, scalar.offset) };
		let mut changes = Changes::new(false);

		variable.write(value);
		changes.mark(scalar.offset);
		changes.flush(context.engine, edict);

		let field = &mut self.teams[index].fields[stat.index()];

		field.real = Some(value);
		field.applied = None;
		field.restored = false;
		self.prune();
		Ok(())
	}

	/// Overrides one of a team's numbers, from the next [`Self::after_frame`]
	/// until cleared.
	///
	/// The game keeps ending rounds and the map by its own values, which
	/// [`set_team_score`] and [`set_team_flag_captures`] change, except that
	/// an increment the game makes between frames is added to the value shown,
	/// and the sum becomes the game's own, as the module documentation
	/// describes. Call [`Self::restore_all`] before causing one.
	///
	/// Fails if the server does not run TF2, an interface is unavailable, the
	/// team has no entity, the running game lays out the variable differently,
	/// or a [`Override::Fixed`] value is outside [`Self::team_stat_range`].
	pub fn set_team_stat(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
		value: Override,
	) -> Result<(), ScoreboardError> {
		check_game(server)?;

		let context = Context::new(server)?;
		let index = team.index();
		let (_, _, cache) = team_entity(
			&mut self.team_caches[index],
			&mut self.teams[index],
			context,
			team,
		)?;
		let scalar = cache.fields[stat.index()].clone()?;

		if let Override::Fixed(value) = value {
			scalar.range.check(stat.display_name(), value)?;
		}

		self.teams[index].fields[stat.index()].overridden = Some(value);
		Ok(())
	}

	/// The player slot whose overrides belong to `player`.
	fn slot_of(&self, player: UserId) -> Option<usize> {
		self.players
			.iter()
			.find(|(_, slot)| slot.owner == Some(player))
			.map(|(&slot, _)| slot)
	}

	/// The override of one of a team's numbers, if any.
	pub fn team_override(&self, team: ScoringTeam, stat: TeamStat) -> Option<Override> {
		self.teams[team.index()].fields[stat.index()].overridden
	}

	/// The values the running game can network for one of a team's numbers,
	/// as [`Self::player_stat_range`] gives them for player columns.
	pub fn team_stat_range(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
	) -> Result<RangeInclusive<i32>, ScoreboardError> {
		check_game(server)?;

		let context = Context::new(server)?;
		let index = team.index();
		let (_, _, cache) = team_entity(
			&mut self.team_caches[index],
			&mut self.teams[index],
			context,
			team,
		)?;
		let scalar = cache.fields[stat.index()].clone()?;

		Ok(scalar.range.inclusive())
	}

	/// Runs `step` on each variable `wanted` selects, in its live entity, then
	/// sends the changes `step` marks to the engine, or marks each entity
	/// visited changed as a whole if `full`.
	///
	/// A variable the running game does not lay out as resolved before is
	/// dropped, override included.
	fn visit(
		&mut self,
		context: Context<'_>,
		full: bool,
		wanted: fn(&FieldState) -> bool,
		mut step: impl FnMut(&mut FieldState, IntField<'_>, NetRange, &mut Changes),
	) {
		let Self {
			players,
			resource,
			team_caches,
			teams,
			..
		} = self;

		if players.values().any(|slot| slot.fields.iter().any(wanted))
			&& let Ok((entity, edict, cache)) = resource_entity(resource, players, context)
		{
			let mut changes = Changes::new(full);

			for (&slot, state) in players.iter_mut() {
				for field in PlayerField::ALL {
					let entry = &mut state.fields[field.index()];

					if !wanted(entry) {
						continue;
					}

					match cache.locate(field, slot) {
						Some((offset, range)) => {
							// SAFETY: The offset was resolved from the send table of the
							// class `resource_entity` checked the entity still has.
							let variable = unsafe { IntField::new(entity, offset) };

							step(entry, variable, range, &mut changes);
						}

						None => *entry = FieldState::default(),
					}
				}
			}

			changes.flush(context.engine, edict);
		}

		for team in ScoringTeam::ALL {
			let index = team.index();
			let state = &mut teams[index];

			if !state.fields.iter().any(wanted) {
				continue;
			}

			let Ok((entity, edict, cache)) =
				team_entity(&mut team_caches[index], state, context, team)
			else {
				continue;
			};

			let mut changes = Changes::new(full);

			for stat in TeamStat::ALL {
				let entry = &mut state.fields[stat.index()];

				if !wanted(entry) {
					continue;
				}

				match cache.fields[stat.index()].as_ref() {
					Ok(scalar) => {
						// SAFETY: As for the player resource, from the team's class.
						let variable = unsafe { IntField::new(entity, scalar.offset) };

						step(entry, variable, scalar.range, &mut changes);
					}

					Err(_) => *entry = FieldState::default(),
				}
			}

			changes.flush(context.engine, edict);
		}
	}
}

/// Why a scoreboard operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScoreboardError {
	/// A required engine or game interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player's datamaps lack a field, or give it an implausible offset.
	#[error("the `{class}` datamap has no usable `{name}` field")]
	MissingField {
		/// The class whose datamap declares the field.
		class: &'static str,

		/// The field's name.
		name: &'static str,
	},

	/// A networked variable could not be found or accessed.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// No player resource exists, as while no level runs.
	#[error("no `tf_player_manager` entity exists; no level is running")]
	NoPlayerResource,

	/// No team entity has the team's number, as while no level runs.
	#[error("no `tf_team` entity has team number {}", .0.to_raw())]
	NoTeam(ScoringTeam),

	/// A team's variable is not a networked `int`.
	#[error("`{name}` is not a networked 32-bit integer")]
	NotAnInteger {
		/// The variable's name.
		name: &'static str,
	},

	/// No connected client has the user ID.
	#[error("no connected client has the user ID")]
	NotConnected,

	/// The server does not run TF2.
	#[error("the scoreboard requires a TF2 server")]
	NotTf2,

	/// The entity is not a TF2 player.
	#[error("the entity is not a TF2 player")]
	NotTfPlayer,

	/// The value cannot be networked as the variable.
	#[error("{value} cannot be networked as `{name}`, which holds {min}..={max}")]
	OutOfRange {
		/// The variable's name.
		name: &'static str,

		/// The value given.
		value: i32,

		/// The lowest value the variable can network.
		min: i32,

		/// The highest value the variable can network.
		max: i32,
	},

	/// A player column is not laid out as an array of `int`s with an element
	/// for the player's slot.
	#[error(
		"`{name}` is not a contiguous array of 32-bit integers with at least {needed} elements"
	)]
	UnexpectedLayout {
		/// The array's name.
		name: &'static str,

		/// The number of elements needed.
		needed: usize,
	},

	/// `m_iStreaks` does not hold `kTFStreak_COUNT` streaks, as the bindings
	/// number them, for each player slot of `m_iTotalScore`, so its elements
	/// cannot be told apart.
	#[error(
		"`m_iStreaks` has {len} elements, not {per_slot} for each of {slots} player slots",
		per_slot = STREAKS_PER_SLOT
	)]
	UnexpectedStreaks {
		/// The number of elements `m_iStreaks` has.
		len: usize,

		/// The number of player slots, as `m_iTotalScore` has elements.
		slots: usize,
	},
}

/// A team with a score: `TF_TEAM_RED` or `TF_TEAM_BLUE`, numbered as TF2 does
/// (`tf_shareddefs.h:31-35`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ScoringTeam {
	/// RED.
	#[doc(alias("TF_TEAM_RED"))]
	Red = TF_TEAM_RED,

	/// BLU.
	#[doc(alias("TF_TEAM_BLUE"))]
	Blue = TF_TEAM_BLUE,
}

impl ScoringTeam {
	/// Both teams.
	pub const ALL: [Self; 2] = [Self::Red, Self::Blue];

	/// The team with this team number, or `None` for any other number.
	pub const fn from_raw(raw: i32) -> Option<Self> {
		match raw {
			TF_TEAM_RED => Some(Self::Red),
			TF_TEAM_BLUE => Some(Self::Blue),
			_ => None,
		}
	}

	/// The team's position in [`Self::ALL`].
	const fn index(self) -> usize {
		match self {
			Self::Red => 0,
			Self::Blue => 1,
		}
	}

	/// The team's number, as `m_iTeamNum` holds it.
	pub const fn to_raw(self) -> i32 {
		self as i32
	}
}

/// What the store keeps of a team entity it last found.
#[derive(Debug)]
struct TeamCache {
	handle: EntityHandle,

	/// The address of the team's server class, which `fields` describes.
	class: usize,

	/// Each number's variable, in [`TeamStat::ALL`]'s order.
	fields: [Result<Scalar, ScoreboardError>; TeamStat::ALL.len()],
}

impl TeamCache {
	fn resolve(dll: ServerGameDll<'_>, entity: Entity<'_>, class: ServerClass<'_>) -> Self {
		Self {
			handle: entity.handle(),
			class: class.as_ptr().addr(),
			fields: TeamStat::ALL.map(|stat| Scalar::resolve(dll, class, stat)),
		}
	}
}

/// What the store keeps for one team.
#[derive(Debug, Default)]
struct TeamSlot {
	/// Each number, in [`TeamStat::ALL`]'s order.
	fields: [FieldState; TeamStat::ALL.len()],
}

/// A team's number on the scoreboard and HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamStat {
	/// The team's score, `CTeam::m_iScore`: rounds won in most game modes.
	#[doc(alias("m_iScore"))]
	Score,

	/// The flags the team captured this round, `CTFTeam::m_nFlagCaptures`,
	/// which the HUD shows instead of the score while
	/// `tf_flag_caps_per_round` is above 0 (`tf_hud_flagstatus.cpp:643-677`).
	/// PASS Time keeps its goals here, and its HUD shows them as the team's
	/// score (`tf_hud_passtime.cpp:200-213`).
	#[doc(alias("m_nFlagCaptures"))]
	FlagCaptures,
}

impl TeamStat {
	/// Both numbers.
	pub const ALL: [Self; 2] = [Self::Score, Self::FlagCaptures];

	/// The networked variable's name, as an `&str` for errors.
	fn display_name(self) -> &'static str {
		self.name().to_str().unwrap_or_default()
	}

	/// The number's position in [`Self::ALL`].
	const fn index(self) -> usize {
		match self {
			Self::Score => 0,
			Self::FlagCaptures => 1,
		}
	}

	/// The networked variable's name in `DT_TFTeam`, such as
	/// `m_nFlagCaptures`.
	pub const fn name(self) -> &'static CStr {
		match self {
			Self::Score => c"m_iScore",
			Self::FlagCaptures => c"m_nFlagCaptures",
		}
	}
}

/// Fails with [`ScoreboardError::NotTf2`] unless the server runs TF2.
fn check_game(server: Server<'_>) -> Result<(), ScoreboardError> {
	if server.game() == Game::TeamFortress2 {
		Ok(())
	} else {
		Err(ScoreboardError::NotTf2)
	}
}

/// The values a networked integer can hold, from its property's live bit
/// count and flags.
///
/// `SendPropInt` replaces a bit count of 0 or less with the variable's width
/// (`dt_send.cpp:519-524`), which is 32 for these variables, and so does this.
/// Integers reuse `SPROP_NORMAL` as `SPROP_VARINT`, which encodes all 32
/// bits. Unsigned ranges stop at `i32::MAX`, since the variables are `int`s.
fn encodable_range(prop: SendProp<'_>) -> NetRange {
	let flags = prop.flags();
	let bits = match prop.bits() {
		_ if flags.contains(PropFlags::NORMAL) => 32,
		bits @ 1..=32 => bits,
		_ => 32,
	};

	let (min, max) = if flags.contains(PropFlags::UNSIGNED) {
		(0, (1_i64 << bits) - 1)
	} else {
		(-(1_i64 << (bits - 1)), (1_i64 << (bits - 1)) - 1)
	};

	NetRange {
		min: saturate(min),
		max: saturate(max),
	}
}

/// Finds the `tf_team` entity of `team`.
fn find_team<'s>(context: Context<'s>, team: ScoringTeam) -> Result<Entity<'s>, ScoreboardError> {
	let mut after = None;

	for _ in 0..EntityHandle::SLOTS {
		let Some(entity) = context.tools.find_by_class_name(after, TEAM_CLASS_NAME) else {
			break;
		};

		let number = context
			.dll
			.entity_net_prop(entity, c"m_iTeamNum")?
			.get::<i32>(entity)?;

		if number == team.to_raw() {
			return Ok(entity);
		}

		after = Some(entity);
	}

	Err(ScoreboardError::NoTeam(team))
}

/// The range of an `int` variable named `name`, or an error if it is not one.
fn int_range(
	prop: SendProp<'_>,
	storage: Storage,
	name: &'static str,
) -> Result<NetRange, ScoreboardError> {
	if prop.kind() == PropKind::Int && storage.is_compatible(Storage::I32) {
		Ok(encodable_range(prop))
	} else {
		Err(ScoreboardError::NotAnInteger { name })
	}
}

/// The entity a cache describes, if `handle` still finds it and it still has
/// the server class at `class`, with its edict.
fn live<'s>(
	tools: ServerTools<'s>,
	handle: EntityHandle,
	class: usize,
) -> Option<(Entity<'s>, Edict<'s>)> {
	let entity = tools.entity_by_handle(handle)?;
	let (found, edict) = networking(entity).ok()?;

	(found.as_ptr().addr() == class).then_some((entity, edict))
}

/// An entity's server class and edict, which every networked entity has.
fn networking(entity: Entity<'_>) -> Result<(ServerClass<'_>, Edict<'_>), ScoreboardError> {
	match (entity.server_class(), entity.edict()) {
		(Some(class), Some(edict)) => Ok((class, edict)),

		_ => Err(NetPropError::NotNetworked {
			class_name: entity.class_name().to_string_lossy().into_owned(),
		}
		.into()),
	}
}

/// Resets `player`'s scoring as a team scramble does, through
/// `CTFPlayer::ResetScores`.
///
/// This is the game's state, not only what clients see. It resets the
/// statistics the game keeps for the player, which the "Score" column is
/// computed from, their frags and deaths, the statistics panel's counts, and
/// every domination and revenge relationship they are part of, and drops their
/// Mann vs. Machine events (`tf_player.cpp:3286-3294`).
///
/// # Re-entrancy
///
/// Other code runs before this returns. Removing the relationships fires the
/// `remove_nemesis_relationships` game event (`tf_player.cpp:4005-4010`), so
/// every listener runs during the call: the game's, other plugins', map
/// scripts', and this plugin's own. For a human player, dropping the Mann vs.
/// Machine events sends the `MVMResetPlayerStats` user message
/// (`tf_mann_vs_machine_stats.cpp:361-373`), which plugins hooking user
/// messages see. Release every borrow of plugin state those may need, such as
/// a `RefCell` or thread-local holding a [`Scoreboard`], before calling this.
///
/// Fails if the server does not run TF2, or `player`'s datamaps do not include
/// `CTFPlayer`'s.
#[doc(alias("ResetScores"))]
pub fn reset_scores(server: Server<'_>, player: Entity<'_>) -> Result<(), ScoreboardError> {
	check_game(server)?;

	if !player.has_data_map_class(c"CTFPlayer") {
		return Err(ScoreboardError::NotTfPlayer);
	}

	let player = player.as_ptr().cast::<sys::CTFPlayer>();

	// SAFETY: The datamaps show a `CTFPlayer`, whose entity base, and so its
	// primary vtable, `sdk_raw::tf2` asserts is at offset zero, and the
	// generated field gives `ResetScores`' slot in it under both ABIs. The
	// game's own code resets statistics and relationships, and frees no entity.
	// It also fires a game event to every listener and, for a human player,
	// sends a user message, both before returning; `Server::new`'s contract
	// requires the code those reach to free entities only through deferred
	// deletion, so every entity of the scope stays allocated.
	unsafe {
		vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_ResetScores());
	}

	Ok(())
}

/// Finds the player resource again through `cache`, or anew if the cached
/// entity is gone or has another class. What `players` remembers of the old
/// entity is then dropped, without writing to the new one.
fn resource_entity<'c, 's>(
	cache: &'c mut Option<ResourceCache>,
	players: &mut BTreeMap<usize, PlayerSlot>,
	context: Context<'s>,
) -> Result<(Entity<'s>, Edict<'s>, &'c ResourceCache), ScoreboardError> {
	let found = cache
		.as_ref()
		.and_then(|cached| live(context.tools, cached.handle, cached.class));

	let (entity, edict) = match found {
		Some(found) => found,

		None => {
			if cache.take().is_some() {
				players.values_mut().for_each(PlayerSlot::forget_memory);
			}

			let entity = context
				.tools
				.find_by_class_name(None, RESOURCE_CLASS_NAME)
				.ok_or(ScoreboardError::NoPlayerResource)?;
			let (class, edict) = networking(entity)?;

			*cache = Some(ResourceCache::resolve(context.dll, entity, class));
			(entity, edict)
		}
	};

	let cached = cache.as_ref().ok_or(ScoreboardError::NoPlayerResource)?;

	Ok((entity, edict, cached))
}

/// Converts to `i32`, saturating.
fn saturate(value: i64) -> i32 {
	i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// Sets the death count behind the statistics panel's deaths,
/// `CBasePlayer::m_iDeaths`.
///
/// This is the game's state, which it keeps counting from, as
/// [`set_frags`] describes for frags. Clients show only the deaths
/// [`PlayerStat::Deaths`] can network, -2048 to 2047 in the SDK, and see others
/// wrapped into that range.
///
/// Fails as [`set_frags`] does.
#[doc(alias("m_iDeaths"))]
pub fn set_deaths(
	server: Server<'_>,
	player: Entity<'_>,
	deaths: i32,
) -> Result<(), ScoreboardError> {
	set_player_count(server, player, c"m_iDeaths", deaths)
}

/// Sets the frag count behind the statistics panel's kills,
/// `CBasePlayer::m_iFrags`.
///
/// This is the game's state, which it keeps counting from, not only what
/// clients see: the player resource copies it into [`PlayerStat::Kills`] at
/// its next update. The copy the game keeps for `IPlayerInfo` (`pl.frags`)
/// only catches up at the player's next frag. The "Score" column is computed
/// from other statistics, and does not change.
///
/// Clients show only the kills [`PlayerStat::Kills`] can network, -2048 to
/// 2047 in the SDK ([`Scoreboard::player_stat_range`] gives the running
/// game's), and see others wrapped into that range.
///
/// Fails if the server does not run TF2, `player`'s datamaps do not include
/// `CTFPlayer`'s, or `CBasePlayer`'s does not declare the count as an `int`
/// at a plausible offset.
#[doc(alias("m_iFrags"))]
pub fn set_frags(
	server: Server<'_>,
	player: Entity<'_>,
	frags: i32,
) -> Result<(), ScoreboardError> {
	set_player_count(server, player, c"m_iFrags", frags)
}

/// Writes the `int` field `name` that `CBasePlayer`'s datamap declares.
fn set_player_count(
	server: Server<'_>,
	player: Entity<'_>,
	name: &'static CStr,
	value: i32,
) -> Result<(), ScoreboardError> {
	check_game(server)?;

	if !player.has_data_map_class(c"CTFPlayer") {
		return Err(ScoreboardError::NotTfPlayer);
	}

	let offset = player
		.data_maps()
		.find(|&map| map.class_name() == Some(c"CBasePlayer"))
		.and_then(|map| map.field_offset(name, sys::_fieldtypes_FIELD_INTEGER))
		.filter(|&offset| offset < MAX_FIELD_OFFSET && offset.is_multiple_of(ELEMENT_SIZE))
		.ok_or_else(|| ScoreboardError::MissingField {
			class: "CBasePlayer",
			name: name.to_str().unwrap_or_default(),
		})?;

	// SAFETY: `CBasePlayer`'s own datamap, which the player's chain includes,
	// declares an `int` at the offset. The game assigns the field the same way,
	// without notifying anything.
	unsafe { IntField::new(player, offset) }.write(value);

	Ok(())
}

/// Sets how many flags a team captured this round, as
/// `CTFTeam::SetFlagCaptures` does, and marks it changed for clients.
///
/// # Game consequences
///
/// This is the game's state, not only what clients see:
///
/// - In capture the flag, the game wins the round for a team whose captures
///   reach `tf_flag_caps_per_round` at its next check
///   (`tf_gamerules.cpp:9383-9419`).
/// - In PASS Time, the number is the team's score. The game wins the round for
///   a team whose score reaches `tf_passtime_scores_per_round` at its next
///   check (`tf_gamerules.cpp:9350-9378`) or when the ball next respawns
///   (`tf_passtime_logic.cpp:937-941`), and works out goals' points and the
///   winner when time runs out from it (`tf_passtime_logic.cpp:1345`,
///   `1706-1707`, `1785-1812`).
///
/// While a [`Scoreboard`] overrides [`TeamStat::FlagCaptures`], set it through
/// [`Scoreboard::set_real_team_stat`] instead, which this otherwise matches.
///
/// Fails as [`Scoreboard::set_real_team_stat`] does; the SDK networks -128 to
/// 127.
#[doc(alias("SetFlagCaptures", "m_nFlagCaptures"))]
pub fn set_team_flag_captures(
	server: Server<'_>,
	team: ScoringTeam,
	captures: i32,
) -> Result<(), ScoreboardError> {
	Scoreboard::new().set_real_team_stat(server, team, TeamStat::FlagCaptures, captures)
}

/// Sets a team's score, as `CTeam::SetScore` does, and marks it changed for
/// clients.
///
/// # Game consequences
///
/// This is the game's state, not only what clients see. A score reaching
/// `mp_winlimit`, or leading by `mp_windifference`, ends the map at the game's
/// next check (`tf_gamerules.cpp:9424-9470`), and arena and team balancing use
/// it too.
///
/// While a [`Scoreboard`] overrides [`TeamStat::Score`], set it through
/// [`Scoreboard::set_real_team_stat`] instead, which this otherwise matches.
///
/// Fails as [`Scoreboard::set_real_team_stat`] does.
#[doc(alias("SetScore", "m_iScore"))]
pub fn set_team_score(
	server: Server<'_>,
	team: ScoringTeam,
	score: i32,
) -> Result<(), ScoreboardError> {
	Scoreboard::new().set_real_team_stat(server, team, TeamStat::Score, score)
}

/// Finds a team's entity again through `cache`, or anew if the cached entity
/// is gone or has another class. What `state` remembers of the old entity is
/// then dropped, without writing to the new one.
fn team_entity<'c, 's>(
	cache: &'c mut Option<TeamCache>,
	state: &mut TeamSlot,
	context: Context<'s>,
	team: ScoringTeam,
) -> Result<(Entity<'s>, Edict<'s>, &'c TeamCache), ScoreboardError> {
	let found = cache
		.as_ref()
		.and_then(|cached| live(context.tools, cached.handle, cached.class));

	let (entity, edict) = match found {
		Some(found) => found,

		None => {
			if cache.take().is_some() {
				state.fields.iter_mut().for_each(FieldState::forget_memory);
			}

			let entity = find_team(context, team)?;
			let (class, edict) = networking(entity)?;

			*cache = Some(TeamCache::resolve(context.dll, entity, class));
			(entity, edict)
		}
	};

	let cached = cache.as_ref().ok_or(ScoreboardError::NoTeam(team))?;

	Ok((entity, edict, cached))
}

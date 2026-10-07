//! TF2's scoreboard, changed through the game's own scoring state.
//!
//! Everything here writes the state the game itself keeps and computes the
//! scoreboard from. The game, clients, and every other reader then agree on
//! the new values, and nothing has to be restored or kept applied: a change
//! lasts until the game itself resets or replaces the value, as described
//! [below](#resets).
//!
//! # Where the scoreboard comes from
//!
//! TF2 clients build the scoreboard from two kinds of networked entities:
//!
//! - `tf_player_manager` (`CTFPlayerResource`), which holds an array per
//!   column with an element per player slot. For each connected player, the
//!   game recomputes the elements from its own state at every think, ten
//!   times a second (`player_resource.cpp:94-101`); the damage, healing,
//!   support and credit columns at most once a second
//!   (`tf_player_resource.cpp:218-230`).
//! - `tf_team` (`CTFTeam`), whose score and flag captures the game only
//!   assigns when they change.
//!
//! The game stores no player's score. It keeps statistics per player, which
//! `sdk_raw`'s [`GameStats`] finds: for the session, which the scoreboard's
//! Score is computed from, and for the current round, which the round's score
//! and its MVPs are computed from. At each think, the player resource scores
//! the session's statistics with `CTFGameRules::CalcPlayerScore`, and copies
//! both scores and some of the statistics into the player's local scoring data
//! (`tf_player_resource.cpp:200-258`). [`PlayerScore`] reads and changes those
//! statistics, and scores them with the game's own function.
//!
//! # Points
//!
//! [`PlayerScore::add_points`] and [`PlayerScore::set_total`] change a
//! player's Score by exactly the amount asked, through
//! [`Stat::KillsRuneCarrier`]: the game scores it one point each, in every
//! mode and whatever the player's attributes, and shows or uploads it nowhere
//! else. The game also counts a kill of any player carrying a Mannpower rune
//! there, so [`PlayerScore::points`] includes those. The game never shows a
//! Score below 0: a total taken below it shows 0, and the deficit absorbs the
//! points the player earns next. [`Applied`] tells what clients see change.
//!
//! The other [`Stat`]s change the Score by their own weights, some of them
//! divided and floored (`tf_gamerules.cpp:17031-17093`), and feed the match
//! summaries and medals the game sends at the end of a competitive match.
//!
//! # What the game reports
//!
//! At each think, the player resource first copies each connected player's
//! session and round scores into the player's scoring data
//! (`m_Shared.m_ScoreData.m_iPoints` and `m_RoundScoreData.m_iPoints`). When
//! that raises the session's points, the game fires `player_score_changed`,
//! which the war tracker and match experience listen to
//! (`tf_player_shared.cpp:14574-14620`); a drop fires nothing. Then it
//! computes the player's Score again and compares it with the one it last
//! sent, the player's element of the resource's `m_iTotalScore`, and stores
//! the new one. Only that comparison reports anything: the game reports the
//! difference to the item servers as Strange "Points Scored" progress of the
//! player's cosmetics, or in Mann vs. Machine to its statistics
//! (`tf_player_resource.cpp:200-258`). A drop is reported as negative
//! progress.
//!
//! Changing statistics with [`PlayerScore`] reports none of this when it is
//! made: it also adds the change in each score to the values the game compares
//! against, and marks them changed for clients, which receive the new values at
//! once. Points the game awards are still reported as usual, including those it
//! awards between a change and its next think. The plugin's change stays in
//! those values, though, so when one of the game's [resets](#resets) later
//! drops the player's Score, the game reports the whole drop, including the
//! plugin's points, as negative progress. Vanilla would report only the points
//! the game awarded.
//!
//! # Reporting nothing
//!
//! [`PlayerScore::rebase`] sets the values the game compares against to the
//! scores themselves, discarding every difference the game has not sent yet.
//! Called at each of these points, it leaves the game nothing to report:
//!
//! - From a [`GameEventId::PlayerScoreChanged`] listener, which covers every
//!   point the game awards. The game fires the event from the think, after
//!   writing the session's points and before computing and comparing the
//!   Score, with nothing changing statistics in between, and the engine calls
//!   each listener before the event returns.
//! - From a [`GameEventId::ScorestatsAccumulatedReset`] listener. The game
//!   fires the event right after resetting every player's scores for
//!   `mp_restartgame`, a tournament restart, or the end of the wait for
//!   players, before any think (`teamplayroundbased_gamerules.cpp:3235-3282`).
//!   A hook after each player's `CTFPlayer::ResetScores`, at
//!   [`RESET_SCORES_SLOT`](sdk_raw::tf2::scoreboard::RESET_SCORES_SLOT), covers
//!   the same resets one player at a time, and also Mann vs. Machine's.
//! - From a [`GameEventId::PlayerActivate`] listener, or a hook after the
//!   game's `IServerGameClients::ClientActive`, which spawns the player
//!   (`tf_client.cpp:101-108`). The engine creates, spawns, and activates a
//!   player in one call, before any think. The game
//!   never clears a slot's element of `m_iTotalScore` when its player leaves,
//!   so a newcomer's first think compares against whoever last had the slot.
//!   A newcomer wears no items yet then, so vanilla's report of that
//!   difference reaches no Strange counter in practice; the rebase covers a
//!   player who spawns with items before that think.
//! - For every player when the plugin starts, unpauses, or a level starts,
//!   discarding what the game has not sent since its last think.
//! - For every connected player from a hook before each of the player
//!   resource's thinks, through `CBaseEntity::Think` at
//!   [`THINK_SLOT`](sdk_raw::entities::THINK_SLOT), which covers all of the
//!   above: the think then finds nothing to report, whatever the game awarded
//!   or reset since its last one. The resource also updates as a Mann vs.
//!   Machine wave completes and as a matchmade game reports its result
//!   (`tf_player_resource.cpp:70-80`, `tf_gamerules.cpp:2575`), which the hook
//!   does not precede.
//!
//! The game reports as usual when nothing rebases, as while the plugin is
//! paused, and in these cases:
//!
//! - After the plugin stops rebasing, a reset reports the whole drop of each
//!   player's Score, including the points the game awarded while it was
//!   rebased and never reported, as negative progress. Disconnecting and
//!   changing maps report nothing.
//! - [`reset_scores`] fires no `scorestats_accumulated_reset`, so the next
//!   think reports the drop unless [`PlayerScore::rebase`] follows it.
//! - The Score can change while no statistic does, and without an event: when
//!   the player's `scoreboard_minigame` attribute comes or goes, when the game
//!   switches to Mann vs. Machine's or Mannpower's scoring, when healing passes
//!   10,000,000, beyond which the game stops scoring it, or when a sum
//!   overflows. The next think reports the difference.
//! - A player resource created while a level runs starts every element of
//!   `m_iTotalScore` at 0, and so reports each player's whole Score at its
//!   first think.
//! - Mann vs. Machine's population manager resets players' scores without an
//!   event, though through each player's `ResetScores`, which a hook sees.
//! - Another plugin's `player_score_changed` listener that runs after the
//!   rebasing one and changes statistics leaves its change to be reported.
//!
//! # Resets
//!
//! The game resets a player's statistics, and so their Score, when they
//! connect, when they disconnect, when `mp_restartgame`, a tournament restart,
//! or the end of the wait for players resets every player's scores (after
//! which it fires `scorestats_accumulated_reset`), when [`reset_scores`] resets
//! them, which fires no event, and when Mann vs. Machine's population manager
//! resets them, which fires none either. A map change reconnects everyone. Changing team
//! resets nothing, and a team scramble only the teams' scores. The round's
//! statistics reset with every round (`stats_resetround`) and Mann vs.
//! Machine wave, leaving the session's alone. Frags and deaths reset with the
//! scores.
//!
//! A plugin that keeps its own points across these resets applies them again
//! after them: the session's after [`GameEventId::PlayerActivate`],
//! [`GameEventId::ScorestatsAccumulatedReset`], and its own [`reset_scores`],
//! but not after [`GameEventId::StatsResetround`], which leaves the session's
//! statistics, and so points already applied, in place.
//!
//! # Holding a score
//!
//! To keep a player's Score at a value of the plugin's choosing while they
//! play, listen to [`GameEventId::PlayerScoreChanged`] and call
//! [`PlayerScore::set_total`] and then [`PlayerScore::rebase`] from the
//! listener. The game fires the event from the player resource's think, after
//! updating the player's scoring data but before computing the Score it
//! sends, so the Score clients receive never changes, and the rebase leaves
//! the game nothing to report. `set_total` alone keeps the difference the game
//! has not sent, so the game would still report the points it awarded.
//!
//! The event only fires when the session's points rise, so the listener sees
//! no decrease or reset: set the held value and rebase again after the resets
//! above. Clients then receive the held Score at once.
//! A script's `ResetScores` on the player clears their scoring data but not
//! their statistics, so the next think fires the event with their whole
//! Score as its increase.
//!
//! A hook before each of the player resource's thinks can hold every
//! statistic the scoreboard shows, and the Score, without listening to any
//! event: set the session's and the round's with [`PlayerScore::set_stats`],
//! and the frags and deaths the think copies with [`PlayerScore::set_frags`]
//! and [`PlayerScore::set_deaths`], then call [`PlayerScore::rebase`]. The
//! think then shows exactly those, fires no `player_score_changed`, and
//! reports nothing, whatever the game changed since its last think.
//!
//! # Limits
//!
//! The scoreboard's Score is networked as an unsigned 32-bit varint, the
//! player's own points readout with 10 bits, so it wraps above 1023, and
//! frags and deaths with 12 signed bits, -2048 to 2047 in the SDK.
//!
//! # Unverified
//!
//! The game's functions and statistics are found and checked in TF2's 64-bit
//! Windows `server.dll` as [`GameStats`] describes. On Linux GNU servers they
//! are found by symbols inferred from the SDK's source, which have not been
//! checked against a retail build. The order of the player resource's think
//! and of `player_score_changed`'s listeners was read from the 64-bit Windows
//! `server.dll` and `engine.dll`; on Linux it is inferred from the SDK's
//! source. No change described here has yet been observed on a live server:
//! that clients receive it at once, that items' Strange counts stay as they
//! were, with or without [`PlayerScore::rebase`], and the client-side effects
//! of [`PlayerScore::set_killstreak`].
//!
//! [`GameEventId::PlayerActivate`]: crate::tf2::game_events::GameEventId::PlayerActivate
//! [`GameEventId::PlayerScoreChanged`]: crate::tf2::game_events::GameEventId::PlayerScoreChanged
//! [`GameEventId::ScorestatsAccumulatedReset`]: crate::tf2::game_events::GameEventId::ScorestatsAccumulatedReset
//! [`GameEventId::StatsResetround`]: crate::tf2::game_events::GameEventId::StatsResetround

#[cfg(test)]
#[path = "../../tests/tf2/scoreboard.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::datatables::{NetPropError, PropFlags, PropKind, SendProp, ServerClass, Storage};
use crate::edicts::Edict;
use crate::entities::{Entity, EntityHandle};
use crate::interfaces::{ServerGameDll, ServerTools, ValveEngine};
use crate::{Game, InterfaceError, Server};
use sdk_raw::edicts::MAX_CHANGE_OFFSETS;

use sdk_raw::tf2::scoreboard::{
	ELEMENT_SIZE, GameStats, GameStatsError, KILL_STREAK, MAX_PLAYERS_ARRAY_SAFE, PlayerStats,
	RoundStats, TF_TEAM_BLUE, TF_TEAM_RED, stat,
};

use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::mem::offset_of;
use std::ops::RangeInclusive;
use std::ptr::NonNull;

/// An exclusive bound on the datamap offsets trusted for a player's fields.
const MAX_FIELD_OFFSET: usize = 1 << 16;

/// The class name of TF2's `CTFPlayerResource` (`tf_player_resource.cpp:52`).
const RESOURCE_CLASS_NAME: &CStr = c"tf_player_manager";

/// The class name of TF2's `CTFTeam` (`tf_team.cpp:61`).
const TEAM_CLASS_NAME: &CStr = c"tf_team";

/// Which of a player's statistics a change applies to, besides the session's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Adjust {
	/// Whether the current round's statistics change too, and with them the
	/// round's score, which the game picks the round's MVPs by, and the
	/// player's round summary. `true` by default.
	///
	/// The game resets the round's statistics every round, but not the
	/// session's.
	pub round: bool,
}

impl Default for Adjust {
	fn default() -> Self {
		Self { round: true }
	}
}

/// How much a change moved the scores clients are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Applied {
	/// The change in the round's score, 0 unless [`Adjust::round`].
	pub round_shown: i32,

	/// The change in the session's score, the scoreboard's Score. It differs
	/// from the change asked for where the game's clamping at 0 hides part of
	/// it, or a statistic's weight is not 1.
	pub shown: i32,
}

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
	/// Resolves the array `name` in `class`, checking that it is a contiguous
	/// array of `int`s that holds at least player slot 1.
	fn resolve(
		dll: ServerGameDll<'_>,
		class: ServerClass<'_>,
		name: &'static CStr,
	) -> Result<Self, ScoreError> {
		let display_name = name.to_str().unwrap_or_default();
		let unexpected = || ScoreError::UnexpectedLayout {
			name: display_name,
			needed: 2,
		};

		let array = dll.net_prop(class, name)?;
		let len = array
			.element_count()
			.filter(|&len| len >= 2)
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

	/// Bytes from the start of the entity to the element for `slot`, or an
	/// error naming the array `name` if it does not reach it.
	fn offset(self, name: &'static str, slot: usize) -> Result<usize, ScoreError> {
		if slot < self.len {
			// `resolve` checked that every element's offset is this.
			Ok(self.first + slot * ELEMENT_SIZE)
		} else {
			Err(ScoreError::UnexpectedLayout {
				name,
				needed: slot.saturating_add(1),
			})
		}
	}
}

/// The variables of one entity marked changed during one write, sent to the
/// engine's change tracking together.
#[derive(Debug, Default)]
struct Changes {
	/// The offsets of the variables to mark changed, without duplicates.
	offsets: Vec<usize>,
}

impl Changes {
	/// Tells the engine about the changes to `edict`'s entity.
	///
	/// More offsets than the engine records per frame, or one it cannot
	/// record, mark the whole entity changed at once, as recording them one by
	/// one would end up doing.
	fn flush(self, engine: ValveEngine<'_>, edict: Edict<'_>) {
		if self.offsets.is_empty() {
			return;
		}

		let offsets: Option<Vec<u16>> = self
			.offsets
			.iter()
			.map(|&offset| u16::try_from(offset).ok())
			.collect();

		match offsets {
			Some(offsets) if offsets.len() <= usize::from(MAX_CHANGE_OFFSETS) => {
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
}

/// A count `CBasePlayer` keeps twice: as a datamap field the game counts
/// with, and in the player state the engine reads.
#[derive(Debug, Clone, Copy)]
struct Count<'s> {
	/// The datamap field, such as `m_iFrags`.
	field: IntField<'s>,

	/// The player state's copy, such as `pl.frags`.
	mirror: IntField<'s>,
}

impl Count<'_> {
	/// Writes `value` to the field, and to the player state's copy if that
	/// still agreed with the field. Returns whether it did.
	fn set(self, value: i32) -> bool {
		let agreed = self.mirror.read() == self.field.read();

		self.field.write(value);

		if agreed {
			self.mirror.write(value);
		}

		agreed
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

/// The values a networked integer can hold, from [`encodable_range`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetRange {
	min: i32,
	max: i32,
}

impl NetRange {
	/// Checks that `value` can be networked as the variable `name`.
	fn check(self, name: &'static str, value: i32) -> Result<(), ScoreError> {
		if (self.min..=self.max).contains(&value) {
			Ok(())
		} else {
			Err(ScoreError::OutOfRange {
				name,
				value,
				min: self.min,
				max: self.max,
			})
		}
	}

	const fn inclusive(self) -> RangeInclusive<i32> {
		self.min..=self.max
	}
}

/// Where a player class keeps its networked points, as resolved from its
/// server class.
#[derive(Debug, Clone, Copy)]
struct PlayerLayout {
	/// The address of the player's server class, which this describes.
	class: usize,

	/// `m_Shared.m_ScoreData.m_iPoints`: the session's score, as the player's
	/// own client receives it.
	points: usize,

	/// `m_Shared.m_RoundScoreData.m_iPoints`: the round's score.
	round_points: usize,
}

impl PlayerLayout {
	fn resolve(dll: ServerGameDll<'_>, class: ServerClass<'_>) -> Result<Self, ScoreError> {
		Ok(Self {
			class: class.as_ptr().addr(),
			points: scoring_points(dll, class, c"m_ScoreData")?,
			round_points: scoring_points(dll, class, c"m_RoundScoreData")?,
		})
	}
}

/// One TF2 player's scoring state, for one callback.
///
/// It reads and changes the statistics the game keeps for the player, which
/// the scoreboard's Score and the round's score are computed from, as the
/// [module documentation](self) describes, and the player's frags, deaths, and
/// kill streak. Every change is the game's own state from then on.
///
/// Like the [`Entity`] it is made for, it belongs to the callback its
/// [`Server`] does, and is `Copy`, but neither `Send` nor `Sync`.
#[derive(Debug, Clone, Copy)]
pub struct PlayerScore<'s> {
	edict: Edict<'s>,
	game_stats: GameStats,
	index: usize,
	layout: PlayerLayout,
	player: Entity<'s>,
	resource: Entity<'s>,
	resource_edict: Edict<'s>,
	server: Server<'s>,

	/// The game's statistics block for the player's entity index.
	stats: NonNull<PlayerStats>,

	/// The offset of the player's element of `m_iTotalScore` in the resource.
	total_score: usize,
}

impl<'s> PlayerScore<'s> {
	/// The scoring state of `player`, a TF2 player, as
	/// [`ScoreboardLayout::player`] finds it with a fresh layout.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, ScoreError> {
		ScoreboardLayout::new().player(server, player)
	}

	/// Adds `delta` deaths, as [`Self::set_deaths`] sets them.
	///
	/// Fails as [`Self::add_frags`] does.
	pub fn add_deaths(self, delta: i32) -> Result<bool, ScoreError> {
		let count = self.count(c"m_iDeaths", offset_of!(sys::CPlayerState, deaths))?;
		let deaths = count
			.field
			.read()
			.checked_add(delta)
			.ok_or(ScoreError::Overflow)?;

		Ok(count.set(deaths))
	}

	/// Adds `delta` frags, as [`Self::set_frags`] sets them.
	///
	/// Fails as [`Self::set_frags`] does, or with [`ScoreError::Overflow`],
	/// writing nothing, if the sum does not fit an `i32`.
	pub fn add_frags(self, delta: i32) -> Result<bool, ScoreError> {
		let count = self.count(c"m_iFrags", offset_of!(sys::CPlayerState, frags))?;
		let frags = count
			.field
			.read()
			.checked_add(delta)
			.ok_or(ScoreError::Overflow)?;

		Ok(count.set(frags))
	}

	/// Changes the player's Score by `delta` points, through
	/// [`Stat::KillsRuneCarrier`], as [`Self::add_stat`] does.
	///
	/// Each point counts once, whatever the game mode and the player's
	/// attributes, but the Score clients see never drops below 0:
	/// [`Applied::shown`] is the change they see. Points taken below 0 are
	/// owed, and absorb the points the player earns next.
	pub fn add_points(self, delta: i32, adjust: Adjust) -> Result<Applied, ScoreError> {
		self.add_stat(Stat::KillsRuneCarrier, delta, adjust)
	}

	/// Adds `delta` to one of the player's statistics for the session, and
	/// for the round if [`Adjust::round`], and returns how much that moved the
	/// scores clients see.
	///
	/// The scores move by the statistic's weight in the game's
	/// `CalcPlayerScore`, as the [module documentation](self#points) describes,
	/// and clients receive them at once. The game does not report the change
	/// to item servers, its Mann vs. Machine statistics, or
	/// `player_score_changed` listeners when it is made, but a later reset of
	/// the player's statistics reports the change as part of the Score it
	/// drops, as the [module documentation](self#what-the-game-reports)
	/// describes, unless [`Self::rebase`] follows the reset. Differences the
	/// game has not sent yet are kept, and reported as usual; [`Self::rebase`]
	/// discards them.
	///
	/// Fails with [`ScoreError::Overflow`], writing nothing, if a statistic, or
	/// a score the game compares against, would not fit an `i32`. The game sums
	/// the statistics as `int`s too, so keep them far from that bound.
	pub fn add_stat(self, stat: Stat, delta: i32, adjust: Adjust) -> Result<Applied, ScoreError> {
		let index = stat.index();
		let session = self.read_block(StatScope::Session);
		let round = self.read_block(StatScope::Round);

		let mut new_session = session;
		new_session.stat[index] = session.stat[index]
			.checked_add(delta)
			.ok_or(ScoreError::Overflow)?;

		let new_round = if adjust.round {
			let mut new_round = round;

			new_round.stat[index] = round.stat[index]
				.checked_add(delta)
				.ok_or(ScoreError::Overflow)?;
			Some(new_round)
		} else {
			None
		};

		// Both scores are clamped at 0, so their difference fits.
		let shown = self.score(&new_session) - self.score(&session);
		let round_shown =
			new_round.map_or(0, |new_round| self.score(&new_round) - self.score(&round));

		// The values the game compares against keep any difference it has not
		// sent yet, so only the change itself is added to them.
		let fields = self.reported_fields();
		let old = fields.read();
		let compensate =
			|value: i32, shown: i32| value.checked_add(shown).ok_or(ScoreError::Overflow);
		let new = ReportedScores {
			total: compensate(old.total, shown)?,
			points: compensate(old.points, shown)?,
			round_points: compensate(old.round_points, round_shown)?,
		};
		let engine = self.server.valve_engine()?;

		self.write_stat(StatScope::Session, index, new_session.stat[index]);

		if let Some(new_round) = new_round {
			self.write_stat(StatScope::Round, index, new_round.stat[index]);
		}

		self.write_reported(fields, new, engine);
		Ok(Applied { round_shown, shown })
	}

	/// The player's statistics block for `scope`, which the game owns.
	fn block(self, scope: StatScope) -> *mut RoundStats {
		match scope {
			StatScope::Session => PlayerStats::accumulated(self.stats.as_ptr()),
			StatScope::Round => PlayerStats::current_round(self.stats.as_ptr()),
		}
	}

	/// The values the player resource's column `name` can network.
	fn column_range(self, name: &'static CStr) -> Result<RangeInclusive<i32>, ScoreError> {
		let (class, _) = networking(self.resource)?;
		let layout = ArrayLayout::resolve(self.server.server_game_dll()?, class, name)?;

		layout.offset(name.to_str().unwrap_or_default(), self.index)?;

		Ok(layout.range.inclusive())
	}

	/// The count `name` and its copy in the player state, from
	/// `CBasePlayer`'s datamap.
	///
	/// The copy's place is the datamap's embedded `pl` plus its offset in the
	/// bindings' `CPlayerState`, `mirror`, which the game's `CPlayerState`
	/// datamap leaves out. The datamap must agree with the bindings on the
	/// fields it does declare.
	fn count(self, name: &'static CStr, mirror: usize) -> Result<Count<'s>, ScoreError> {
		let missing = |name: &'static CStr| ScoreError::MissingField {
			class: "CBasePlayer",
			name: name.to_str().unwrap_or_default(),
		};

		let map = self
			.player
			.data_maps()
			.find(|&map| map.class_name() == Some(c"CBasePlayer"))
			.ok_or_else(|| missing(name))?;

		let offset = map
			.field_offset(name, sys::_fieldtypes_FIELD_INTEGER)
			.filter(|&offset| offset < MAX_FIELD_OFFSET && offset.is_multiple_of(ELEMENT_SIZE))
			.ok_or_else(|| missing(name))?;

		let state = map
			.fields()
			.iter()
			.filter(|field| field.fieldType == sys::_fieldtypes_FIELD_EMBEDDED)
			.find(|field| field.name() == Some(c"pl"))
			.filter(|field| {
				field.embedded().next().is_some_and(|state| {
					state.class_name() == Some(c"CPlayerState")
						&& state.field_offset(c"deadflag", sys::_fieldtypes_FIELD_BOOLEAN)
							== Some(offset_of!(sys::CPlayerState, deadflag))
						&& state.field_offset(c"v_angle", sys::_fieldtypes_FIELD_VECTOR)
							== Some(offset_of!(sys::CPlayerState, v_angle))
				})
			})
			.and_then(|field| field.offset())
			.filter(|&state| {
				state.is_multiple_of(align_of::<sys::CPlayerState>())
					&& state
						.checked_add(size_of::<sys::CPlayerState>())
						.is_some_and(|end| end <= MAX_FIELD_OFFSET)
			})
			.ok_or_else(|| missing(c"pl"))?;

		// SAFETY: `CBasePlayer`'s own datamap, which the player's chain
		// includes, declares an `int` at `offset`, and embeds a `CPlayerState` at
		// `state`, whose datamap agrees with the bindings' layout, which places
		// an `int` at `mirror`. The game assigns both without notifying
		// anything.
		Ok(unsafe {
			Count {
				field: IntField::new(self.player, offset),
				mirror: IntField::new(self.player, state + mirror),
			}
		})
	}

	/// The player's death count, `CBasePlayer::m_iDeaths`, which the
	/// statistics panel shows.
	///
	/// Fails as [`Self::set_deaths`] does.
	#[doc(alias("m_iDeaths", "DeathCount"))]
	pub fn deaths(self) -> Result<i32, ScoreError> {
		Ok(self
			.count(c"m_iDeaths", offset_of!(sys::CPlayerState, deaths))?
			.field
			.read())
	}

	/// The deaths clients can be shown, from the player resource's `m_iDeaths`
	/// as the running game networks it: -2048 to 2047 in the SDK.
	pub fn deaths_range(self) -> Result<RangeInclusive<i32>, ScoreError> {
		self.column_range(c"m_iDeaths")
	}

	/// The player's frag count, `CBasePlayer::m_iFrags`, which the statistics
	/// panel shows as kills.
	///
	/// Fails as [`Self::set_frags`] does.
	#[doc(alias("m_iFrags", "FragCount"))]
	pub fn frags(self) -> Result<i32, ScoreError> {
		Ok(self
			.count(c"m_iFrags", offset_of!(sys::CPlayerState, frags))?
			.field
			.read())
	}

	/// The frags clients can be shown, from the player resource's `m_iScore`
	/// as the running game networks it: -2048 to 2047 in the SDK.
	pub fn frags_range(self) -> Result<RangeInclusive<i32>, ScoreError> {
		self.column_range(c"m_iScore")
	}

	/// The player's entity index, which is also their player slot.
	pub const fn index(self) -> usize {
		self.index
	}

	/// The player's kill streak, `m_nStreaks[kTFStreak_Kills]`, which clients
	/// show on the scoreboard and use for kill streak effects.
	///
	/// Fails if the running game does not network it as an `int`.
	#[doc(alias("m_nStreaks", "kTFStreak_Kills", "GetStreak"))]
	pub fn killstreak(self) -> Result<i32, ScoreError> {
		Ok(self.streak()?.read())
	}

	/// The player.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// The player's points: the session's [`Stat::KillsRuneCarrier`], which
	/// [`Self::add_points`] changes, including the kills of rune carriers the
	/// game counts there itself.
	pub fn points(self) -> i32 {
		self.stat(Stat::KillsRuneCarrier, StatScope::Session)
	}

	/// A copy of the player's statistics for `scope`.
	fn read_block(self, scope: StatScope) -> RoundStats {
		// SAFETY: `GameStats::player_stats` found the block in the game's
		// singleton, aligned for `int`s, and its statistics blocks within it; the
		// singleton lives as long as the game server module, which `Server::new`
		// keeps loaded during the callback. It is copied without forming a
		// reference, since the game writes it through its own pointers, on this
		// thread.
		unsafe { self.block(scope).read() }
	}

	/// Sets the values the game compares the player's scores against, and
	/// networks, to the scores themselves, and marks those that change for
	/// clients, so that the game reports nothing for the scores as they are.
	/// Returns what the game would otherwise have reported.
	///
	/// It writes the player's element of the player resource's
	/// `m_iTotalScore`, the Score the game last sent, and the session's and
	/// round's points of the player's scoring data
	/// (`m_Shared.m_ScoreData.m_iPoints` and `m_RoundScoreData.m_iPoints`), to
	/// the session's and round's scores the game's `CalcPlayerScore` computes
	/// for the player. The game's next think then finds no difference to
	/// report as Strange "Points Scored" progress, or in Mann vs. Machine to its
	/// statistics, and no rise of the session's points to fire
	/// `player_score_changed` for. Clients receive the scores at once.
	///
	/// Where and when to call it, and what it cannot cover, are in the
	/// [module documentation](self#reporting-nothing). It only reads and writes
	/// memory and runs `CalcPlayerScore`, which fires no event, so it can be
	/// called from inside a `player_score_changed` listener, which the game
	/// fires from the think it would report from.
	///
	/// Fails, writing nothing, if the engine's interface is unavailable.
	#[doc(alias("m_iTotalScore", "kKillEaterEvent_PointsScored"))]
	pub fn rebase(self) -> Result<Rebased, ScoreError> {
		let engine = self.server.valve_engine()?;
		let total = self.total();
		let fields = self.reported_fields();
		let old = fields.read();

		self.write_reported(
			fields,
			ReportedScores {
				total,
				points: total,
				round_points: self.round_total(),
			},
			engine,
		);

		Ok(Rebased {
			discarded: total.wrapping_sub(old.total),
			points_discarded: total.wrapping_sub(old.points),
		})
	}

	/// The values the game compares the player's scores against, and
	/// networks.
	fn reported_fields(self) -> ReportedFields<'s> {
		// SAFETY: The offsets were resolved from the send tables of the classes
		// the resource and the player had when this was made, during this
		// callback, and are `int`s, as `ArrayLayout` and `scoring_points`
		// checked.
		unsafe {
			ReportedFields {
				total: IntField::new(self.resource, self.total_score),
				points: IntField::new(self.player, self.layout.points),
				round_points: IntField::new(self.player, self.layout.round_points),
			}
		}
	}

	/// The round's score, as the game computes it for the round's MVPs and
	/// the player's round summary.
	pub fn round_total(self) -> i32 {
		self.score(&self.read_block(StatScope::Round))
	}

	/// Scores `stats` for the player with the game's `CalcPlayerScore`.
	fn score(self, stats: &RoundStats) -> i32 {
		// SAFETY: `Server::new` keeps the game server module, which `game_stats`
		// was resolved in, loaded for the whole callback (condition 1), and this
		// runs on the server's main thread within it (condition 3). `stats` is
		// borrowed for the call, which only reads it. The player is a live
		// `CTFPlayer` of the callback, as its datamaps showed, whose entity
		// pointer is its `CTFPlayer` pointer, as `sdk_raw::tf2` asserts; the game
		// reads its attributes through its items, which are entities of the
		// callback too, and frees nothing.
		unsafe {
			self.game_stats
				.calc_player_score(stats, self.player.as_ptr().cast::<sys::CTFPlayer>())
		}
	}

	/// Sets the player's death count, as [`Self::set_frags`] sets frags.
	///
	/// Fails as [`Self::set_frags`] does.
	#[doc(alias("m_iDeaths"))]
	pub fn set_deaths(self, deaths: i32) -> Result<bool, ScoreError> {
		Ok(self
			.count(c"m_iDeaths", offset_of!(sys::CPlayerState, deaths))?
			.set(deaths))
	}

	/// Sets the player's frag count, `CBasePlayer::m_iFrags`, which the game
	/// keeps counting from, and the copy in the player's state (`pl.frags`),
	/// which the engine, rather than the game, reports, as in the player list
	/// it gives Steam's server browser.
	///
	/// The player resource copies the frags into the statistics panel's kills
	/// at its next update. Clients see them wrapped into [`Self::frags_range`].
	/// The Score is computed from [`Stat::Kills`] instead, and does not change.
	///
	/// The engine's copy is only written if it agreed with the frag count
	/// before, as the game keeps it; returns whether it was. Code that sets the
	/// frag count alone leaves the copy behind until the game next counts a
	/// frag.
	///
	/// Fails if `CBasePlayer`'s datamap does not declare the count as an `int`
	/// at a plausible offset, or does not embed a player state laid out as the
	/// bindings' `CPlayerState`.
	#[doc(alias("m_iFrags"))]
	pub fn set_frags(self, frags: i32) -> Result<bool, ScoreError> {
		Ok(self
			.count(c"m_iFrags", offset_of!(sys::CPlayerState, frags))?
			.set(frags))
	}

	/// Sets the player's kill streak, which the player resource copies into
	/// the scoreboard at its next update, and marks it changed for clients.
	///
	/// The game counts on from it at the player's next kill, announcing kill
	/// streaks by it, and resets it when the player dies or respawns.
	///
	/// Fails if `killstreak` is negative or cannot be networked, or as
	/// [`Self::killstreak`] does.
	#[doc(alias("m_nStreaks", "kTFStreak_Kills"))]
	pub fn set_killstreak(self, killstreak: i32) -> Result<(), ScoreError> {
		let (field, range) = self.streak_with_range()?;
		let engine = self.server.valve_engine()?;

		range.check("m_nStreaks", killstreak)?;
		field.write(killstreak);

		let mut changes = Changes::default();

		changes.mark(field.offset);
		changes.flush(engine, self.edict);
		Ok(())
	}

	/// Sets one of the player's statistics for the session to `value`, and
	/// changes the round's by as much if [`Adjust::round`], as
	/// [`Self::add_stat`] does.
	pub fn set_stat(self, stat: Stat, value: i32, adjust: Adjust) -> Result<Applied, ScoreError> {
		let delta = value
			.checked_sub(self.stat(stat, StatScope::Session))
			.ok_or(ScoreError::Overflow)?;

		self.add_stat(stat, delta, adjust)
	}

	/// Sets the listed statistics of the player's block for `scope` to their
	/// values, and the block's points, [`Stat::KillsRuneCarrier`], so that it
	/// scores `total`, as [`Self::set_total`] does, and returns how much that
	/// moved the score clients see for `scope`.
	///
	/// As [`Self::add_stat`] does, it adds the change in the score to the values
	/// the game compares against, so the game reports nothing for it, and
	/// clients receive it at once. Setting both blocks this way, then
	/// [rebasing](Self::rebase), before each of the player resource's thinks
	/// shows exactly the statistics set, as the
	/// [module documentation](self#holding-a-score) describes.
	///
	/// Fails with [`ScoreError::Overflow`], writing nothing, if `total` exceeds
	/// `i32::MAX`, or the points, or a score the game compares against, would
	/// not fit an `i32`.
	pub fn set_stats(
		self,
		scope: StatScope,
		stats: &[(Stat, i32)],
		total: u32,
	) -> Result<Applied, ScoreError> {
		let total = i32::try_from(total).map_err(|_| ScoreError::Overflow)?;
		let old = self.read_block(scope);
		let mut new = old;

		for &(stat, value) in stats {
			new.stat[stat.index()] = value;
		}

		let unclamped = self.unclamped(new)?;

		// A block scoring below 0 already shows 0.
		if total != 0 || unclamped > 0 {
			let points = new.stat[stat::KILLS_RUNECARRIER];

			new.stat[stat::KILLS_RUNECARRIER] = total
				.checked_sub(unclamped)
				.and_then(|delta| points.checked_add(delta))
				.ok_or(ScoreError::Overflow)?;
		}

		if new.stat == old.stat {
			return Ok(Applied::default());
		}

		// Both scores are clamped at 0, so their difference fits.
		let change = self.score(&new) - self.score(&old);
		let fields = self.reported_fields();
		let mut reported = fields.read();
		let compensate = |value: i32| value.checked_add(change).ok_or(ScoreError::Overflow);

		let applied = match scope {
			StatScope::Session => {
				reported.total = compensate(reported.total)?;
				reported.points = compensate(reported.points)?;

				Applied {
					round_shown: 0,
					shown: change,
				}
			}

			StatScope::Round => {
				reported.round_points = compensate(reported.round_points)?;

				Applied {
					round_shown: change,
					shown: 0,
				}
			}
		};

		let engine = self.server.valve_engine()?;

		// Only the listed statistics and the points can differ.
		for (index, (&old, &new)) in old.stat.iter().zip(&new.stat).enumerate() {
			if new != old {
				self.write_stat(scope, index, new);
			}
		}

		self.write_reported(fields, reported, engine);
		Ok(applied)
	}

	/// Sets the player's Score to `target` exactly, by changing their points,
	/// as [`Self::add_points`] does, and the round's points by as much if
	/// [`Adjust::round`].
	///
	/// While the player's statistics score below 0, which the game shows as 0,
	/// the change needed is found by scoring copies of them with more points,
	/// the game's scoring being linear in points.
	///
	/// Fails as [`Self::add_points`] does, or with [`ScoreError::Overflow`],
	/// writing nothing, if `target` exceeds `i32::MAX`.
	pub fn set_total(self, target: u32, adjust: Adjust) -> Result<Applied, ScoreError> {
		let target = i32::try_from(target).map_err(|_| ScoreError::Overflow)?;
		let unclamped = self.unclamped(self.read_block(StatScope::Session))?;

		if target == 0 && unclamped <= 0 {
			return Ok(Applied::default());
		}

		let delta = target.checked_sub(unclamped).ok_or(ScoreError::Overflow)?;

		self.add_points(delta, adjust)
	}

	/// One of the player's statistics, for `scope`.
	pub fn stat(self, stat: Stat, scope: StatScope) -> i32 {
		self.read_block(scope).stat[stat.index()]
	}

	/// The player's kill streak variable.
	fn streak(self) -> Result<IntField<'s>, ScoreError> {
		self.streak_with_range().map(|(field, _)| field)
	}

	/// The player's kill streak variable, and the values it can hold.
	fn streak_with_range(self) -> Result<(IntField<'s>, NetRange), ScoreError> {
		let element = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, c"m_nStreaks")?
			.element(KILL_STREAK)?;
		let range = int_range(element.prop(), element.storage(), "m_nStreaks")?;

		// SAFETY: The offset was resolved from the send table of the player's
		// class, and is an `int`, as `int_range` checked.
		let field = unsafe { IntField::new(self.player, element.offset()) };

		Ok((
			field,
			NetRange {
				min: range.min.max(0),
				max: range.max,
			},
		))
	}

	/// The player's Score, as the game computes and shows it: the session's
	/// statistics scored with the game's `CalcPlayerScore`, including the terms
	/// of the player's `scoreboard_minigame` attribute, if any.
	#[doc(alias("m_iTotalScore", "CalcPlayerScore"))]
	pub fn total(self) -> i32 {
		self.score(&self.read_block(StatScope::Session))
	}

	/// The unclamped score of `stats`, which the game clamps at 0.
	///
	/// Below 0, it adds points to a copy, doubling them until the copy scores
	/// above 0, then takes them back off: the score is linear in points.
	fn unclamped(self, stats: RoundStats) -> Result<i32, ScoreError> {
		let score = self.score(&stats);

		if score > 0 {
			return Ok(score);
		}

		let points = stats.stat[stat::KILLS_RUNECARRIER];
		let mut lift: i32 = 1;

		loop {
			let mut probe = stats;

			probe.stat[stat::KILLS_RUNECARRIER] =
				points.checked_add(lift).ok_or(ScoreError::Overflow)?;

			let score = self.score(&probe);

			if score > 0 {
				return Ok(score - lift);
			}

			lift = lift.checked_mul(2).ok_or(ScoreError::Overflow)?;
		}
	}

	/// Writes each of `new`'s values that differs from `fields`', and marks it
	/// changed for clients.
	///
	/// The game compares what it computes with these, and only reports, and
	/// marks changed, a difference, so a value written without a mark would
	/// not reach clients until the game itself changes it.
	fn write_reported(
		self,
		fields: ReportedFields<'s>,
		new: ReportedScores,
		engine: ValveEngine<'_>,
	) {
		let old = fields.read();
		let mut resource_changes = Changes::default();
		let mut player_changes = Changes::default();

		if new.total != old.total {
			fields.total.write(new.total);
			resource_changes.mark(fields.total.offset);
		}

		if new.points != old.points {
			fields.points.write(new.points);
			player_changes.mark(fields.points.offset);
		}

		if new.round_points != old.round_points {
			fields.round_points.write(new.round_points);
			player_changes.mark(fields.round_points.offset);
		}

		resource_changes.flush(engine, self.resource_edict);
		player_changes.flush(engine, self.edict);
	}

	/// Writes one statistic of the player's block for `scope`.
	fn write_stat(self, scope: StatScope, index: usize, value: i32) {
		// SAFETY: As for `read_block`, and `index` is one of the block's
		// `TFSTAT_TOTAL` statistics, since a `Stat` gave it. The game writes its
		// statistics the same way, on this thread.
		unsafe {
			(&raw mut (*self.block(scope)).stat)
				.cast::<c_int>()
				.add(index)
				.write(value);
		}
	}
}

/// What [`PlayerScore::rebase`] discarded: the differences the game had not
/// sent yet. Each wraps as the game's own `int` subtraction does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rebased {
	/// The Score minus the one the game last sent: the change the player
	/// resource would have reported at its next think, as Strange "Points
	/// Scored" progress or to Mann vs. Machine's statistics.
	pub discarded: i32,

	/// The Score minus the session's points of the player's scoring data. Above
	/// 0, the game would have fired `player_score_changed` with this increase
	/// at its next think.
	pub points_discarded: i32,
}

/// The values of one player that the player resource compares the scores it
/// computes against at each think, and networks.
#[derive(Debug, Clone, Copy)]
struct ReportedFields<'s> {
	/// The player's element of the resource's `m_iTotalScore`: the Score the
	/// game last sent, which it reports changes from.
	total: IntField<'s>,

	/// `m_Shared.m_ScoreData.m_iPoints`: the session's score the game last
	/// copied into the player's scoring data, whose rises fire
	/// `player_score_changed`.
	points: IntField<'s>,

	/// `m_Shared.m_RoundScoreData.m_iPoints`: the round's score the game last
	/// copied into the player's scoring data.
	round_points: IntField<'s>,
}

impl ReportedFields<'_> {
	fn read(self) -> ReportedScores {
		ReportedScores {
			total: self.total.read(),
			points: self.points.read(),
			round_points: self.round_points.read(),
		}
	}
}

/// The values of [`ReportedFields`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReportedScores {
	total: i32,
	points: i32,
	round_points: i32,
}

/// What [`ScoreboardLayout`] keeps of the player resource it last found.
#[derive(Debug)]
struct ResourceCache {
	handle: EntityHandle,

	/// The address of the resource's server class, which `total_score`
	/// describes.
	class: usize,

	/// The array of `m_iTotalScore`, the scoreboard's Score.
	total_score: Result<ArrayLayout, ScoreError>,
}

impl ResourceCache {
	fn resolve(dll: ServerGameDll<'_>, entity: Entity<'_>, class: ServerClass<'_>) -> Self {
		Self {
			handle: entity.handle(),
			class: class.as_ptr().addr(),
			total_score: ArrayLayout::resolve(dll, class, c"m_iTotalScore"),
		}
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
	) -> Result<Self, ScoreError> {
		let variable = dll.net_prop(class, stat.name())?;

		Ok(Self {
			offset: variable.offset(),
			range: int_range(variable.prop(), variable.storage(), stat.display_name())?,
		})
	}
}

/// Why a scoreboard operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScoreError {
	/// The game's statistics or score calculation could not be found or
	/// trusted.
	#[error(transparent)]
	GameStats(#[from] GameStatsError),

	/// A required engine or game interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player's datamaps lack a field, or give it an implausible offset
	/// or layout.
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

	/// The game keeps no statistics block for the player's entity index, as
	/// for an entity index above `MAX_PLAYERS`, or a player without an edict.
	#[error("the game keeps no statistics for the player's entity index")]
	NoPlayerStats,

	/// No team entity has the team's number, as while no level runs.
	#[error("no `tf_team` entity has team number {}", .0.to_raw())]
	NoTeam(ScoringTeam),

	/// A variable is not a networked `int`.
	#[error("`{name}` is not a networked 32-bit integer")]
	NotAnInteger {
		/// The variable's name.
		name: &'static str,
	},

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

	/// A statistic, score, or count would not fit a 32-bit integer.
	#[error("the change would take a value outside 32-bit integers")]
	Overflow,

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
}

/// What the scoreboard resolves in the running game, kept by a plugin
/// between callbacks so that each [`PlayerScore`] and team change does not
/// resolve it again.
///
/// It holds no engine pointer: each call finds the entities again by handle,
/// and checks them against the server class their layout was resolved from,
/// so it stays sound across levels. The free functions and
/// [`PlayerScore::new`] use a fresh one each call.
#[derive(Debug, Default)]
pub struct ScoreboardLayout {
	/// The game's statistics, with the address of the `CreateInterface` of
	/// the game server module they were found in.
	game_stats: Option<(usize, GameStats)>,
	player: Option<PlayerLayout>,
	resource: Option<ResourceCache>,
	teams: [Option<TeamCache>; ScoringTeam::ALL.len()],
	_not_thread_safe: NotThreadSafe,
}

impl ScoreboardLayout {
	/// An empty layout, which resolves everything at its first use.
	pub fn new() -> Self {
		Self::default()
	}

	/// Adds `delta` to one of a team's numbers, as [`Self::set_team_stat`]
	/// sets it, and returns the new value.
	///
	/// Fails as [`Self::set_team_stat`] does, or with
	/// [`ScoreError::Overflow`], writing nothing.
	pub fn add_team_stat(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
		delta: i32,
	) -> Result<i32, ScoreError> {
		let (variable, range, edict, engine) = self.team_variable(server, team, stat)?;
		let value = variable
			.read()
			.checked_add(delta)
			.ok_or(ScoreError::Overflow)?;

		range.check(stat.display_name(), value)?;
		write_team_variable(variable, value, edict, engine);
		Ok(value)
	}

	/// The scoring state of `player`, a TF2 player.
	///
	/// The first call in the game server module finds and tests the game's
	/// statistics and score calculation, as `sdk_raw`'s [`GameStats::cached`]
	/// describes, running the game's `CalcPlayerScore` once; later calls reuse
	/// them, and the layout keeps them, so that its own later calls need not
	/// look the module up. This relies on Source never unloading that module
	/// while plugins are loaded.
	///
	/// Fails if the server does not run TF2, `player` is not a TF2 player with
	/// an entity index from 1 to `MAX_PLAYERS`, the game's statistics cannot
	/// be found or trusted, or no player resource exists or the running game
	/// lays out the scores differently.
	pub fn player<'s>(
		&mut self,
		server: Server<'s>,
		player: Entity<'s>,
	) -> Result<PlayerScore<'s>, ScoreError> {
		self.player_with(server, player, || {
			// SAFETY: `Server::new` guarantees that the game server module, whose
			// factory this is, stays loaded through the callback, on the server's
			// main thread, where the resolution's test calls `CalcPlayerScore`. A
			// cached resolution for the same factory and module base was made in
			// this same image: Source never unloads the game server module while
			// plugins are loaded, since Metamod:Source and the engine unload
			// plugins first, and the cache, a static of this plugin, is unloaded
			// with it.
			unsafe { GameStats::cached(server.game_server_factory().as_raw()) }
		})
	}

	/// The layout of `class`, a player's server class.
	fn player_layout(
		&mut self,
		dll: ServerGameDll<'_>,
		class: ServerClass<'_>,
	) -> Result<PlayerLayout, ScoreError> {
		if let Some(layout) = self.player
			&& layout.class == class.as_ptr().addr()
		{
			return Ok(layout);
		}

		let layout = PlayerLayout::resolve(dll, class)?;

		self.player = Some(layout);
		Ok(layout)
	}

	/// [`Self::player`], with the game's statistics from `game_stats`, which
	/// is only called once the player is checked, and unless the layout kept
	/// them for the same game server module.
	fn player_with<'s>(
		&mut self,
		server: Server<'s>,
		player: Entity<'s>,
		game_stats: impl FnOnce() -> Result<GameStats, GameStatsError>,
	) -> Result<PlayerScore<'s>, ScoreError> {
		check_game(server)?;

		if !player.has_data_map_class(c"CTFPlayer") {
			return Err(ScoreError::NotTfPlayer);
		}

		let index = player
			.index()
			.filter(|index| (1..MAX_PLAYERS_ARRAY_SAFE).contains(index))
			.ok_or(ScoreError::NoPlayerStats)?;
		let (class, edict) = networking(player)?;
		let context = Context::new(server)?;
		let factory = server.game_server_factory().as_raw() as usize;

		let game_stats = match self.game_stats {
			Some((module, game_stats)) if module == factory => game_stats,

			_ => {
				let game_stats = game_stats()?;

				self.game_stats = Some((factory, game_stats));
				game_stats
			}
		};

		let base = NonNull::new(player.as_ptr().cast::<sys::CBasePlayer>())
			.ok_or(ScoreError::NotTfPlayer)?;

		// SAFETY: As for `Self::player`'s resolution, the module stays loaded
		// and this runs on the main thread. Statistics the layout kept were
		// resolved in the module of the same factory, which is this one, since
		// Source never unloads the game server module while plugins are loaded.
		// The player is a live `CTFPlayer`, as its datamaps show, whose entity
		// pointer is its `CBasePlayer` pointer, as `sdk_raw::tf2` asserts, with
		// its live edict.
		let stats =
			unsafe { game_stats.player_stats(base, index) }.ok_or(ScoreError::NoPlayerStats)?;

		let layout = self.player_layout(context.dll, class)?;
		let (resource, resource_edict, cache) = resource_entity(&mut self.resource, context)?;
		let total_score = cache.total_score.clone()?.offset("m_iTotalScore", index)?;

		Ok(PlayerScore {
			edict,
			game_stats,
			index,
			layout,
			player,
			resource,
			resource_edict,
			server,
			stats,
			total_score,
		})
	}

	/// Sets one of a team's numbers in the game's own state, with the
	/// consequences [`set_team_score`] and [`set_team_flag_captures`]
	/// document, and marks it changed for clients.
	///
	/// Fails if the server does not run TF2, an interface is unavailable, the
	/// team has no entity, the running game does not network the number as an
	/// `int`, or `value` is outside [`Self::team_stat_range`].
	#[doc(alias("SetScore", "SetFlagCaptures"))]
	pub fn set_team_stat(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
		value: i32,
	) -> Result<(), ScoreError> {
		let (variable, range, edict, engine) = self.team_variable(server, team, stat)?;

		range.check(stat.display_name(), value)?;
		write_team_variable(variable, value, edict, engine);
		Ok(())
	}

	/// One of a team's numbers, as the game keeps it.
	///
	/// Fails as [`Self::set_team_stat`] does.
	pub fn team_stat(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
	) -> Result<i32, ScoreError> {
		let (variable, ..) = self.team_variable(server, team, stat)?;

		Ok(variable.read())
	}

	/// The values the running game can network for one of a team's numbers,
	/// from its networked variable's bit count and flags.
	pub fn team_stat_range(
		&mut self,
		server: Server<'_>,
		team: ScoringTeam,
		stat: TeamStat,
	) -> Result<RangeInclusive<i32>, ScoreError> {
		let (_, range, ..) = self.team_variable(server, team, stat)?;

		Ok(range.inclusive())
	}

	/// One of a team's numbers in its live entity, with what it can network,
	/// the entity's edict, and the engine.
	fn team_variable<'s>(
		&mut self,
		server: Server<'s>,
		team: ScoringTeam,
		stat: TeamStat,
	) -> Result<(IntField<'s>, NetRange, Edict<'s>, ValveEngine<'s>), ScoreError> {
		check_game(server)?;

		let context = Context::new(server)?;
		let (entity, edict, cache) = team_entity(&mut self.teams[team.index()], context, team)?;
		let scalar = cache.fields[stat.index()].clone()?;

		// SAFETY: The offset was resolved from the send table of the class
		// `team_entity` checked the entity still has, and is an `int`, as
		// `int_range` checked.
		let variable = unsafe { IntField::new(entity, scalar.offset) };

		Ok((variable, scalar.range, edict, context.engine))
	}
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

/// One of the statistics TF2 keeps for each player, `TFStatType_t`, numbered
/// as `sdk_raw`'s [`stat`] indices.
///
/// The game's `CalcPlayerScore` scores some of them, with the weights of
/// `sdk_raw`'s `TF_SCORE_` constants, as the [module documentation](self#points)
/// describes. The others only appear in summaries the game sends.
#[doc(alias("TFStatType_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Stat {
	/// Shots that hit.
	#[doc(alias("TFSTAT_SHOTS_HIT"))]
	ShotsHit = stat::SHOTS_HIT,

	/// Shots fired.
	#[doc(alias("TFSTAT_SHOTS_FIRED"))]
	ShotsFired = stat::SHOTS_FIRED,

	/// Kills, one point each, and one more with the `scoreboard_minigame`
	/// attribute.
	#[doc(alias("TFSTAT_KILLS"))]
	Kills = stat::KILLS,

	/// Deaths, which only score with the `scoreboard_minigame` attribute,
	/// minus three points each.
	#[doc(alias("TFSTAT_DEATHS"))]
	Deaths = stat::DEATHS,

	/// Damage dealt, a point per 600, or per 250 in Mann vs. Machine, together
	/// with the other damage and healing statistics the game scores.
	#[doc(alias("TFSTAT_DAMAGE"))]
	Damage = stat::DAMAGE,

	/// Captures of control points and flags, two points each, five in
	/// Mannpower, and one more with the `scoreboard_minigame` attribute.
	#[doc(alias("TFSTAT_CAPTURES"))]
	Captures = stat::CAPTURES,

	/// Defenses of control points and flags, one point each, and one more with
	/// the `scoreboard_minigame` attribute.
	#[doc(alias("TFSTAT_DEFENSES"))]
	Defenses = stat::DEFENSES,

	/// Dominations.
	#[doc(alias("TFSTAT_DOMINATIONS"))]
	Dominations = stat::DOMINATIONS,

	/// Revenges, one point each.
	#[doc(alias("TFSTAT_REVENGE"))]
	Revenge = stat::REVENGE,

	/// Points scored, which the game counts per life only.
	#[doc(alias("TFSTAT_POINTSSCORED"))]
	PointsScored = stat::POINTSSCORED,

	/// Enemy buildings destroyed, one point each, and one more with the
	/// `scoreboard_minigame` attribute.
	#[doc(alias("TFSTAT_BUILDINGSDESTROYED"))]
	BuildingsDestroyed = stat::BUILDINGSDESTROYED,

	/// Headshots, a point per two.
	#[doc(alias("TFSTAT_HEADSHOTS"))]
	Headshots = stat::HEADSHOTS,

	/// Seconds played.
	#[doc(alias("TFSTAT_PLAYTIME"))]
	PlayTime = stat::PLAYTIME,

	/// Healing given, scored as [`Self::Damage`] is, while it lies within 0 to
	/// 10,000,000.
	#[doc(alias("TFSTAT_HEALING"))]
	Healing = stat::HEALING,

	/// Invulnerabilities given, one point each.
	#[doc(alias("TFSTAT_INVULNS"))]
	Invulns = stat::INVULNS,

	/// Kill assists, a point per two.
	#[doc(alias("TFSTAT_KILLASSISTS"))]
	KillAssists = stat::KILLASSISTS,

	/// Backstabs, one point each.
	#[doc(alias("TFSTAT_BACKSTABS"))]
	Backstabs = stat::BACKSTABS,

	/// Health leached.
	#[doc(alias("TFSTAT_HEALTHLEACHED"))]
	HealthLeached = stat::HEALTHLEACHED,

	/// Buildings built.
	#[doc(alias("TFSTAT_BUILDINGSBUILT"))]
	BuildingsBuilt = stat::BUILDINGSBUILT,

	/// The most kills of one sentry gun.
	#[doc(alias("TFSTAT_MAXSENTRYKILLS"))]
	MaxSentryKills = stat::MAXSENTRYKILLS,

	/// Teleports given by the player's teleporters, a point per two.
	#[doc(alias("TFSTAT_TELEPORTS"))]
	Teleports = stat::TELEPORTS,

	/// Damage dealt by fire.
	#[doc(alias("TFSTAT_FIREDAMAGE"))]
	FireDamage = stat::FIREDAMAGE,

	/// Bonus points, a point per ten. The player resource's bonus column shows
	/// the round's.
	#[doc(alias("TFSTAT_BONUS_POINTS"))]
	BonusPoints = stat::BONUS_POINTS,

	/// Damage dealt by explosions.
	#[doc(alias("TFSTAT_BLASTDAMAGE"))]
	BlastDamage = stat::BLASTDAMAGE,

	/// Damage taken.
	#[doc(alias("TFSTAT_DAMAGETAKEN"))]
	DamageTaken = stat::DAMAGETAKEN,

	/// Health kits picked up.
	#[doc(alias("TFSTAT_HEALTHKITS"))]
	HealthKits = stat::HEALTHKITS,

	/// Ammunition kits picked up.
	#[doc(alias("TFSTAT_AMMOKITS"))]
	AmmoKits = stat::AMMOKITS,

	/// Changes of class.
	#[doc(alias("TFSTAT_CLASSCHANGES"))]
	ClassChanges = stat::CLASSCHANGES,

	/// Critical hits.
	#[doc(alias("TFSTAT_CRITS"))]
	Crits = stat::CRITS,

	/// Suicides.
	#[doc(alias("TFSTAT_SUICIDES"))]
	Suicides = stat::SUICIDES,

	/// Credits collected in Mann vs. Machine, a point per twenty.
	#[doc(alias("TFSTAT_CURRENCY_COLLECTED"))]
	CurrencyCollected = stat::CURRENCY_COLLECTED,

	/// Damage dealt by others with the player's help, scored as
	/// [`Self::Damage`] is.
	#[doc(alias("TFSTAT_DAMAGE_ASSIST"))]
	DamageAssist = stat::DAMAGE_ASSIST,

	/// Healing given by others with the player's help, scored as
	/// [`Self::Damage`] is.
	#[doc(alias("TFSTAT_HEALING_ASSIST"))]
	HealingAssist = stat::HEALING_ASSIST,

	/// Damage dealt to bosses, scored as [`Self::Damage`] is.
	#[doc(alias("TFSTAT_DAMAGE_BOSS"))]
	DamageBoss = stat::DAMAGE_BOSS,

	/// Damage blocked, scored as [`Self::Damage`] is.
	#[doc(alias("TFSTAT_DAMAGE_BLOCKED"))]
	DamageBlocked = stat::DAMAGE_BLOCKED,

	/// Damage dealt at range.
	#[doc(alias("TFSTAT_DAMAGE_RANGED"))]
	DamageRanged = stat::DAMAGE_RANGED,

	/// Damage dealt at range by random critical hits.
	#[doc(alias("TFSTAT_DAMAGE_RANGED_CRIT_RANDOM"))]
	DamageRangedCritRandom = stat::DAMAGE_RANGED_CRIT_RANDOM,

	/// Damage dealt at range by crit-boosted hits.
	#[doc(alias("TFSTAT_DAMAGE_RANGED_CRIT_BOOSTED"))]
	DamageRangedCritBoosted = stat::DAMAGE_RANGED_CRIT_BOOSTED,

	/// Revivals.
	#[doc(alias("TFSTAT_REVIVED"))]
	Revived = stat::REVIVED,

	/// Hits with throwables.
	#[doc(alias("TFSTAT_THROWABLEHIT"))]
	ThrowableHit = stat::THROWABLEHIT,

	/// Kills with throwables.
	#[doc(alias("TFSTAT_THROWABLEKILL"))]
	ThrowableKill = stat::THROWABLEKILL,

	/// The longest kill streak.
	#[doc(alias("TFSTAT_KILLSTREAK_MAX"))]
	KillstreakMax = stat::KILLSTREAK_MAX,

	/// Kills of players carrying a Mannpower rune, one point each, whatever
	/// the mode and the player's attributes. The game counts them for any rune
	/// carrier, and the scoreboard's points are kept here, as the
	/// [module documentation](self#points) describes.
	#[doc(alias("TFSTAT_KILLS_RUNECARRIER"))]
	KillsRuneCarrier = stat::KILLS_RUNECARRIER,

	/// Flags returned, four points each.
	#[doc(alias("TFSTAT_FLAGRETURNS"))]
	FlagReturns = stat::FLAGRETURNS,
}

impl Stat {
	/// The statistic's index in a `RoundStats_t`, its `TFStatType_t` value.
	pub const fn index(self) -> usize {
		self as usize
	}
}

/// Which of a player's statistics to read: the session's or the current
/// round's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatScope {
	/// Since the player connected or their scores were last reset,
	/// `statsAccumulated`, which the scoreboard's Score is computed from.
	#[doc(alias("statsAccumulated"))]
	Session,

	/// Since the current round started, `statsCurrentRound`, which the
	/// round's score is computed from.
	#[doc(alias("statsCurrentRound"))]
	Round,
}

/// What [`ScoreboardLayout`] keeps of a team entity it last found.
#[derive(Debug)]
struct TeamCache {
	handle: EntityHandle,

	/// The address of the team's server class, which `fields` describes.
	class: usize,

	/// Each number's variable, in [`TeamStat::ALL`]'s order.
	fields: [Result<Scalar, ScoreError>; TeamStat::ALL.len()],
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

/// Adds `delta` to a team's score, as [`set_team_score`] sets it, and returns
/// the new score.
///
/// Fails as [`set_team_score`] does, or with [`ScoreError::Overflow`].
#[doc(alias("AddScore", "m_iScore"))]
pub fn add_team_score(
	server: Server<'_>,
	team: ScoringTeam,
	delta: i32,
) -> Result<i32, ScoreError> {
	ScoreboardLayout::new().add_team_stat(server, team, TeamStat::Score, delta)
}

/// Fails with [`ScoreError::NotTf2`] unless the server runs TF2.
fn check_game(server: Server<'_>) -> Result<(), ScoreError> {
	if server.game() == Game::TeamFortress2 {
		Ok(())
	} else {
		Err(ScoreError::NotTf2)
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
fn find_team<'s>(context: Context<'s>, team: ScoringTeam) -> Result<Entity<'s>, ScoreError> {
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

	Err(ScoreError::NoTeam(team))
}

/// The range of an `int` variable named `name`, or an error if it is not one.
fn int_range(
	prop: SendProp<'_>,
	storage: Storage,
	name: &'static str,
) -> Result<NetRange, ScoreError> {
	if prop.kind() == PropKind::Int && storage.is_compatible(Storage::I32) {
		Ok(encodable_range(prop))
	} else {
		Err(ScoreError::NotAnInteger { name })
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
fn networking(entity: Entity<'_>) -> Result<(ServerClass<'_>, Edict<'_>), ScoreError> {
	match (entity.server_class(), entity.edict()) {
		(Some(class), Some(edict)) => Ok((class, edict)),

		_ => Err(NetPropError::NotNetworked {
			class_name: entity.class_name().to_string_lossy().into_owned(),
		}
		.into()),
	}
}

/// Resets `player`'s scoring through `CTFPlayer::ResetScores`, as
/// `mp_restartgame`, a tournament restart, and the end of the wait for players
/// do for every player.
///
/// This resets the statistics the game keeps for the player, which the
/// "Score" column is computed from, their frags and deaths, the statistics
/// panel's counts, and every domination and revenge relationship they are part
/// of, and drops their Mann vs. Machine events (`tf_player.cpp:3286-3294`).
/// Unlike the game's resets of every player, it fires no
/// `scorestats_accumulated_reset`. The player resource then reports the drop
/// in their Score at its next think, as a change of Strange "Points Scored"
/// progress, or of Mann vs. Machine statistics, unless
/// [`PlayerScore::rebase`] follows this, as the
/// [module documentation](self#reporting-nothing) describes.
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
/// a `RefCell` or thread-local, before calling this.
///
/// Fails if the server does not run TF2, or `player`'s datamaps do not include
/// `CTFPlayer`'s.
#[doc(alias("ResetScores"))]
pub fn reset_scores(server: Server<'_>, player: Entity<'_>) -> Result<(), ScoreError> {
	check_game(server)?;

	if !player.has_data_map_class(c"CTFPlayer") {
		return Err(ScoreError::NotTfPlayer);
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
/// entity is gone or has another class.
fn resource_entity<'c, 's>(
	cache: &'c mut Option<ResourceCache>,
	context: Context<'s>,
) -> Result<(Entity<'s>, Edict<'s>, &'c ResourceCache), ScoreError> {
	let found = cache
		.as_ref()
		.and_then(|cached| live(context.tools, cached.handle, cached.class));

	let (entity, edict) = match found {
		Some(found) => found,

		None => {
			*cache = None;

			let entity = context
				.tools
				.find_by_class_name(None, RESOURCE_CLASS_NAME)
				.ok_or(ScoreError::NoPlayerResource)?;
			let (class, edict) = networking(entity)?;

			*cache = Some(ResourceCache::resolve(context.dll, entity, class));
			(entity, edict)
		}
	};

	let cached = cache.as_ref().ok_or(ScoreError::NoPlayerResource)?;

	Ok((entity, edict, cached))
}

/// Converts to `i32`, saturating.
fn saturate(value: i64) -> i32 {
	i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// The offset of `m_iPoints` in the player's scoring data `table`, such as
/// `m_ScoreData`, from the player's class.
///
/// A name search finds only the first `m_iPoints`, so the variable is looked
/// up within the scoring data's own nested table.
fn scoring_points(
	dll: ServerGameDll<'_>,
	class: ServerClass<'_>,
	table: &'static CStr,
) -> Result<usize, ScoreError> {
	let data = dll.net_prop(class, table)?;

	if data.prop().kind() == PropKind::DataTable {
		for index in 0..data.element_count().unwrap_or(0) {
			let element = data.element(index)?;

			if element.prop().name() == c"m_iPoints" {
				int_range(element.prop(), element.storage(), "m_iPoints")?;

				return Ok(element.offset());
			}
		}
	}

	Err(NetPropError::NotFound {
		table: table.to_string_lossy().into_owned(),
		name: "m_iPoints".to_owned(),
	}
	.into())
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
/// The game resets it when each round starts (`tf_gamerules.cpp:15089-15098`).
///
/// Fails as [`ScoreboardLayout::set_team_stat`] does; the SDK networks -128 to
/// 127.
#[doc(alias("SetFlagCaptures", "m_nFlagCaptures"))]
pub fn set_team_flag_captures(
	server: Server<'_>,
	team: ScoringTeam,
	captures: i32,
) -> Result<(), ScoreError> {
	ScoreboardLayout::new().set_team_stat(server, team, TeamStat::FlagCaptures, captures)
}

/// Sets a team's score, as `CTeam::SetScore` does, and marks it changed for
/// clients.
///
/// # Game consequences
///
/// This is the game's state, not only what clients see. A score reaching
/// `mp_winlimit`, or leading by `mp_windifference`, ends the map at the game's
/// next check (`tf_gamerules.cpp:9424-9470`), and arena and team balancing use
/// it too. Restarts and some modes reset it.
///
/// Fails as [`ScoreboardLayout::set_team_stat`] does.
#[doc(alias("SetScore", "m_iScore"))]
pub fn set_team_score(server: Server<'_>, team: ScoringTeam, score: i32) -> Result<(), ScoreError> {
	ScoreboardLayout::new().set_team_stat(server, team, TeamStat::Score, score)
}

/// Finds a team's entity again through `cache`, or anew if the cached entity
/// is gone or has another class.
fn team_entity<'c, 's>(
	cache: &'c mut Option<TeamCache>,
	context: Context<'s>,
	team: ScoringTeam,
) -> Result<(Entity<'s>, Edict<'s>, &'c TeamCache), ScoreError> {
	let found = cache
		.as_ref()
		.and_then(|cached| live(context.tools, cached.handle, cached.class));

	let (entity, edict) = match found {
		Some(found) => found,

		None => {
			*cache = None;

			let entity = find_team(context, team)?;
			let (class, edict) = networking(entity)?;

			*cache = Some(TeamCache::resolve(context.dll, entity, class));
			(entity, edict)
		}
	};

	let cached = cache.as_ref().ok_or(ScoreError::NoTeam(team))?;

	Ok((entity, edict, cached))
}

/// Writes a team's variable and marks it changed, as the game's setters do.
fn write_team_variable(
	variable: IntField<'_>,
	value: i32,
	edict: Edict<'_>,
	engine: ValveEngine<'_>,
) {
	let mut changes = Changes::default();

	variable.write(value);
	changes.mark(variable.offset);
	changes.flush(engine, edict);
}

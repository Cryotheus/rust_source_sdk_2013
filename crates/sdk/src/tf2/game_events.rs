//! TF2's game events, identified by [`GameEventId`].

use crate::interfaces::game_event::GameEvent;
use sdk_raw::tf2::game_events as raw;
use std::ffi::{CStr, c_int};
use std::fmt::{Display, Formatter};

/// Declares the event ID enum from `Variant = "event_name"` pairs, documenting
/// and aliasing each variant with its event name and deriving every name form
/// from it.
macro_rules! game_events {
	(
		$(#[$EnumMeta:meta])*
		$Vis:vis $Enum:ident {
			$(
			$(#[$VariantMeta:meta])*
			$Variant:ident = $Name:literal
			),* $(,)?
		}
	) => {
		$(#[$EnumMeta])*
		#[allow(
			clippy::empty_docs,
			reason = "a blank line starts each event name's paragraph, which clippy cannot see"
		)]
		$Vis enum $Enum {
			$(
			$(#[$VariantMeta])*
			#[doc = ""]
			#[doc = concat!("The `", $Name, "` game event.")]
			#[doc(alias($Name))]
			$Variant,
			)*
		}

		/// Each event's name as bytes, named after its variant. A `const fn`
		/// cannot match `str` patterns, but can match these.
		#[allow(non_upper_case_globals, reason = "named after the variants")]
		mod names {
			$(pub(super) const $Variant: &[u8] = $Name.as_bytes();)*
		}

		impl $Enum {
			/// Every event ID, in declaration order.
			pub const ALL: &[Self] = &[$(Self::$Variant),*];

			/// Looks up the event named `bstr`, or returns `None` for a name
			/// without an ID.
			pub const fn from_bstr(bstr: &[u8]) -> Option<Self> {
				match bstr {
					$(names::$Variant => Some(Self::$Variant),)*
					_ => None,
				}
			}

			/// The event's name, such as `player_death`.
			#[doc(alias("GetName"))]
			pub const fn name(&self) -> &'static str {
				match self {
					$(Self::$Variant => $Name,)*
				}
			}

			/// The event's name as bytes, without a NUL terminator.
			pub const fn name_bstr(&self) -> &'static [u8] {
				self.name().as_bytes()
			}

			/// The event's name as a C string, as
			/// [`GameEventManager::add_listener`] and
			/// [`GameEventManager::create_event`] take it.
			///
			/// [`GameEventManager::add_listener`]: crate::interfaces::game_event::GameEventManager::add_listener
			/// [`GameEventManager::create_event`]: crate::interfaces::game_event::GameEventManager::create_event
			pub const fn name_cstr(&self) -> &'static CStr {
				match self {
					$(Self::$Variant => const {
						match CStr::from_bytes_with_nul(concat!($Name, "\0").as_bytes()) {
							Ok(name) => name,
							Err(_) => panic!("game event names contain no NUL"),
						}
					},)*
				}
			}
		}
	};
}

game_events! {
	/// The IDs associated with game events from Team Fortress 2.
	///
	/// Some client side events have been removed. [`GameEvent::id`] identifies
	/// an event. Searching the documentation for an event name, such as
	/// `player_death`, finds its variant.
	///
	/// [`GameEvent::id`]: GameEvent::id
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
	pub GameEventId {
		AchievementEarned = "achievement_earned",
		AchievementEarnedLocal = "achievement_earned_local",
		AirDash = "air_dash",
		ArenaMatchMaxstreak = "arena_match_maxstreak",
		ArenaPlayerNotification = "arena_player_notification",
		ArenaRoundStart = "arena_round_start",
		ArenaWinPanel = "arena_win_panel",
		ArrowImpact = "arrow_impact",

		// Omits `begin_xp_lerp`: "Really only for debugging".

		BuildingHealed = "building_healed",
		BuildingInfoChanged = "building_info_changed",
		CapperKilled = "capper_killed",
		ChristmasGiftGrab = "christmas_gift_grab",
		ClDrawline = "cl_drawline",
		CompetitiveStateChanged = "competitive_state_changed",
		CompetitiveStatsUpdate = "competitive_stats_update",
		CompetitiveVictory = "competitive_victory",
		CongaKill = "conga_kill",
		ControlpointEndtouch = "controlpoint_endtouch",
		ControlpointFakeCapture = "controlpoint_fake_capture",
		ControlpointFakeCaptureMult = "controlpoint_fake_capture_mult",
		ControlpointInitialized = "controlpoint_initialized",
		ControlpointPulseElement = "controlpoint_pulse_element",
		ControlpointStarttouch = "controlpoint_starttouch",
		ControlpointTimerUpdated = "controlpoint_timer_updated",
		ControlpointUnlockUpdated = "controlpoint_unlock_updated",
		ControlpointUpdatecapping = "controlpoint_updatecapping",
		ControlpointUpdateimages = "controlpoint_updateimages",
		ControlpointUpdatelayout = "controlpoint_updatelayout",
		ControlpointUpdateowner = "controlpoint_updateowner",
		CrossSpectralBridge = "cross_spectral_bridge",
		CrossbowHeal = "crossbow_heal",
		CtfFlagCaptured = "ctf_flag_captured",
		DamageMitigated = "damage_mitigated",
		DamagePrevented = "damage_prevented",
		DamageResisted = "damage_resisted",
		DeadringerCheatDeath = "deadringer_cheat_death",
		DemomanDetStickies = "demoman_det_stickies",
		DeployBuffBanner = "deploy_buff_banner",
		DoomsdayRocketOpen = "doomsday_rocket_open",
		DsScreenshot = "ds_screenshot",
		DsStop = "ds_stop",
		DuckXpLevelUp = "duck_xp_level_up",
		DuelStatus = "duel_status",
		EnvironmentalDeath = "environmental_death",
		EscapeHell = "escape_hell",
		EscapedLootIsland = "escaped_loot_island",
		EscortProgress = "escort_progress",
		EscortRecede = "escort_recede",
		EscortSpeed = "escort_speed",
		ExperienceChanged = "experience_changed",
		EyeballBossEscapeImminent = "eyeball_boss_escape_imminent",
		EyeballBossEscaped = "eyeball_boss_escaped",
		EyeballBossKilled = "eyeball_boss_killed",
		EyeballBossKiller = "eyeball_boss_killer",
		EyeballBossStunned = "eyeball_boss_stunned",
		EyeballBossSummoned = "eyeball_boss_summoned",

		/// > clone of "fish_notice" (...clone of "player_death")
		FishNoticeArm = "fish_notice__arm",

		FlagCarriedInDetectionZone = "flag_carried_in_detection_zone",
		FlagstatusUpdate = "flagstatus_update",
		FreezecamStarted = "freezecam_started",
		GameuiActivated = "gameui_activated",
		GameuiHidden = "gameui_hidden",
		GasDousedPlayerIgnited = "gas_doused_player_ignited",
		GlobalWarDataUpdated = "global_war_data_updated",
		HalloweenBossKilled = "halloween_boss_killed",
		HalloweenDuckCollected = "halloween_duck_collected",
		HalloweenPumpkinGrab = "halloween_pumpkin_grab",
		HalloweenSkeletonKilled = "halloween_skeleton_killed",
		HalloweenSoulCollected = "halloween_soul_collected",
		HideAnnotation = "hide_annotation",
		HideFreezepanel = "hide_freezepanel",
		IntroFinish = "intro_finish",
		IntroNextcamera = "intro_nextcamera",
		ItemFound = "item_found",
		ItemPickup = "item_pickup",
		ItemsAcknowledged = "items_acknowledged",
		KillInHell = "kill_in_hell",
		KillRefillsMeter = "kill_refills_meter",
		KilledBallCarrier = "killed_ball_carrier",
		KilledCappingPlayer = "killed_capping_player",
		Landed = "landed",
		LobbyUpdated = "lobby_updated",
		LocalplayerBecameobserver = "localplayer_becameobserver",
		LocalplayerBuiltobject = "localplayer_builtobject",
		LocalplayerChangeclass = "localplayer_changeclass",
		LocalplayerChangedisguise = "localplayer_changedisguise",
		LocalplayerChangeteam = "localplayer_changeteam",

		// Omits `localplayer_chargeready`: "client only".

		LocalplayerHealed = "localplayer_healed",
		LocalplayerPickupWeapon = "localplayer_pickup_weapon",
		LocalplayerRespawn = "localplayer_respawn",
		LocalplayerScoreChanged = "localplayer_score_changed",
		LocalplayerWinddown = "localplayer_winddown",
		MainmenuStabilized = "mainmenu_stabilized",

		/// > GC Match invites changed
		MatchInvitesUpdated = "match_invites_updated",

		MatchmakerStatsUpdated = "matchmaker_stats_updated",
		MedicDeath = "medic_death",
		MedicDefended = "medic_defended",
		MedigunShieldBlockedDamage = "medigun_shield_blocked_damage",
		MerasmusEscapeWarning = "merasmus_escape_warning",
		MerasmusEscaped = "merasmus_escaped",
		MerasmusKilled = "merasmus_killed",
		MerasmusPropFound = "merasmus_prop_found",
		MerasmusStunned = "merasmus_stunned",
		MerasmusSummoned = "merasmus_summoned",
		MinigameWin = "minigame_win",
		MinigameWon = "minigame_won",
		MmstatsUpdated = "mmstats_updated",
		MvmAdvWaveCompleteNoGates = "mvm_adv_wave_complete_no_gates",
		MvmAdvWaveKilledStunRadio = "mvm_adv_wave_killed_stun_radio",
		MvmBeginWave = "mvm_begin_wave",
		MvmBombAlarmTriggered = "mvm_bomb_alarm_triggered",
		MvmBombCarrierKilled = "mvm_bomb_carrier_killed",
		MvmBombDeployResetByPlayer = "mvm_bomb_deploy_reset_by_player",
		MvmBombResetByPlayer = "mvm_bomb_reset_by_player",
		MvmCreditbonusAll = "mvm_creditbonus_all",
		MvmCreditbonusAllAdvanced = "mvm_creditbonus_all_advanced",
		MvmCreditbonusWave = "mvm_creditbonus_wave",
		MvmKillRobotDeliveringBomb = "mvm_kill_robot_delivering_bomb",
		MvmMannhattanPit = "mvm_mannhattan_pit",
		MvmMedicPowerupShared = "mvm_medic_powerup_shared",
		MvmMissionComplete = "mvm_mission_complete",
		MvmMissionUpdate = "mvm_mission_update",
		MvmPickupCurrency = "mvm_pickup_currency",
		MvmQuickSentryUpgrade = "mvm_quick_sentry_upgrade",
		MvmResetStats = "mvm_reset_stats",
		MvmScoutMarkedForDeath = "mvm_scout_marked_for_death",
		MvmSentrybusterDetonate = "mvm_sentrybuster_detonate",
		MvmSentrybusterKilled = "mvm_sentrybuster_killed",
		MvmSniperHeadshotCurrency = "mvm_sniper_headshot_currency",
		MvmTankDestroyedByPlayers = "mvm_tank_destroyed_by_players",
		MvmWaveComplete = "mvm_wave_complete",
		MvmWaveFailed = "mvm_wave_failed",
		NavBlocked = "nav_blocked",
		NpcHurt = "npc_hurt",
		NumCappersChanged = "num_cappers_changed",
		ObjectDeflected = "object_deflected",
		ObjectDestroyed = "object_destroyed",
		ObjectDetonated = "object_detonated",
		ObjectRemoved = "object_removed",
		OvertimeNag = "overtime_nag",
		ParachuteDeploy = "parachute_deploy",
		ParachuteHolster = "parachute_holster",
		PartyChat = "party_chat",

		/// > Party's effective criteria has changed
		PartyCriteriaChanged = "party_criteria_changed",

		/// > Party's invite list changed
		PartyInvitesChanged = "party_invites_changed",

		PartyMemberJoin = "party_member_join",
		PartyMemberLeave = "party_member_leave",

		/// > If one of the persisted party client preferences was changed (ignore invites etc.)
		PartyPrefChanged = "party_pref_changed",

		/// > Party's in-queue state changed
		PartyQueueStateChanged = "party_queue_state_changed",

		/// > If your party changes ~at all~, heavyweight
		PartyUpdated = "party_updated",

		/// > passtime
		PassBallBlocked = "pass_ball_blocked",

		/// > passtime
		PassBallStolen = "pass_ball_stolen",

		/// > passtime
		PassFree = "pass_free",

		/// > passtime
		PassGet = "pass_get",

		/// > passtime
		PassPassCaught = "pass_pass_caught",

		/// > passtime
		PassScore = "pass_score",

		PathTrackPassed = "path_track_passed",
		PayloadPushed = "payload_pushed",
		PingUpdated = "ping_updated",
		PlayerAbandonedMatch = "player_abandoned_match",
		PlayerAccountChanged = "player_account_changed",
		/// A player finished connecting and entered the game. The engine fires
		/// it, not the game.
		PlayerActivate = "player_activate",
		PlayerAskedforball = "player_askedforball",
		PlayerBonuspoints = "player_bonuspoints",
		PlayerBuff = "player_buff",

		/// > Objects built by players (sentry gun, teleporter, etc.)
		/// <br><br>
		/// Some object events have "object" and some have "objecttype".
		/// We can't change them as there are third-party
		/// scripts that listen for these events.
		PlayerBuiltobject = "player_builtobject",

		PlayerBuyback = "player_buyback",
		PlayerCalledformedic = "player_calledformedic",
		PlayerCarryobject = "player_carryobject",
		PlayerChangeclass = "player_changeclass",
		PlayerChargedeployed = "player_chargedeployed",

		/// A client began connecting, before it loads the level or has a player.
		/// Bots connect too, with `bot` set. The engine fires it, not the game,
		/// and only to the server's listeners.
		PlayerConnect = "player_connect",

		/// As [`PlayerConnect`](Self::PlayerConnect), without the address, for
		/// clients' listeners. The engine fires it.
		PlayerConnectClient = "player_connect_client",
		PlayerCurrencyChanged = "player_currency_changed",
		PlayerDamageDodged = "player_damage_dodged",
		PlayerDamaged = "player_damaged",
		PlayerDeath = "player_death",
		PlayerDestroyedPipebomb = "player_destroyed_pipebomb",
		PlayerDirecthitStun = "player_directhit_stun",

		/// A client is disconnecting, including one that had not finished
		/// connecting. Its player, if it had one, still exists. The engine fires
		/// it, not the game.
		PlayerDisconnect = "player_disconnect",

		PlayerDomination = "player_domination",
		PlayerDropobject = "player_dropobject",
		PlayerEscortScore = "player_escort_score",
		PlayerExtinguished = "player_extinguished",
		PlayerHealed = "player_healed",
		PlayerHealedbymedic = "player_healedbymedic",

		// Omits `player_healedmediccall`: "client only".

		PlayerHealonhit = "player_healonhit",
		PlayerHighfiveCancel = "player_highfive_cancel",
		PlayerHighfiveStart = "player_highfive_start",
		PlayerHighfiveSuccess = "player_highfive_success",
		PlayerHurt = "player_hurt",
		PlayerIgnited = "player_ignited",
		PlayerIgnitedInv = "player_ignited_inv",
		PlayerInitialSpawn = "player_initial_spawn",
		PlayerInvulned = "player_invulned",
		PlayerJarated = "player_jarated",
		PlayerJaratedFade = "player_jarated_fade",
		PlayerKilledAchievementZone = "player_killed_achievement_zone",
		PlayerMvp = "player_mvp",
		PlayerNextMapVoteChange = "player_next_map_vote_change",
		PlayerPinned = "player_pinned",
		PlayerRegenerate = "player_regenerate",
		PlayerRematchChange = "player_rematch_change",
		PlayerRocketpackPushed = "player_rocketpack_pushed",
		PlayerSappedObject = "player_sapped_object",
		PlayerScoreChanged = "player_score_changed",
		PlayerShieldBlocked = "player_shield_blocked",
		PlayerSpawn = "player_spawn",
		PlayerStatsUpdated = "player_stats_updated",
		PlayerStealsandvich = "player_stealsandvich",
		PlayerStunned = "player_stunned",

		/// A player's team is changing; the team lists still hold the old team.
		/// Fired only for a change, including on disconnecting (`disconnect`).
		PlayerTeam = "player_team",

		PlayerTeleported = "player_teleported",
		PlayerTurnedToGhost = "player_turned_to_ghost",
		PlayerUpgraded = "player_upgraded",
		PlayerUpgradedobject = "player_upgradedobject",
		PlayerUsedPowerupBottle = "player_used_powerup_bottle",
		PlayingCommentary = "playing_commentary",
		PostInventoryApplication = "post_inventory_application",
		ProjectileDirectHit = "projectile_direct_hit",
		ProjectileRemoved = "projectile_removed",
		ProtoDefChanged = "proto_def_changed",
		PumpkinLordKilled = "pumpkin_lord_killed",
		PumpkinLordSummoned = "pumpkin_lord_summoned",
		PveWinPanel = "pve_win_panel",
		QuestMapDataChanged = "quest_map_data_changed",
		QuestObjectiveCompleted = "quest_objective_completed",
		QuestProgress = "quest_progress",
		QuestRequest = "quest_request",
		QuestResponse = "quest_response",
		QuestTurnInState = "quest_turn_in_state",
		QuestlogOpened = "questlog_opened",
		RaidSpawnMob = "raid_spawn_mob",
		RaidSpawnSquad = "raid_spawn_squad",
		RdPlayerScorePoints = "rd_player_score_points",
		RdRobotImpact = "rd_robot_impact",
		RdRobotKilled = "rd_robot_killed",
		RdRulesStateChanged = "rd_rules_state_changed",
		RdTeamPointsChanged = "rd_team_points_changed",
		RecalculateHolidays = "recalculate_holidays",
		RecalculateTruce = "recalculate_truce",
		RematchFailedToCreate = "rematch_failed_to_create",
		RematchVotePeriodOver = "rematch_vote_period_over",
		RemoveNemesisRelationships = "remove_nemesis_relationships",
		RespawnGhost = "respawn_ghost",
		RestartTimerTime = "restart_timer_time",
		RevivePlayerComplete = "revive_player_complete",
		RevivePlayerNotify = "revive_player_notify",
		RevivePlayerStopped = "revive_player_stopped",
		RocketJump = "rocket_jump",
		RocketJumpLanded = "rocket_jump_landed",
		RocketpackLanded = "rocketpack_landed",
		RocketpackLaunch = "rocketpack_launch",
		RpsTauntEvent = "rps_taunt_event",
		SchemaUpdated = "schema_updated",
		ScorestatsAccumulatedReset = "scorestats_accumulated_reset",
		ScorestatsAccumulatedUpdate = "scorestats_accumulated_update",
		ScoutGrandSlam = "scout_grand_slam",

		/// Declared twice for some reason.
		ScoutSlamdollLanded = "scout_slamdoll_landed",

		SentryOnGoActive = "sentry_on_go_active",
		ShowAnnotation = "show_annotation",
		ShowClassLayout = "show_class_layout",
		ShowFreezepanel = "show_freezepanel",
		ShowMatchSummary = "show_match_summary",
		ShowVsPanel = "show_vs_panel",
		SkeletonKilledQuest = "skeleton_killed_quest",
		SkeletonKingKilledQuest = "skeleton_king_killed_quest",

		/// > clone of player_death = "player_death",
		SlapNotice = "slap_notice",

		SpecTargetUpdated = "spec_target_updated",
		SpecialScore = "special_score",
		SpyPdaReset = "spy_pda_reset",
		StatsResetround = "stats_resetround",
		StickyJump = "sticky_jump",
		StickyJumpLanded = "sticky_jump_landed",
		StopWatchChanged = "stop_watch_changed",
		TaggedPlayerAsIt = "tagged_player_as_it",
		TeamLeaderKilled = "team_leader_killed",
		TeamplayAlert = "teamplay_alert",
		TeamplayBroadcastAudio = "teamplay_broadcast_audio",
		TeamplayCaptureBlocked = "teamplay_capture_blocked",
		TeamplayCaptureBroken = "teamplay_capture_broken",
		TeamplayFlagEvent = "teamplay_flag_event",
		TeamplayGameOver = "teamplay_game_over",
		TeamplayMapTimeRemaining = "teamplay_map_time_remaining",
		TeamplayOvertimeBegin = "teamplay_overtime_begin",
		TeamplayOvertimeEnd = "teamplay_overtime_end",
		TeamplayPointCaptured = "teamplay_point_captured",
		TeamplayPointLocked = "teamplay_point_locked",
		TeamplayPointStartcapture = "teamplay_point_startcapture",
		TeamplayPointUnlocked = "teamplay_point_unlocked",
		TeamplayPreRoundTimeLeft = "teamplay_pre_round_time_left",
		TeamplayReadyRestart = "teamplay_ready_restart",
		TeamplayRestartRound = "teamplay_restart_round",
		TeamplayRoundActive = "teamplay_round_active",
		TeamplayRoundRestartSeconds = "teamplay_round_restart_seconds",
		TeamplayRoundSelected = "teamplay_round_selected",
		TeamplayRoundStalemate = "teamplay_round_stalemate",
		TeamplayRoundStart = "teamplay_round_start",
		TeamplayRoundWin = "teamplay_round_win",
		TeamplaySetupFinished = "teamplay_setup_finished",
		TeamplaySuddendeathBegin = "teamplay_suddendeath_begin",
		TeamplaySuddendeathEnd = "teamplay_suddendeath_end",
		TeamplayTeamReady = "teamplay_team_ready",
		TeamplayTeambalancedPlayer = "teamplay_teambalanced_player",
		TeamplayTimerFlash = "teamplay_timer_flash",
		TeamplayTimerTimeAdded = "teamplay_timer_time_added",
		TeamplayUpdateTimer = "teamplay_update_timer",
		TeamplayWaitingAbouttoend = "teamplay_waiting_abouttoend",
		TeamplayWaitingBegins = "teamplay_waiting_begins",
		TeamplayWaitingEnds = "teamplay_waiting_ends",
		TeamplayWinPanel = "teamplay_win_panel",
		TeamsChanged = "teams_changed",
		TfGameOver = "tf_game_over",
		TfMapTimeRemaining = "tf_map_time_remaining",

		/// > clone of "player_death" with added counts
		ThrowableHit = "throwable_hit",

		TopStreamsRequestFinished = "top_streams_request_finished",
		TournamentEnablecountdown = "tournament_enablecountdown",
		TournamentStateupdate = "tournament_stateupdate",
		TrainingComplete = "training_complete",
		UpdateStatusItem = "update_status_item",
		UpgradesFileChanged = "upgrades_file_changed",
		/// Accepted ballot; `entityid` is an entity index, not a user ID.
		VoteCast = "vote_cast",
		VoteMapsChanged = "vote_maps_changed",
		/// A vote has been created; `voteidx` correlates its later ballots.
		VoteOptions = "vote_options",
		WinlimitChanged = "winlimit_changed",
		WinpanelShowScores = "winpanel_show_scores",
		WorldStatusChanged = "world_status_changed",
	}
}

bitflags::bitflags! {
	/// The `TF_DEATH_*` flags of [`GameEventId::PlayerDeath`]'s `death_flags`,
	/// from `game/shared/tf/tf_shareddefs.h`.
	///
	/// Read them with [`from_bits_retain`](Self::from_bits_retain), which
	/// keeps bits without a constant here.
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct DeathFlags: c_int {
		/// The killer is dominating the victim.
		#[doc(alias("TF_DEATH_DOMINATION"))]
		const DOMINATION = raw::TF_DEATH_DOMINATION;

		/// The assister is dominating the victim.
		#[doc(alias("TF_DEATH_ASSISTER_DOMINATION"))]
		const ASSISTER_DOMINATION = raw::TF_DEATH_ASSISTER_DOMINATION;

		/// The killer got revenge on the victim.
		#[doc(alias("TF_DEATH_REVENGE"))]
		const REVENGE = raw::TF_DEATH_REVENGE;

		/// The assister got revenge on the victim.
		#[doc(alias("TF_DEATH_ASSISTER_REVENGE"))]
		const ASSISTER_REVENGE = raw::TF_DEATH_ASSISTER_REVENGE;

		/// The death triggered a first blood.
		#[doc(alias("TF_DEATH_FIRST_BLOOD"))]
		const FIRST_BLOOD = raw::TF_DEATH_FIRST_BLOOD;

		/// A feigned death, by a Spy's Dead Ringer: the victim lives on.
		#[doc(alias("TF_DEATH_FEIGN_DEATH"))]
		const FEIGN_DEATH = raw::TF_DEATH_FEIGN_DEATH;

		/// The death interrupted the victim doing an important game event,
		/// like capturing a point or carrying the flag.
		#[doc(alias("TF_DEATH_INTERRUPTED"))]
		const INTERRUPTED = raw::TF_DEATH_INTERRUPTED;

		/// The victim was gibbed.
		#[doc(alias("TF_DEATH_GIBBED"))]
		const GIBBED = raw::TF_DEATH_GIBBED;

		/// The victim died while in purgatory.
		#[doc(alias("TF_DEATH_PURGATORY"))]
		const PURGATORY = raw::TF_DEATH_PURGATORY;

		/// The victim was a miniboss.
		#[doc(alias("TF_DEATH_MINIBOSS"))]
		const MINIBOSS = raw::TF_DEATH_MINIBOSS;

		/// The victim was killed by an Australium weapon.
		#[doc(alias("TF_DEATH_AUSTRALIUM"))]
		const AUSTRALIUM = raw::TF_DEATH_AUSTRALIUM;
	}
}

impl GameEvent<'_> {
	/// Identifies the event by its [name](Self::name), or returns `None` for
	/// an event [`GameEventId`] does not list.
	pub fn id(self) -> Option<GameEventId> {
		GameEventId::from_cstr(self.name())
	}
}

impl GameEventId {
	/// Looks up the event named `cstr`, such as [`GameEvent::name`] returns,
	/// or returns `None` for a name without an ID.
	///
	/// [`GameEvent::name`]: GameEvent::name
	pub const fn from_cstr(cstr: &CStr) -> Option<Self> {
		Self::from_bstr(cstr.to_bytes())
	}

	/// Looks up the event named `str`, or returns `None` for a name without an
	/// ID.
	pub const fn from_str(str: &str) -> Option<Self> {
		Self::from_bstr(str.as_bytes())
	}
}

impl Display for GameEventId {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.name())
	}
}

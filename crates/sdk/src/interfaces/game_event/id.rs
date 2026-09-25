use std::ffi::CStr;
use std::fmt::{Display, Formatter};

macro_rules! game_events {
	(
		$(#[$EnumMeta:meta])*
		$Vis:vis $Enum:ident {
			$(
			$(#[$VariantMeta:meta])*
			$Variant:ident = $BName:literal $CName:literal
			),* $(,)?
		}
	) => {
		$(#[$EnumMeta])*
		$Vis enum $Enum {
			$(
			$(#[$VariantMeta])*
			$Variant,
			)*
		}

		impl $Enum {
			pub const ALL: &[Self] = &[$(Self::$Variant),*];

			pub const fn from_bstr(bstr: &[u8]) -> Option<Self> {
				match bstr {
					$($BName => Some(Self::$Variant),)*
					_ => None,
				}
			}

			pub const fn name_bstr(&self) -> &'static [u8] {
				match self {
					$(Self::$Variant => $BName,)*
				}
			}

			pub const fn name_cstr(&self) -> &'static CStr {
				match self {
					$(Self::$Variant => $CName,)*
				}
			}
		}
	};
}

game_events! {
	/// The IDs associated with game events from Team Fortress 2.
	///
	/// Some client side events have been removed.
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
	pub GameEventId {
		AchievementEarned = b"achievement_earned" c"achievement_earned",
		AchievementEarnedLocal = b"achievement_earned_local" c"achievement_earned_local",
		AirDash = b"air_dash" c"air_dash",
		ArenaMatchMaxstreak = b"arena_match_maxstreak" c"arena_match_maxstreak",
		ArenaPlayerNotification = b"arena_player_notification" c"arena_player_notification",
		ArenaRoundStart = b"arena_round_start" c"arena_round_start",
		ArenaWinPanel = b"arena_win_panel" c"arena_win_panel",
		ArrowImpact = b"arrow_impact" c"arrow_impact",

		// /// > Really only for debugging
		// BeginXpLerp = b"begin_xp_lerp", c"begin_xp_lerp",

		BuildingHealed = b"building_healed" c"building_healed",
		BuildingInfoChanged = b"building_info_changed" c"building_info_changed",
		CapperKilled = b"capper_killed" c"capper_killed",
		ChristmasGiftGrab = b"christmas_gift_grab" c"christmas_gift_grab",
		ClDrawline = b"cl_drawline" c"cl_drawline",
		CompetitiveStateChanged = b"competitive_state_changed" c"competitive_state_changed",
		CompetitiveStatsUpdate = b"competitive_stats_update" c"competitive_stats_update",
		CompetitiveVictory = b"competitive_victory" c"competitive_victory",
		CongaKill = b"conga_kill" c"conga_kill",
		ControlpointEndtouch = b"controlpoint_endtouch" c"controlpoint_endtouch",
		ControlpointFakeCapture = b"controlpoint_fake_capture" c"controlpoint_fake_capture",
		ControlpointFakeCaptureMult = b"controlpoint_fake_capture_mult" c"controlpoint_fake_capture_mult",
		ControlpointInitialized = b"controlpoint_initialized" c"controlpoint_initialized",
		ControlpointPulseElement = b"controlpoint_pulse_element" c"controlpoint_pulse_element",
		ControlpointStarttouch = b"controlpoint_starttouch" c"controlpoint_starttouch",
		ControlpointTimerUpdated = b"controlpoint_timer_updated" c"controlpoint_timer_updated",
		ControlpointUnlockUpdated = b"controlpoint_unlock_updated" c"controlpoint_unlock_updated",
		ControlpointUpdatecapping = b"controlpoint_updatecapping" c"controlpoint_updatecapping",
		ControlpointUpdateimages = b"controlpoint_updateimages" c"controlpoint_updateimages",
		ControlpointUpdatelayout = b"controlpoint_updatelayout" c"controlpoint_updatelayout",
		ControlpointUpdateowner = b"controlpoint_updateowner" c"controlpoint_updateowner",
		CrossSpectralBridge = b"cross_spectral_bridge" c"cross_spectral_bridge",
		CrossbowHeal = b"crossbow_heal" c"crossbow_heal",
		CtfFlagCaptured = b"ctf_flag_captured" c"ctf_flag_captured",
		DamageMitigated = b"damage_mitigated" c"damage_mitigated",
		DamagePrevented = b"damage_prevented" c"damage_prevented",
		DamageResisted = b"damage_resisted" c"damage_resisted",
		DeadringerCheatDeath = b"deadringer_cheat_death" c"deadringer_cheat_death",
		DemomanDetStickies = b"demoman_det_stickies" c"demoman_det_stickies",
		DeployBuffBanner = b"deploy_buff_banner" c"deploy_buff_banner",
		DoomsdayRocketOpen = b"doomsday_rocket_open" c"doomsday_rocket_open",
		DsScreenshot = b"ds_screenshot" c"ds_screenshot",
		DsStop = b"ds_stop" c"ds_stop",
		DuckXpLevelUp = b"duck_xp_level_up" c"duck_xp_level_up",
		DuelStatus = b"duel_status" c"duel_status",
		EnvironmentalDeath = b"environmental_death" c"environmental_death",
		EscapeHell = b"escape_hell" c"escape_hell",
		EscapedLootIsland = b"escaped_loot_island" c"escaped_loot_island",
		EscortProgress = b"escort_progress" c"escort_progress",
		EscortRecede = b"escort_recede" c"escort_recede",
		EscortSpeed = b"escort_speed" c"escort_speed",
		ExperienceChanged = b"experience_changed" c"experience_changed",
		EyeballBossEscapeImminent = b"eyeball_boss_escape_imminent" c"eyeball_boss_escape_imminent",
		EyeballBossEscaped = b"eyeball_boss_escaped" c"eyeball_boss_escaped",
		EyeballBossKilled = b"eyeball_boss_killed" c"eyeball_boss_killed",
		EyeballBossKiller = b"eyeball_boss_killer" c"eyeball_boss_killer",
		EyeballBossStunned = b"eyeball_boss_stunned" c"eyeball_boss_stunned",
		EyeballBossSummoned = b"eyeball_boss_summoned" c"eyeball_boss_summoned",

		/// > clone of "fish_notice" (...clone of "player_death")
		FishNoticeArm = b"fish_notice__arm" c"fish_notice__arm",

		FlagCarriedInDetectionZone = b"flag_carried_in_detection_zone" c"flag_carried_in_detection_zone",
		FlagstatusUpdate = b"flagstatus_update" c"flagstatus_update",
		FreezecamStarted = b"freezecam_started" c"freezecam_started",
		GameuiActivated = b"gameui_activated" c"gameui_activated",
		GameuiHidden = b"gameui_hidden" c"gameui_hidden",
		GasDousedPlayerIgnited = b"gas_doused_player_ignited" c"gas_doused_player_ignited",
		GlobalWarDataUpdated = b"global_war_data_updated" c"global_war_data_updated",
		HalloweenBossKilled = b"halloween_boss_killed" c"halloween_boss_killed",
		HalloweenDuckCollected = b"halloween_duck_collected" c"halloween_duck_collected",
		HalloweenPumpkinGrab = b"halloween_pumpkin_grab" c"halloween_pumpkin_grab",
		HalloweenSkeletonKilled = b"halloween_skeleton_killed" c"halloween_skeleton_killed",
		HalloweenSoulCollected = b"halloween_soul_collected" c"halloween_soul_collected",
		HideAnnotation = b"hide_annotation" c"hide_annotation",
		HideFreezepanel = b"hide_freezepanel" c"hide_freezepanel",
		IntroFinish = b"intro_finish" c"intro_finish",
		IntroNextcamera = b"intro_nextcamera" c"intro_nextcamera",
		ItemFound = b"item_found" c"item_found",
		ItemPickup = b"item_pickup" c"item_pickup",
		ItemsAcknowledged = b"items_acknowledged" c"items_acknowledged",
		KillInHell = b"kill_in_hell" c"kill_in_hell",
		KillRefillsMeter = b"kill_refills_meter" c"kill_refills_meter",
		KilledBallCarrier = b"killed_ball_carrier" c"killed_ball_carrier",
		KilledCappingPlayer = b"killed_capping_player" c"killed_capping_player",
		Landed = b"landed" c"landed",
		LobbyUpdated = b"lobby_updated" c"lobby_updated",
		LocalplayerBecameobserver = b"localplayer_becameobserver" c"localplayer_becameobserver",
		LocalplayerBuiltobject = b"localplayer_builtobject" c"localplayer_builtobject",
		LocalplayerChangeclass = b"localplayer_changeclass" c"localplayer_changeclass",
		LocalplayerChangedisguise = b"localplayer_changedisguise" c"localplayer_changedisguise",
		LocalplayerChangeteam = b"localplayer_changeteam" c"localplayer_changeteam",

		// /// > client only
		// LocalplayerChargeready = b"localplayer_chargeready", c"localplayer_chargeready",

		LocalplayerHealed = b"localplayer_healed" c"localplayer_healed",
		LocalplayerPickupWeapon = b"localplayer_pickup_weapon" c"localplayer_pickup_weapon",
		LocalplayerRespawn = b"localplayer_respawn" c"localplayer_respawn",
		LocalplayerScoreChanged = b"localplayer_score_changed" c"localplayer_score_changed",
		LocalplayerWinddown = b"localplayer_winddown" c"localplayer_winddown",
		MainmenuStabilized = b"mainmenu_stabilized" c"mainmenu_stabilized",

		/// > GC Match invites changed
		MatchInvitesUpdated = b"match_invites_updated" c"match_invites_updated",

		MatchmakerStatsUpdated = b"matchmaker_stats_updated" c"matchmaker_stats_updated",
		MedicDeath = b"medic_death" c"medic_death",
		MedicDefended = b"medic_defended" c"medic_defended",
		MedigunShieldBlockedDamage = b"medigun_shield_blocked_damage" c"medigun_shield_blocked_damage",
		MerasmusEscapeWarning = b"merasmus_escape_warning" c"merasmus_escape_warning",
		MerasmusEscaped = b"merasmus_escaped" c"merasmus_escaped",
		MerasmusKilled = b"merasmus_killed" c"merasmus_killed",
		MerasmusPropFound = b"merasmus_prop_found" c"merasmus_prop_found",
		MerasmusStunned = b"merasmus_stunned" c"merasmus_stunned",
		MerasmusSummoned = b"merasmus_summoned" c"merasmus_summoned",
		MinigameWin = b"minigame_win" c"minigame_win",
		MinigameWon = b"minigame_won" c"minigame_won",
		MmstatsUpdated = b"mmstats_updated" c"mmstats_updated",
		MvmAdvWaveCompleteNoGates = b"mvm_adv_wave_complete_no_gates" c"mvm_adv_wave_complete_no_gates",
		MvmAdvWaveKilledStunRadio = b"mvm_adv_wave_killed_stun_radio" c"mvm_adv_wave_killed_stun_radio",
		MvmBeginWave = b"mvm_begin_wave" c"mvm_begin_wave",
		MvmBombAlarmTriggered = b"mvm_bomb_alarm_triggered" c"mvm_bomb_alarm_triggered",
		MvmBombCarrierKilled = b"mvm_bomb_carrier_killed" c"mvm_bomb_carrier_killed",
		MvmBombDeployResetByPlayer = b"mvm_bomb_deploy_reset_by_player" c"mvm_bomb_deploy_reset_by_player",
		MvmBombResetByPlayer = b"mvm_bomb_reset_by_player" c"mvm_bomb_reset_by_player",
		MvmCreditbonusAll = b"mvm_creditbonus_all" c"mvm_creditbonus_all",
		MvmCreditbonusAllAdvanced = b"mvm_creditbonus_all_advanced" c"mvm_creditbonus_all_advanced",
		MvmCreditbonusWave = b"mvm_creditbonus_wave" c"mvm_creditbonus_wave",
		MvmKillRobotDeliveringBomb = b"mvm_kill_robot_delivering_bomb" c"mvm_kill_robot_delivering_bomb",
		MvmMannhattanPit = b"mvm_mannhattan_pit" c"mvm_mannhattan_pit",
		MvmMedicPowerupShared = b"mvm_medic_powerup_shared" c"mvm_medic_powerup_shared",
		MvmMissionComplete = b"mvm_mission_complete" c"mvm_mission_complete",
		MvmMissionUpdate = b"mvm_mission_update" c"mvm_mission_update",
		MvmPickupCurrency = b"mvm_pickup_currency" c"mvm_pickup_currency",
		MvmQuickSentryUpgrade = b"mvm_quick_sentry_upgrade" c"mvm_quick_sentry_upgrade",
		MvmResetStats = b"mvm_reset_stats" c"mvm_reset_stats",
		MvmScoutMarkedForDeath = b"mvm_scout_marked_for_death" c"mvm_scout_marked_for_death",
		MvmSentrybusterDetonate = b"mvm_sentrybuster_detonate" c"mvm_sentrybuster_detonate",
		MvmSentrybusterKilled = b"mvm_sentrybuster_killed" c"mvm_sentrybuster_killed",
		MvmSniperHeadshotCurrency = b"mvm_sniper_headshot_currency" c"mvm_sniper_headshot_currency",
		MvmTankDestroyedByPlayers = b"mvm_tank_destroyed_by_players" c"mvm_tank_destroyed_by_players",
		MvmWaveComplete = b"mvm_wave_complete" c"mvm_wave_complete",
		MvmWaveFailed = b"mvm_wave_failed" c"mvm_wave_failed",
		NavBlocked = b"nav_blocked" c"nav_blocked",
		NpcHurt = b"npc_hurt" c"npc_hurt",
		NumCappersChanged = b"num_cappers_changed" c"num_cappers_changed",
		ObjectDeflected = b"object_deflected" c"object_deflected",
		ObjectDestroyed = b"object_destroyed" c"object_destroyed",
		ObjectDetonated = b"object_detonated" c"object_detonated",
		ObjectRemoved = b"object_removed" c"object_removed",
		OvertimeNag = b"overtime_nag" c"overtime_nag",
		ParachuteDeploy = b"parachute_deploy" c"parachute_deploy",
		ParachuteHolster = b"parachute_holster" c"parachute_holster",
		PartyChat = b"party_chat" c"party_chat",

		/// > Party's effective criteria has changed
		PartyCriteriaChanged = b"party_criteria_changed" c"party_criteria_changed",

		/// > Party's invite list changed
		PartyInvitesChanged = b"party_invites_changed" c"party_invites_changed",

		PartyMemberJoin = b"party_member_join" c"party_member_join",
		PartyMemberLeave = b"party_member_leave" c"party_member_leave",

		/// > If one of the persisted party client preferences was changed (ignore invites etc.)
		PartyPrefChanged = b"party_pref_changed" c"party_pref_changed",

		/// > Party's in-queue state changed
		PartyQueueStateChanged = b"party_queue_state_changed" c"party_queue_state_changed",

		/// > If your party changes ~at all~, heavyweight
		PartyUpdated = b"party_updated" c"party_updated",

		/// > passtime
		PassBallBlocked = b"pass_ball_blocked" c"pass_ball_blocked",

		/// > passtime
		PassBallStolen = b"pass_ball_stolen" c"pass_ball_stolen",

		/// > passtime
		PassFree = b"pass_free" c"pass_free",

		/// > passtime
		PassGet = b"pass_get" c"pass_get",

		/// > passtime
		PassPassCaught = b"pass_pass_caught" c"pass_pass_caught",

		/// > passtime
		PassScore = b"pass_score" c"pass_score",

		PathTrackPassed = b"path_track_passed" c"path_track_passed",
		PayloadPushed = b"payload_pushed" c"payload_pushed",
		PingUpdated = b"ping_updated" c"ping_updated",
		PlayerAbandonedMatch = b"player_abandoned_match" c"player_abandoned_match",
		PlayerAccountChanged = b"player_account_changed" c"player_account_changed",
		PlayerAskedforball = b"player_askedforball" c"player_askedforball",
		PlayerBonuspoints = b"player_bonuspoints" c"player_bonuspoints",
		PlayerBuff = b"player_buff" c"player_buff",

		/// > Objects built by players (sentry gun, teleporter, etc.)
		/// <br><br>
		/// Some object events have "object" and some have "objecttype".
		/// We can't change them as there are third-party
		/// scripts that listen for these events.
		PlayerBuiltobject = b"player_builtobject" c"player_builtobject",

		PlayerBuyback = b"player_buyback" c"player_buyback",
		PlayerCalledformedic = b"player_calledformedib" c"player_calledformedib",
		PlayerCarryobject = b"player_carryobject" c"player_carryobject",
		PlayerChangeclass = b"player_changeclass" c"player_changeclass",
		PlayerChargedeployed = b"player_chargedeployed" c"player_chargedeployed",
		PlayerCurrencyChanged = b"player_currency_changed" c"player_currency_changed",
		PlayerDamageDodged = b"player_damage_dodged" c"player_damage_dodged",
		PlayerDamaged = b"player_damaged" c"player_damaged",
		PlayerDeath = b"player_death" c"player_death",
		PlayerDestroyedPipebomb = b"player_destroyed_pipebomb" c"player_destroyed_pipebomb",
		PlayerDirecthitStun = b"player_directhit_stun" c"player_directhit_stun",
		PlayerDomination = b"player_domination" c"player_domination",
		PlayerDropobject = b"player_dropobject" c"player_dropobject",
		PlayerEscortScore = b"player_escort_score" c"player_escort_score",
		PlayerExtinguished = b"player_extinguished" c"player_extinguished",
		PlayerHealed = b"player_healed" c"player_healed",
		PlayerHealedbymedic = b"player_healedbymedib" c"player_healedbymedib",

		// /// > client only
		// PlayerHealedmediccall = b"player_healedmediccall", c"player_healedmediccall",

		PlayerHealonhit = b"player_healonhit" c"player_healonhit",
		PlayerHighfiveCancel = b"player_highfive_cancel" c"player_highfive_cancel",
		PlayerHighfiveStart = b"player_highfive_start" c"player_highfive_start",
		PlayerHighfiveSuccess = b"player_highfive_success" c"player_highfive_success",
		PlayerHurt = b"player_hurt" c"player_hurt",
		PlayerIgnited = b"player_ignited" c"player_ignited",
		PlayerIgnitedInv = b"player_ignited_inv" c"player_ignited_inv",
		PlayerInitialSpawn = b"player_initial_spawn" c"player_initial_spawn",
		PlayerInvulned = b"player_invulned" c"player_invulned",
		PlayerJarated = b"player_jarated" c"player_jarated",
		PlayerJaratedFade = b"player_jarated_fade" c"player_jarated_fade",
		PlayerKilledAchievementZone = b"player_killed_achievement_zone" c"player_killed_achievement_zone",
		PlayerMvp = b"player_mvp" c"player_mvp",
		PlayerNextMapVoteChange = b"player_next_map_vote_change" c"player_next_map_vote_change",
		PlayerPinned = b"player_pinned" c"player_pinned",
		PlayerRegenerate = b"player_regenerate" c"player_regenerate",
		PlayerRematchChange = b"player_rematch_change" c"player_rematch_change",
		PlayerRocketpackPushed = b"player_rocketpack_pushed" c"player_rocketpack_pushed",
		PlayerSappedObject = b"player_sapped_object" c"player_sapped_object",
		PlayerScoreChanged = b"player_score_changed" c"player_score_changed",
		PlayerShieldBlocked = b"player_shield_blocked" c"player_shield_blocked",
		PlayerSpawn = b"player_spawn" c"player_spawn",
		PlayerStatsUpdated = b"player_stats_updated" c"player_stats_updated",
		PlayerStealsandvich = b"player_stealsandvich" c"player_stealsandvich",
		PlayerStunned = b"player_stunned" c"player_stunned",
		PlayerTeleported = b"player_teleported" c"player_teleported",
		PlayerTurnedToGhost = b"player_turned_to_ghost" c"player_turned_to_ghost",
		PlayerUpgraded = b"player_upgraded" c"player_upgraded",
		PlayerUpgradedobject = b"player_upgradedobject" c"player_upgradedobject",
		PlayerUsedPowerupBottle = b"player_used_powerup_bottle" c"player_used_powerup_bottle",
		PlayingCommentary = b"playing_commentary" c"playing_commentary",
		PostInventoryApplication = b"post_inventory_application" c"post_inventory_application",
		ProjectileDirectHit = b"projectile_direct_hit" c"projectile_direct_hit",
		ProjectileRemoved = b"projectile_removed" c"projectile_removed",
		ProtoDefChanged = b"proto_def_changed" c"proto_def_changed",
		PumpkinLordKilled = b"pumpkin_lord_killed" c"pumpkin_lord_killed",
		PumpkinLordSummoned = b"pumpkin_lord_summoned" c"pumpkin_lord_summoned",
		PveWinPanel = b"pve_win_panel" c"pve_win_panel",
		QuestMapDataChanged = b"quest_map_data_changed" c"quest_map_data_changed",
		QuestObjectiveCompleted = b"quest_objective_completed" c"quest_objective_completed",
		QuestProgress = b"quest_progress" c"quest_progress",
		QuestRequest = b"quest_request" c"quest_request",
		QuestResponse = b"quest_response" c"quest_response",
		QuestTurnInState = b"quest_turn_in_state" c"quest_turn_in_state",
		QuestlogOpened = b"questlog_opened" c"questlog_opened",
		RaidSpawnMob = b"raid_spawn_mob" c"raid_spawn_mob",
		RaidSpawnSquad = b"raid_spawn_squad" c"raid_spawn_squad",
		RdPlayerScorePoints = b"rd_player_score_points" c"rd_player_score_points",
		RdRobotImpact = b"rd_robot_impact" c"rd_robot_impact",
		RdRobotKilled = b"rd_robot_killed" c"rd_robot_killed",
		RdRulesStateChanged = b"rd_rules_state_changed" c"rd_rules_state_changed",
		RdTeamPointsChanged = b"rd_team_points_changed" c"rd_team_points_changed",
		RecalculateHolidays = b"recalculate_holidays" c"recalculate_holidays",
		RecalculateTruce = b"recalculate_truce" c"recalculate_truce",
		RematchFailedToCreate = b"rematch_failed_to_create" c"rematch_failed_to_create",
		RematchVotePeriodOver = b"rematch_vote_period_over" c"rematch_vote_period_over",
		RemoveNemesisRelationships = b"remove_nemesis_relationships" c"remove_nemesis_relationships",
		RespawnGhost = b"respawn_ghost" c"respawn_ghost",
		RestartTimerTime = b"restart_timer_time" c"restart_timer_time",
		RevivePlayerComplete = b"revive_player_complete" c"revive_player_complete",
		RevivePlayerNotify = b"revive_player_notify" c"revive_player_notify",
		RevivePlayerStopped = b"revive_player_stopped" c"revive_player_stopped",
		RocketJump = b"rocket_jump" c"rocket_jump",
		RocketJumpLanded = b"rocket_jump_landed" c"rocket_jump_landed",
		RocketpackLanded = b"rocketpack_landed" c"rocketpack_landed",
		RocketpackLaunch = b"rocketpack_launch" c"rocketpack_launch",
		RpsTauntEvent = b"rps_taunt_event" c"rps_taunt_event",
		SchemaUpdated = b"schema_updated" c"schema_updated",
		ScorestatsAccumulatedReset = b"scorestats_accumulated_reset" c"scorestats_accumulated_reset",
		ScorestatsAccumulatedUpdate = b"scorestats_accumulated_update" c"scorestats_accumulated_update",
		ScoutGrandSlam = b"scout_grand_slam" c"scout_grand_slam",

		/// Declared twice for some reason.
		ScoutSlamdollLanded = b"scout_slamdoll_landed" c"scout_slamdoll_landed",

		SentryOnGoActive = b"sentry_on_go_active" c"sentry_on_go_active",
		ShowAnnotation = b"show_annotation" c"show_annotation",
		ShowClassLayout = b"show_class_layout" c"show_class_layout",
		ShowFreezepanel = b"show_freezepanel" c"show_freezepanel",
		ShowMatchSummary = b"show_match_summary" c"show_match_summary",
		ShowVsPanel = b"show_vs_panel" c"show_vs_panel",
		SkeletonKilledQuest = b"skeleton_killed_quest" c"skeleton_killed_quest",
		SkeletonKingKilledQuest = b"skeleton_king_killed_quest" c"skeleton_king_killed_quest",

		/// > clone of player_death = "player_death",
		SlapNotice = b"slap_notice" c"slap_notice",

		SpecTargetUpdated = b"spec_target_updated" c"spec_target_updated",
		SpecialScore = b"special_score" c"special_score",
		SpyPdaReset = b"spy_pda_reset" c"spy_pda_reset",
		StatsResetround = b"stats_resetround" c"stats_resetround",
		StickyJump = b"sticky_jump" c"sticky_jump",
		StickyJumpLanded = b"sticky_jump_landed" c"sticky_jump_landed",
		StopWatchChanged = b"stop_watch_changed" c"stop_watch_changed",
		TaggedPlayerAsIt = b"tagged_player_as_it" c"tagged_player_as_it",
		TeamLeaderKilled = b"team_leader_killed" c"team_leader_killed",
		TeamplayAlert = b"teamplay_alert" c"teamplay_alert",
		TeamplayBroadcastAudio = b"teamplay_broadcast_audio" c"teamplay_broadcast_audio",
		TeamplayCaptureBlocked = b"teamplay_capture_blocked" c"teamplay_capture_blocked",
		TeamplayCaptureBroken = b"teamplay_capture_broken" c"teamplay_capture_broken",
		TeamplayFlagEvent = b"teamplay_flag_event" c"teamplay_flag_event",
		TeamplayGameOver = b"teamplay_game_over" c"teamplay_game_over",
		TeamplayMapTimeRemaining = b"teamplay_map_time_remaining" c"teamplay_map_time_remaining",
		TeamplayOvertimeBegin = b"teamplay_overtime_begin" c"teamplay_overtime_begin",
		TeamplayOvertimeEnd = b"teamplay_overtime_end" c"teamplay_overtime_end",
		TeamplayPointCaptured = b"teamplay_point_captured" c"teamplay_point_captured",
		TeamplayPointLocked = b"teamplay_point_locked" c"teamplay_point_locked",
		TeamplayPointStartcapture = b"teamplay_point_startcapture" c"teamplay_point_startcapture",
		TeamplayPointUnlocked = b"teamplay_point_unlocked" c"teamplay_point_unlocked",
		TeamplayPreRoundTimeLeft = b"teamplay_pre_round_time_left" c"teamplay_pre_round_time_left",
		TeamplayReadyRestart = b"teamplay_ready_restart" c"teamplay_ready_restart",
		TeamplayRestartRound = b"teamplay_restart_round" c"teamplay_restart_round",
		TeamplayRoundActive = b"teamplay_round_active" c"teamplay_round_active",
		TeamplayRoundRestartSeconds = b"teamplay_round_restart_seconds" c"teamplay_round_restart_seconds",
		TeamplayRoundSelected = b"teamplay_round_selected" c"teamplay_round_selected",
		TeamplayRoundStalemate = b"teamplay_round_stalemate" c"teamplay_round_stalemate",
		TeamplayRoundStart = b"teamplay_round_start" c"teamplay_round_start",
		TeamplayRoundWin = b"teamplay_round_win" c"teamplay_round_win",
		TeamplaySetupFinished = b"teamplay_setup_finished" c"teamplay_setup_finished",
		TeamplaySuddendeathBegin = b"teamplay_suddendeath_begin" c"teamplay_suddendeath_begin",
		TeamplaySuddendeathEnd = b"teamplay_suddendeath_end" c"teamplay_suddendeath_end",
		TeamplayTeamReady = b"teamplay_team_ready" c"teamplay_team_ready",
		TeamplayTeambalancedPlayer = b"teamplay_teambalanced_player" c"teamplay_teambalanced_player",
		TeamplayTimerFlash = b"teamplay_timer_flash" c"teamplay_timer_flash",
		TeamplayTimerTimeAdded = b"teamplay_timer_time_added" c"teamplay_timer_time_added",
		TeamplayUpdateTimer = b"teamplay_update_timer" c"teamplay_update_timer",
		TeamplayWaitingAbouttoend = b"teamplay_waiting_abouttoend" c"teamplay_waiting_abouttoend",
		TeamplayWaitingBegins = b"teamplay_waiting_begins" c"teamplay_waiting_begins",
		TeamplayWaitingEnds = b"teamplay_waiting_ends" c"teamplay_waiting_ends",
		TeamplayWinPanel = b"teamplay_win_panel" c"teamplay_win_panel",
		TeamsChanged = b"teams_changed" c"teams_changed",
		TfGameOver = b"tf_game_over" c"tf_game_over",
		TfMapTimeRemaining = b"tf_map_time_remaining" c"tf_map_time_remaining",

		/// > clone of "player_death" with added counts
		ThrowableHit = b"throwable_hit" c"throwable_hit",

		TopStreamsRequestFinished = b"top_streams_request_finished" c"top_streams_request_finished",
		TournamentEnablecountdown = b"tournament_enablecountdown" c"tournament_enablecountdown",
		TournamentStateupdate = b"tournament_stateupdate" c"tournament_stateupdate",
		TrainingComplete = b"training_complete" c"training_complete",
		UpdateStatusItem = b"update_status_item" c"update_status_item",
		UpgradesFileChanged = b"upgrades_file_changed" c"upgrades_file_changed",
		VoteMapsChanged = b"vote_maps_changed" c"vote_maps_changed",
		WinlimitChanged = b"winlimit_changed" c"winlimit_changed",
		WinpanelShowScores = b"winpanel_show_scores" c"winpanel_show_scores",
		WorldStatusChanged = b"world_status_changed" c"world_status_changed",
	}
}

impl GameEventId {
	pub const fn from_cstr(cstr: &CStr) -> Option<Self> {
		Self::from_bstr(cstr.to_bytes())
	}

	pub const fn from_str(str: &str) -> Option<Self> {
		Self::from_bstr(str.as_bytes())
	}

	#[doc(alias = "GetName")]
	pub const fn name(&self) -> &'static str {
		unsafe { str::from_utf8_unchecked(self.name_bstr()) }
	}
}

impl Display for GameEventId {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.name())
	}
}

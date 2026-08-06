//! Gesture & first-run chrome UI paths (spec 43).
//!
//! Locks the pure decision/mutation entry points that paint and pointer code
//! depend on — the residual gaps a live MCP audit cannot mouse-drive:
//! Esc drag cancel, 12px trim hit, snap capture, K-A10 fixed playhead / edge
//! pan, K-B5 compare toggle, social coach step machine, command registry.

use photonic_core::timeline::Tick;
use photonic_gui::app::engine::EngineBridge;
use photonic_gui::app::timeline::interact::{hit_zone, nearest_snap, should_cancel_drag, ClipZone};
use photonic_gui::app::timeline::layout::{
    TimelineView, EDGE_AUTO_PAN_ZONE_PX, EDGE_HIT_PX, EDGE_ZONE_PX,
};
use photonic_gui::commands;
use photonic_gui::preferences::{
    coach_advance_button, coach_auto_advance_on_clips, coach_skip_dismisses, AppPreferences,
};

// ── 2.1 Esc drag cancel ─────────────────────────────────────────────────────

#[test]
fn esc_cancels_only_when_a_drag_is_active() {
    assert!(!should_cancel_drag(false, false));
    assert!(!should_cancel_drag(true, false), "Esc alone is a no-op");
    assert!(
        !should_cancel_drag(false, true),
        "active drag without Esc stays"
    );
    assert!(
        should_cancel_drag(true, true),
        "Esc + active drag must cancel (no commit_drag)"
    );
}

// ── 2.2 Trim hit target (41 R-9 / 210) ───────────────────────────────────────

#[test]
fn trim_hit_uses_twelve_px_not_paint_six() {
    assert_eq!(EDGE_HIT_PX, 12.0);
    assert_eq!(EDGE_ZONE_PX, 6.0);
    // 10px inside the left edge: Body at paint width, LeftEdge at hit width.
    assert_eq!(
        hit_zone(200.0, 300.0, 210.0, EDGE_ZONE_PX),
        Some(ClipZone::Body)
    );
    assert_eq!(
        hit_zone(200.0, 300.0, 210.0, EDGE_HIT_PX),
        Some(ClipZone::LeftEdge)
    );
}

#[test]
fn widened_trim_handle_preserves_body_zone_for_short_clips() {
    for width in [13.0f32, 20.0, 24.0, 48.0] {
        let (x0, x1) = (0.0, width);
        let mid = width * 0.5;
        assert_eq!(
            hit_zone(x0, x1, mid, EDGE_HIT_PX),
            Some(ClipZone::Body),
            "{width}px clip must stay body-draggable"
        );
    }
}

// ── 2.3 Snap guide only when captured ───────────────────────────────────────

#[test]
fn snap_guide_none_outside_threshold() {
    let value = Tick(1000);
    let candidates = [Tick(1050), Tick(2000)];
    assert_eq!(
        nearest_snap(value, &candidates, Tick(5)),
        None,
        "outside threshold → no guide paint"
    );
    assert_eq!(
        nearest_snap(value, &candidates, Tick(60)),
        Some(Tick(1050)),
        "inside threshold → first priority candidate"
    );
}

// ── 2.4 Fixed playhead + edge pan (K-A10) ───────────────────────────────────

#[test]
fn fixed_playhead_toggles_as_view_state() {
    let mut v = TimelineView::default();
    assert!(!v.fixed_playhead);
    v.fixed_playhead = !v.fixed_playhead;
    assert!(v.fixed_playhead);
    v.fixed_playhead = !v.fixed_playhead;
    assert!(!v.fixed_playhead);
}

#[test]
fn center_on_playhead_and_edge_pan_zones() {
    let mut v = TimelineView::default();
    let ph = Tick::from_seconds(8);
    let lane_w = 500.0_f32;
    v.center_on_playhead(ph, lane_w);
    let x = v.tick_to_x(ph, 0.0);
    assert!(
        (x - lane_w / 2.0).abs() < 3.0,
        "playhead near centre, got x={x}"
    );

    let left = TimelineView::edge_auto_pan_speed(5.0, 0.0, 400.0);
    let mid = TimelineView::edge_auto_pan_speed(200.0, 0.0, 400.0);
    let right = TimelineView::edge_auto_pan_speed(395.0, 0.0, 400.0);
    assert!(left < 0.0);
    assert_eq!(mid, 0.0);
    assert!(right > 0.0);
    assert!(EDGE_AUTO_PAN_ZONE_PX > 0.0);
}

// ── 2.5 Compare effects (K-B5) ──────────────────────────────────────────────

#[test]
fn compare_effects_toggle_flips_bridge_flag() {
    let Some(engine) = photonic_video::VideoEngine::headless() else {
        eprintln!("no GPU adapter — skipping compare_effects toggle");
        return;
    };
    let mut bridge = EngineBridge::new(engine);
    assert!(!bridge.compare_effects());
    bridge.toggle_compare_effects();
    assert!(bridge.compare_effects());
    bridge.toggle_compare_effects();
    assert!(!bridge.compare_effects());
}

#[test]
fn compare_and_fixed_playhead_commands_are_registered() {
    let ids: Vec<&str> = commands::REGISTRY.iter().map(|c| c.id).collect();
    assert!(
        ids.contains(&"video.compare_effects"),
        "K-B5 command missing from REGISTRY"
    );
    assert!(
        ids.contains(&"video.toggle_fixed_playhead"),
        "K-A10 command missing from REGISTRY"
    );
}

// ── 2.6 Social coach (213) ──────────────────────────────────────────────────

#[test]
fn coach_defaults_favour_social_velocity() {
    let p = AppPreferences::default();
    assert!(!p.video_coach_dismissed);
    assert_eq!(p.video_coach_step, 0);
    assert!(p.auto_place_import_on_timeline);
}

#[test]
fn coach_step_machine_import_split_export() {
    assert_eq!(coach_auto_advance_on_clips(0, false), 0);
    assert_eq!(coach_auto_advance_on_clips(0, true), 1);
    assert_eq!(coach_auto_advance_on_clips(1, true), 1);

    assert_eq!(coach_advance_button(0), (1, false));
    assert_eq!(coach_advance_button(1), (2, false));
    assert_eq!(coach_advance_button(2), (2, true));
    assert!(coach_skip_dismisses());
}

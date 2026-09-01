use serde_json::json;

use super::*;

fn active_tab(snapshot: &AppSnapshot) -> &TabSnapshot {
    snapshot
        .session
        .tabs
        .iter()
        .find(|tab| tab.id == snapshot.session.active_tab_id)
        .expect("the active tab should exist")
}

fn pane_in_layout<'a>(root: &'a LayoutNode, pane_id: &str) -> Option<&'a PaneSnapshot> {
    match root {
        LayoutNode::Pane { pane } => (pane.id == pane_id).then_some(pane),
        LayoutNode::Split { first, second, .. } => {
            pane_in_layout(first, pane_id).or_else(|| pane_in_layout(second, pane_id))
        }
    }
}

fn pane<'a>(snapshot: &'a AppSnapshot, pane_id: &str) -> &'a PaneSnapshot {
    pane_in_layout(&active_tab(snapshot).root, pane_id).expect("the pane should exist")
}

fn assert_close(left: f64, right: f64) {
    assert!(
        (left - right).abs() < 1.0e-9,
        "expected {left} to be close to {right}"
    );
}

#[test]
fn default_model_has_one_session_tab_and_pane() {
    let model = AppModel::default();
    let snapshot = model.snapshot();

    assert_eq!(snapshot.schema_version, 1);
    assert_eq!(snapshot.revision, 0);
    assert_eq!(snapshot.active_session_id, snapshot.session.id);
    assert_eq!(snapshot.session.tabs.len(), 1);
    assert_eq!(model.pane_ids().len(), 1);
    assert_eq!(active_tab(&snapshot).active_pane_id, model.pane_ids()[0]);
}

#[test]
fn snapshots_follow_the_frozen_ipc_contract() {
    let mut model = AppModel::default();
    let snapshot = model
        .split_active(Direction::Right)
        .expect("split should succeed");
    let value = serde_json::to_value(snapshot).expect("snapshot should serialize");

    assert_eq!(value["schemaVersion"], json!(1));
    assert!(value.get("activeSessionId").is_some());
    assert!(value.get("session").is_some());
    assert!(value.get("sessions").is_none());
    let tab = &value["session"]["tabs"][0];
    assert!(tab.get("activePaneId").is_some());
    assert!(tab.get("zoomedPaneId").is_some());
    assert_eq!(tab["root"]["kind"], json!("split"));
    assert_eq!(tab["root"]["axis"], json!("row"));
    assert_eq!(tab["root"]["first"]["kind"], json!("pane"));
    assert!(tab["root"]["first"]["pane"].get("profileId").is_some());
}

#[test]
fn pane_status_values_match_the_ipc_contract() {
    for (status, expected) in [
        (PaneStatus::Starting, "starting"),
        (PaneStatus::Running, "running"),
        (PaneStatus::Exited, "exited"),
        (PaneStatus::Error, "error"),
    ] {
        assert_eq!(
            serde_json::to_value(status).expect("status should serialize"),
            json!(expected)
        );
    }
}

#[test]
fn split_places_and_focuses_new_pane_in_all_directions() {
    for direction in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        let mut model = AppModel::default();
        let original_pane_id = model.pane_ids()[0].clone();
        let snapshot = model.split_active(direction).expect("split should succeed");
        let new_pane_id = &active_tab(&snapshot).active_pane_id;
        let rectangles = model
            .active_tab()
            .expect("active tab should exist")
            .rectangles();
        let original = rectangles
            .get(&original_pane_id)
            .expect("original rectangle should exist");
        let new_pane = rectangles
            .get(new_pane_id)
            .expect("new rectangle should exist");

        assert_ne!(new_pane_id, &original_pane_id);
        assert_eq!(model.pane_ids().len(), 2);
        assert_eq!(snapshot.revision, 1);
        match direction {
            Direction::Left => assert!(new_pane.x < original.x),
            Direction::Right => assert!(new_pane.x > original.x),
            Direction::Up => assert!(new_pane.y < original.y),
            Direction::Down => assert!(new_pane.y > original.y),
        }
    }
}

#[test]
fn nested_layout_rectangles_fill_the_normalized_canvas() {
    let mut model = AppModel::default();
    let left_pane_id = model.pane_ids()[0].clone();
    model
        .split_active(Direction::Right)
        .expect("right split should succeed");
    let top_right_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();
    model
        .split_active(Direction::Down)
        .expect("down split should succeed");
    let bottom_right_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();
    let rectangles = model
        .active_tab()
        .expect("active tab should exist")
        .rectangles();

    assert_eq!(
        rectangles[&left_pane_id],
        NormalizedRect {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        }
    );
    assert_eq!(
        rectangles[&top_right_pane_id],
        NormalizedRect {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 0.5,
        }
    );
    assert_eq!(
        rectangles[&bottom_right_pane_id],
        NormalizedRect {
            x: 0.5,
            y: 0.5,
            width: 0.5,
            height: 0.5,
        }
    );
}

#[test]
fn geometric_focus_uses_final_rectangles() {
    let mut model = AppModel::default();
    let left_pane_id = model.pane_ids()[0].clone();
    model
        .split_active(Direction::Right)
        .expect("right split should succeed");
    let top_right_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();
    model
        .split_active(Direction::Down)
        .expect("down split should succeed");
    let bottom_right_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();

    let snapshot = model
        .focus_active(Direction::Up)
        .expect("focus up should succeed");
    assert_eq!(active_tab(&snapshot).active_pane_id, top_right_pane_id);

    let snapshot = model
        .focus_active(Direction::Left)
        .expect("focus left should succeed");
    assert_eq!(active_tab(&snapshot).active_pane_id, left_pane_id);

    let snapshot = model
        .focus_active(Direction::Right)
        .expect("focus right should succeed");
    let stable_tie_winner = top_right_pane_id.min(bottom_right_pane_id);
    assert_eq!(active_tab(&snapshot).active_pane_id, stable_tie_winner);
}

#[test]
fn focus_at_an_outer_edge_is_a_no_op() {
    let mut model = AppModel::default();
    let before = model.snapshot();
    let after = model
        .focus_active(Direction::Left)
        .expect("edge focus should not fail");

    assert_eq!(after, before);
}

#[test]
fn resize_uses_nearest_matching_ancestor_and_clamps_ratio() {
    let mut model = AppModel::default();
    model
        .split_active(Direction::Right)
        .expect("right split should succeed");
    let top_right_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();
    model
        .split_active(Direction::Down)
        .expect("down split should succeed");

    model
        .resize_active(Direction::Up, 0.2)
        .expect("vertical resize should succeed");
    let rectangles = model
        .active_tab()
        .expect("active tab should exist")
        .rectangles();
    assert_close(rectangles[&top_right_pane_id].height, 0.3);
    assert_close(rectangles[&top_right_pane_id].width, 0.5);

    model
        .resize_active(Direction::Up, 10.0)
        .expect("resize should clamp instead of failing");
    let rectangles = model
        .active_tab()
        .expect("active tab should exist")
        .rectangles();
    assert_close(rectangles[&top_right_pane_id].height, 0.1);
}

#[test]
fn resize_skips_an_inner_split_to_reach_the_nearest_matching_ancestor() {
    let mut model = AppModel::default();
    let left_pane_id = model.pane_ids()[0].clone();
    model
        .split_active(Direction::Right)
        .expect("right split should succeed");
    model
        .split_active(Direction::Down)
        .expect("down split should succeed");

    let before_revision = model.snapshot().revision;
    let snapshot = model
        .resize_active(Direction::Left, 0.2)
        .expect("ancestor resize should succeed");
    let rectangles = model
        .active_tab()
        .expect("active tab should exist")
        .rectangles();

    assert_close(rectangles[&left_pane_id].width, 0.3);
    assert_eq!(snapshot.revision, before_revision + 1);
}

#[test]
fn resize_at_an_outer_edge_is_a_no_op() {
    let mut model = AppModel::default();
    let before = model.snapshot();
    let after = model
        .resize_active(Direction::Left, 0.1)
        .expect("outer edge resize should not fail");

    assert_eq!(after, before);
}

#[test]
fn invalid_resize_amount_does_not_mutate_state() {
    let mut model = AppModel::default();
    let before = model.snapshot();

    assert_eq!(
        model.resize_active(Direction::Right, f64::NAN),
        Err(DomainError::InvalidResizeAmount)
    );
    assert_eq!(model.snapshot(), before);
}

#[test]
fn close_promotes_the_sibling_and_returns_removed_id() {
    let mut model = AppModel::default();
    let original_pane_id = model.pane_ids()[0].clone();
    model
        .split_active(Direction::Right)
        .expect("split should succeed");
    let removed_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();

    let result = model.close_active_pane().expect("close should succeed");

    assert_eq!(result.removed_pane_ids, vec![removed_pane_id]);
    assert_eq!(model.pane_ids(), vec![original_pane_id.clone()]);
    assert_eq!(
        active_tab(&result.snapshot).active_pane_id,
        original_pane_id
    );
    assert!(matches!(
        active_tab(&result.snapshot).root,
        LayoutNode::Pane { .. }
    ));
}

#[test]
fn close_last_pane_replaces_the_last_tab_atomically() {
    let mut model = AppModel::default();
    let removed_pane_id = model.pane_ids()[0].clone();
    let old_tab_id = active_tab(&model.snapshot()).id.clone();

    let result = model
        .close_active_pane()
        .expect("closing the last pane should create a default tab");
    let replacement_tab = active_tab(&result.snapshot);

    assert_eq!(result.removed_pane_ids, vec![removed_pane_id]);
    assert_ne!(replacement_tab.id, old_tab_id);
    assert_eq!(model.pane_ids().len(), 1);
    assert_eq!(result.snapshot.revision, 1);
}

#[test]
fn tabs_keep_independent_layout_and_active_pane() {
    let mut model = AppModel::default();
    let first_tab_id = active_tab(&model.snapshot()).id.clone();
    model
        .split_active(Direction::Right)
        .expect("split should succeed");
    let first_tab_active_pane_id = active_tab(&model.snapshot()).active_pane_id.clone();

    let snapshot = model.create_tab().expect("tab creation should succeed");
    let second_tab_id = active_tab(&snapshot).id.clone();
    assert_ne!(first_tab_id, second_tab_id);

    let snapshot = model
        .activate_tab(&first_tab_id)
        .expect("tab activation should succeed");
    assert_eq!(
        active_tab(&snapshot).active_pane_id,
        first_tab_active_pane_id
    );
    assert_eq!(active_tab(&snapshot).root.pane_count(), 2);
}

#[test]
fn closing_a_tab_returns_all_its_panes() {
    let mut model = AppModel::default();
    let first_tab_id = active_tab(&model.snapshot()).id.clone();
    model
        .split_active(Direction::Right)
        .expect("split should succeed");
    let removed_pane_ids = model.pane_ids();
    model.create_tab().expect("tab creation should succeed");

    let result = model
        .close_tab(&first_tab_id)
        .expect("close should succeed");

    assert_eq!(result.removed_pane_ids, removed_pane_ids);
    assert_eq!(result.snapshot.session.tabs.len(), 1);
}

#[test]
fn zoom_does_not_modify_layout_and_focus_exits_zoom() {
    let mut model = AppModel::default();
    model
        .split_active(Direction::Right)
        .expect("split should succeed");
    let before_zoom = active_tab(&model.snapshot()).root.clone();
    let zoomed = model.toggle_zoom().expect("zoom should succeed");
    assert!(active_tab(&zoomed).zoomed_pane_id.is_some());
    assert_eq!(active_tab(&zoomed).root, before_zoom);

    let focused = model
        .focus_active(Direction::Left)
        .expect("focus should succeed");
    assert!(active_tab(&focused).zoomed_pane_id.is_none());
    assert_eq!(active_tab(&focused).root, before_zoom);
}

#[test]
fn pane_status_and_titles_are_domain_mutations() {
    let mut model = AppModel::default();
    let pane_id = model.pane_ids()[0].clone();

    model
        .set_pane_status(&pane_id, PaneStatus::Error, Some("shell failed".to_owned()))
        .expect("status update should succeed");
    let snapshot = model
        .set_pane_title(&pane_id, "Build server")
        .expect("title update should succeed");
    let pane = pane(&snapshot, &pane_id);

    assert_eq!(pane.status, PaneStatus::Error);
    assert_eq!(pane.status_message.as_deref(), Some("shell failed"));
    assert_eq!(pane.title, "Build server");
    assert_eq!(snapshot.revision, 2);
}

#[test]
fn missing_targets_return_errors_without_mutating_state() {
    let mut model = AppModel::default();
    let before = model.snapshot();

    assert_eq!(
        model.activate_tab("missing-tab"),
        Err(DomainError::TabNotFound("missing-tab".to_owned()))
    );
    assert_eq!(
        model.set_pane_status("missing-pane", PaneStatus::Running, None),
        Err(DomainError::PaneNotFound("missing-pane".to_owned()))
    );
    assert_eq!(model.snapshot(), before);
}

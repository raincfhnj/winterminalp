use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_SPLIT_RATIO: f64 = 0.5;
const MIN_SPLIT_RATIO: f64 = 0.1;
const MAX_SPLIT_RATIO: f64 = 1.0 - MIN_SPLIT_RATIO;
const GEOMETRY_EPSILON: f64 = 1.0e-9;
const DEFAULT_PROFILE_ID: &str = "powershell";

/// A user-facing direction for pane creation, focus, and resizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn split_axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Row,
            Self::Up | Self::Down => Axis::Column,
        }
    }
}

/// The visual axis of a split.
///
/// `Row` places children left-to-right. `Column` places children top-to-bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Axis {
    Row,
    Column,
}

/// The lifecycle state exposed for a pane's terminal process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PaneStatus {
    Starting,
    Running,
    Exited,
    Error,
}

/// A pane rectangle normalized to the `[0, 1]` layout canvas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }

    fn center_x(self) -> f64 {
        self.x + self.width / 2.0
    }

    fn center_y(self) -> f64 {
        self.y + self.height / 2.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PaneSnapshot {
    pub id: String,
    pub title: String,
    pub profile_id: String,
    pub status: PaneStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

impl PaneSnapshot {
    fn new(id: String) -> Self {
        Self {
            id,
            title: "PowerShell".to_owned(),
            profile_id: DEFAULT_PROFILE_ID.to_owned(),
            status: PaneStatus::Starting,
            status_message: None,
        }
    }
}

/// The serialized binary split tree used by the renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum LayoutNode {
    Pane {
        pane: PaneSnapshot,
    },
    Split {
        axis: Axis,
        ratio: f64,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    fn pane(pane_id: String) -> Self {
        Self::Pane {
            pane: PaneSnapshot::new(pane_id),
        }
    }

    fn contains_pane(&self, target_pane_id: &str) -> bool {
        match self {
            Self::Pane { pane } => pane.id == target_pane_id,
            Self::Split { first, second, .. } => {
                first.contains_pane(target_pane_id) || second.contains_pane(target_pane_id)
            }
        }
    }

    fn pane_count(&self) -> usize {
        match self {
            Self::Pane { .. } => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    fn collect_pane_ids<'a>(&'a self, pane_ids: &mut Vec<&'a str>) {
        match self {
            Self::Pane { pane } => pane_ids.push(&pane.id),
            Self::Split { first, second, .. } => {
                first.collect_pane_ids(pane_ids);
                second.collect_pane_ids(pane_ids);
            }
        }
    }

    fn collect_rectangles(
        &self,
        rectangle: NormalizedRect,
        rectangles: &mut HashMap<String, NormalizedRect>,
    ) {
        match self {
            Self::Pane { pane } => {
                rectangles.insert(pane.id.clone(), rectangle);
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match axis {
                Axis::Row => {
                    let first_width = rectangle.width * ratio;
                    first.collect_rectangles(
                        NormalizedRect {
                            width: first_width,
                            ..rectangle
                        },
                        rectangles,
                    );
                    second.collect_rectangles(
                        NormalizedRect {
                            x: rectangle.x + first_width,
                            width: rectangle.width - first_width,
                            ..rectangle
                        },
                        rectangles,
                    );
                }
                Axis::Column => {
                    let first_height = rectangle.height * ratio;
                    first.collect_rectangles(
                        NormalizedRect {
                            height: first_height,
                            ..rectangle
                        },
                        rectangles,
                    );
                    second.collect_rectangles(
                        NormalizedRect {
                            y: rectangle.y + first_height,
                            height: rectangle.height - first_height,
                            ..rectangle
                        },
                        rectangles,
                    );
                }
            },
        }
    }

    fn split_pane(
        &mut self,
        target_pane_id: &str,
        new_pane_id: String,
        direction: Direction,
    ) -> bool {
        match self {
            Self::Pane { pane } if pane.id == target_pane_id => {
                let current = Self::Pane { pane: pane.clone() };
                let new_pane = Self::pane(new_pane_id);
                let (first, second) = match direction {
                    Direction::Left | Direction::Up => (new_pane, current),
                    Direction::Right | Direction::Down => (current, new_pane),
                };

                *self = Self::Split {
                    axis: direction.split_axis(),
                    ratio: DEFAULT_SPLIT_RATIO,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Pane { .. } => false,
            Self::Split { first, second, .. } => {
                first.split_pane(target_pane_id, new_pane_id.clone(), direction)
                    || second.split_pane(target_pane_id, new_pane_id, direction)
            }
        }
    }

    fn remove_pane(self, target_pane_id: &str) -> RemovePaneOutcome {
        match self {
            Self::Pane { pane } => {
                if pane.id == target_pane_id {
                    RemovePaneOutcome::Removed
                } else {
                    RemovePaneOutcome::NotFound(Self::Pane { pane })
                }
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                if first.contains_pane(target_pane_id) {
                    match (*first).remove_pane(target_pane_id) {
                        RemovePaneOutcome::Removed => RemovePaneOutcome::Updated(*second),
                        RemovePaneOutcome::Updated(updated_first) => {
                            RemovePaneOutcome::Updated(Self::Split {
                                axis,
                                ratio,
                                first: Box::new(updated_first),
                                second,
                            })
                        }
                        RemovePaneOutcome::NotFound(updated_first) => {
                            RemovePaneOutcome::NotFound(Self::Split {
                                axis,
                                ratio,
                                first: Box::new(updated_first),
                                second,
                            })
                        }
                    }
                } else if second.contains_pane(target_pane_id) {
                    match (*second).remove_pane(target_pane_id) {
                        RemovePaneOutcome::Removed => RemovePaneOutcome::Updated(*first),
                        RemovePaneOutcome::Updated(updated_second) => {
                            RemovePaneOutcome::Updated(Self::Split {
                                axis,
                                ratio,
                                first,
                                second: Box::new(updated_second),
                            })
                        }
                        RemovePaneOutcome::NotFound(updated_second) => {
                            RemovePaneOutcome::NotFound(Self::Split {
                                axis,
                                ratio,
                                first,
                                second: Box::new(updated_second),
                            })
                        }
                    }
                } else {
                    RemovePaneOutcome::NotFound(Self::Split {
                        axis,
                        ratio,
                        first,
                        second,
                    })
                }
            }
        }
    }

    fn resize_nearest(
        &mut self,
        target_pane_id: &str,
        direction: Direction,
        amount: f64,
    ) -> ResizeOutcome {
        let Self::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return ResizeOutcome::NoBoundary;
        };

        let target_in_first = first.contains_pane(target_pane_id);
        let target_in_second = second.contains_pane(target_pane_id);
        let nested_outcome = if target_in_first {
            first.resize_nearest(target_pane_id, direction, amount)
        } else if target_in_second {
            second.resize_nearest(target_pane_id, direction, amount)
        } else {
            return ResizeOutcome::PaneNotFound;
        };

        if nested_outcome != ResizeOutcome::NoBoundary {
            return nested_outcome;
        }

        let ratio_delta = match (axis, direction, target_in_first, target_in_second) {
            (Axis::Row, Direction::Right, true, _) => Some(amount),
            (Axis::Row, Direction::Left, _, true) => Some(-amount),
            (Axis::Column, Direction::Down, true, _) => Some(amount),
            (Axis::Column, Direction::Up, _, true) => Some(-amount),
            _ => None,
        };
        let Some(ratio_delta) = ratio_delta else {
            return ResizeOutcome::NoBoundary;
        };

        let next_ratio = (*ratio + ratio_delta).clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        if (next_ratio - *ratio).abs() <= GEOMETRY_EPSILON {
            ResizeOutcome::AtLimit
        } else {
            *ratio = next_ratio;
            ResizeOutcome::Resized
        }
    }

    fn pane_mut(&mut self, pane_id: &str) -> Option<&mut PaneSnapshot> {
        match self {
            Self::Pane { pane } => (pane.id == pane_id).then_some(pane),
            Self::Split { first, second, .. } => {
                first.pane_mut(pane_id).or_else(|| second.pane_mut(pane_id))
            }
        }
    }
}

enum RemovePaneOutcome {
    Removed,
    Updated(LayoutNode),
    NotFound(LayoutNode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeOutcome {
    Resized,
    AtLimit,
    NoBoundary,
    PaneNotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TabSnapshot {
    pub id: String,
    pub title: String,
    pub active_pane_id: String,
    pub zoomed_pane_id: Option<String>,
    pub root: LayoutNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SessionSnapshot {
    pub id: String,
    pub name: String,
    pub active_tab_id: String,
    pub tabs: Vec<TabSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct AppSnapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub active_session_id: String,
    pub session: SessionSnapshot,
}

/// The result of a destructive layout mutation.
///
/// Terminal resources named by `removed_pane_ids` can be reclaimed only after
/// the domain mutation succeeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[must_use]
#[non_exhaustive]
pub struct MutationResult {
    pub snapshot: AppSnapshot,
    pub removed_pane_ids: Vec<String>,
}

#[derive(Debug, Error, Clone, PartialEq)]
#[non_exhaustive]
pub enum DomainError {
    #[error("the active session does not exist")]
    ActiveSessionNotFound,
    #[error("the active tab does not exist")]
    ActiveTabNotFound,
    #[error("tab `{0}` does not exist")]
    TabNotFound(String),
    #[error("pane `{0}` does not exist")]
    PaneNotFound(String),
    #[error("resize amount must be a finite number greater than zero")]
    InvalidResizeAmount,
}

#[derive(Debug, Clone)]
struct TabModel {
    id: String,
    title: String,
    active_pane_id: String,
    zoomed_pane_id: Option<String>,
    root: LayoutNode,
}

impl TabModel {
    fn new(id: String, title: String, pane_id: String) -> Self {
        Self {
            id,
            title,
            active_pane_id: pane_id.clone(),
            zoomed_pane_id: None,
            root: LayoutNode::pane(pane_id),
        }
    }

    fn rectangles(&self) -> HashMap<String, NormalizedRect> {
        let mut rectangles = HashMap::with_capacity(self.root.pane_count());
        self.root
            .collect_rectangles(NormalizedRect::FULL, &mut rectangles);
        rectangles
    }

    fn pane_ids(&self) -> Vec<String> {
        let mut pane_ids = Vec::with_capacity(self.root.pane_count());
        self.root.collect_pane_ids(&mut pane_ids);
        pane_ids.into_iter().map(ToOwned::to_owned).collect()
    }

    fn snapshot(&self) -> TabSnapshot {
        TabSnapshot {
            id: self.id.clone(),
            title: self.title.clone(),
            active_pane_id: self.active_pane_id.clone(),
            zoomed_pane_id: self.zoomed_pane_id.clone(),
            root: self.root.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct SessionModel {
    id: String,
    name: String,
    active_tab_id: String,
    tabs: Vec<TabModel>,
}

impl SessionModel {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            id: self.id.clone(),
            name: self.name.clone(),
            active_tab_id: self.active_tab_id.clone(),
            tabs: self.tabs.iter().map(TabModel::snapshot).collect(),
        }
    }
}

/// The authoritative in-memory layout state for the desktop application.
#[derive(Debug, Clone)]
pub struct AppModel {
    revision: u64,
    active_session_id: String,
    session: SessionModel,
    next_tab_number: u64,
}

impl Default for AppModel {
    fn default() -> Self {
        let session_id = new_id("session");
        let tab_id = new_id("tab");
        let pane_id = new_id("pane");
        let tab = TabModel::new(tab_id.clone(), "Terminal 1".to_owned(), pane_id);
        let session = SessionModel {
            id: session_id.clone(),
            name: "Default".to_owned(),
            active_tab_id: tab_id,
            tabs: vec![tab],
        };

        Self {
            revision: 0,
            active_session_id: session_id,
            session,
            next_tab_number: 2,
        }
    }
}

impl AppModel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            schema_version: SCHEMA_VERSION,
            revision: self.revision,
            active_session_id: self.active_session_id.clone(),
            session: self.session.snapshot(),
        }
    }

    pub fn split_active(&mut self, direction: Direction) -> Result<AppSnapshot, DomainError> {
        let new_pane_id = new_id("pane");
        let tab = self.active_tab_mut()?;
        let active_pane_id = tab.active_pane_id.clone();
        if !tab
            .root
            .split_pane(&active_pane_id, new_pane_id.clone(), direction)
        {
            return Err(DomainError::PaneNotFound(active_pane_id));
        }
        tab.active_pane_id = new_pane_id;
        tab.zoomed_pane_id = None;
        self.bump_revision();
        Ok(self.snapshot())
    }

    pub fn focus_active(&mut self, direction: Direction) -> Result<AppSnapshot, DomainError> {
        let (target_pane_id, was_zoomed) = {
            let tab = self.active_tab()?;
            (
                focus_target(&tab.rectangles(), &tab.active_pane_id, direction),
                tab.zoomed_pane_id.is_some(),
            )
        };

        if was_zoomed || target_pane_id.is_some() {
            let tab = self.active_tab_mut()?;
            tab.zoomed_pane_id = None;
            if let Some(target_pane_id) = target_pane_id {
                tab.active_pane_id = target_pane_id;
            }
            self.bump_revision();
        }
        Ok(self.snapshot())
    }

    pub fn resize_active(
        &mut self,
        direction: Direction,
        amount: f64,
    ) -> Result<AppSnapshot, DomainError> {
        if !amount.is_finite() || amount <= 0.0 {
            return Err(DomainError::InvalidResizeAmount);
        }

        let (outcome, was_zoomed) = {
            let tab = self.active_tab_mut()?;
            let active_pane_id = tab.active_pane_id.clone();
            let was_zoomed = tab.zoomed_pane_id.take().is_some();
            (
                tab.root.resize_nearest(&active_pane_id, direction, amount),
                was_zoomed,
            )
        };

        match outcome {
            ResizeOutcome::PaneNotFound => {
                let pane_id = self.active_tab()?.active_pane_id.clone();
                Err(DomainError::PaneNotFound(pane_id))
            }
            ResizeOutcome::Resized => {
                self.bump_revision();
                Ok(self.snapshot())
            }
            ResizeOutcome::AtLimit | ResizeOutcome::NoBoundary if was_zoomed => {
                self.bump_revision();
                Ok(self.snapshot())
            }
            ResizeOutcome::AtLimit | ResizeOutcome::NoBoundary => Ok(self.snapshot()),
        }
    }

    pub fn toggle_zoom(&mut self) -> Result<AppSnapshot, DomainError> {
        let tab = self.active_tab_mut()?;
        tab.zoomed_pane_id = match &tab.zoomed_pane_id {
            Some(pane_id) if pane_id == &tab.active_pane_id => None,
            _ => Some(tab.active_pane_id.clone()),
        };
        self.bump_revision();
        Ok(self.snapshot())
    }

    pub fn create_tab(&mut self) -> Result<AppSnapshot, DomainError> {
        self.ensure_active_session()?;
        let tab_id = new_id("tab");
        let pane_id = new_id("pane");
        let tab = TabModel::new(tab_id.clone(), self.next_tab_title(), pane_id);
        self.session.tabs.push(tab);
        self.session.active_tab_id = tab_id;
        self.bump_revision();
        Ok(self.snapshot())
    }

    pub fn activate_tab(&mut self, tab_id: &str) -> Result<AppSnapshot, DomainError> {
        self.ensure_active_session()?;
        if self.session.active_tab_id == tab_id {
            return Ok(self.snapshot());
        }
        if !self.session.tabs.iter().any(|tab| tab.id == tab_id) {
            return Err(DomainError::TabNotFound(tab_id.to_owned()));
        }

        self.session.active_tab_id = tab_id.to_owned();
        self.bump_revision();
        Ok(self.snapshot())
    }

    pub fn close_active_pane(&mut self) -> Result<MutationResult, DomainError> {
        let (tab_id, pane_id, pane_count) = {
            let tab = self.active_tab()?;
            (
                tab.id.clone(),
                tab.active_pane_id.clone(),
                tab.root.pane_count(),
            )
        };

        if pane_count == 1 {
            return self.close_tab(&tab_id);
        }

        {
            let tab = self.active_tab_mut()?;
            let next_active_pane_id = closest_pane_after_close(&tab.rectangles(), &pane_id)
                .ok_or_else(|| DomainError::PaneNotFound(pane_id.clone()))?;
            let root = tab.root.clone();
            tab.root = match root.remove_pane(&pane_id) {
                RemovePaneOutcome::Updated(root) => root,
                RemovePaneOutcome::Removed | RemovePaneOutcome::NotFound(_) => {
                    return Err(DomainError::PaneNotFound(pane_id));
                }
            };
            tab.active_pane_id = next_active_pane_id;
            tab.zoomed_pane_id = None;
        }

        self.bump_revision();
        Ok(MutationResult {
            snapshot: self.snapshot(),
            removed_pane_ids: vec![pane_id],
        })
    }

    pub fn close_tab(&mut self, tab_id: &str) -> Result<MutationResult, DomainError> {
        self.ensure_active_session()?;
        let tab_index = self
            .session
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or_else(|| DomainError::TabNotFound(tab_id.to_owned()))?;
        let removed_pane_ids = self.session.tabs[tab_index].pane_ids();
        let was_active = self.session.active_tab_id == tab_id;

        if self.session.tabs.len() == 1 {
            let replacement_tab_id = new_id("tab");
            let replacement_pane_id = new_id("pane");
            let replacement = TabModel::new(
                replacement_tab_id.clone(),
                self.next_tab_title(),
                replacement_pane_id,
            );
            self.session.tabs[0] = replacement;
            self.session.active_tab_id = replacement_tab_id;
        } else {
            self.session.tabs.remove(tab_index);
            if was_active {
                let next_index = tab_index.min(self.session.tabs.len().saturating_sub(1));
                let next_tab = self
                    .session
                    .tabs
                    .get(next_index)
                    .ok_or(DomainError::ActiveTabNotFound)?;
                self.session.active_tab_id = next_tab.id.clone();
            }
        }

        self.bump_revision();
        Ok(MutationResult {
            snapshot: self.snapshot(),
            removed_pane_ids,
        })
    }

    #[must_use]
    pub fn pane_ids(&self) -> Vec<String> {
        self.session
            .tabs
            .iter()
            .flat_map(TabModel::pane_ids)
            .collect()
    }

    pub fn set_pane_status(
        &mut self,
        pane_id: &str,
        status: PaneStatus,
        status_message: Option<String>,
    ) -> Result<AppSnapshot, DomainError> {
        let pane = self.pane_mut(pane_id)?;
        if pane.status == status && pane.status_message == status_message {
            return Ok(self.snapshot());
        }
        pane.status = status;
        pane.status_message = status_message;
        self.bump_revision();
        Ok(self.snapshot())
    }

    pub fn set_pane_title(
        &mut self,
        pane_id: &str,
        title: impl Into<String>,
    ) -> Result<AppSnapshot, DomainError> {
        let title = title.into();
        let pane = self.pane_mut(pane_id)?;
        if pane.title == title {
            return Ok(self.snapshot());
        }
        pane.title = title;
        self.bump_revision();
        Ok(self.snapshot())
    }

    pub fn set_tab_title(
        &mut self,
        tab_id: &str,
        title: impl Into<String>,
    ) -> Result<AppSnapshot, DomainError> {
        self.ensure_active_session()?;
        let title = title.into();
        let tab = self
            .session
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .ok_or_else(|| DomainError::TabNotFound(tab_id.to_owned()))?;
        if tab.title == title {
            return Ok(self.snapshot());
        }
        tab.title = title;
        self.bump_revision();
        Ok(self.snapshot())
    }

    fn ensure_active_session(&self) -> Result<(), DomainError> {
        if self.session.id == self.active_session_id {
            Ok(())
        } else {
            Err(DomainError::ActiveSessionNotFound)
        }
    }

    fn active_tab(&self) -> Result<&TabModel, DomainError> {
        self.ensure_active_session()?;
        self.session
            .tabs
            .iter()
            .find(|tab| tab.id == self.session.active_tab_id)
            .ok_or(DomainError::ActiveTabNotFound)
    }

    fn active_tab_mut(&mut self) -> Result<&mut TabModel, DomainError> {
        self.ensure_active_session()?;
        let active_tab_id = &self.session.active_tab_id;
        self.session
            .tabs
            .iter_mut()
            .find(|tab| tab.id == active_tab_id.as_str())
            .ok_or(DomainError::ActiveTabNotFound)
    }

    fn pane_mut(&mut self, pane_id: &str) -> Result<&mut PaneSnapshot, DomainError> {
        self.session
            .tabs
            .iter_mut()
            .find_map(|tab| tab.root.pane_mut(pane_id))
            .ok_or_else(|| DomainError::PaneNotFound(pane_id.to_owned()))
    }

    fn next_tab_title(&mut self) -> String {
        let number = self.next_tab_number;
        self.next_tab_number = self.next_tab_number.saturating_add(1);
        format!("Terminal {number}")
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

fn new_id(kind: &str) -> String {
    format!("{kind}-{}", Uuid::new_v4().simple())
}

#[derive(Debug)]
struct FocusCandidate<'a> {
    pane_id: &'a str,
    edge_distance: f64,
    overlap: f64,
    center_distance: f64,
}

impl FocusCandidate<'_> {
    fn compare(&self, other: &Self) -> Ordering {
        self.edge_distance
            .total_cmp(&other.edge_distance)
            .then_with(|| other.overlap.total_cmp(&self.overlap))
            .then_with(|| self.center_distance.total_cmp(&other.center_distance))
            .then_with(|| self.pane_id.cmp(other.pane_id))
    }
}

fn focus_target(
    rectangles: &HashMap<String, NormalizedRect>,
    active_pane_id: &str,
    direction: Direction,
) -> Option<String> {
    let active = rectangles.get(active_pane_id)?;
    rectangles
        .iter()
        .filter(|(pane_id, _)| pane_id.as_str() != active_pane_id)
        .filter_map(|(pane_id, candidate)| focus_candidate(active, candidate, direction, pane_id))
        .min_by(|left, right| left.compare(right))
        .map(|candidate| candidate.pane_id.to_owned())
}

fn focus_candidate<'a>(
    active: &NormalizedRect,
    candidate: &NormalizedRect,
    direction: Direction,
    pane_id: &'a str,
) -> Option<FocusCandidate<'a>> {
    let (in_half_plane, edge_distance, overlap, center_distance) = match direction {
        Direction::Left => (
            candidate.center_x() < active.center_x() - GEOMETRY_EPSILON,
            (active.x - candidate.right()).max(0.0),
            projection_overlap(active.y, active.bottom(), candidate.y, candidate.bottom()),
            (active.center_y() - candidate.center_y()).abs(),
        ),
        Direction::Right => (
            candidate.center_x() > active.center_x() + GEOMETRY_EPSILON,
            (candidate.x - active.right()).max(0.0),
            projection_overlap(active.y, active.bottom(), candidate.y, candidate.bottom()),
            (active.center_y() - candidate.center_y()).abs(),
        ),
        Direction::Up => (
            candidate.center_y() < active.center_y() - GEOMETRY_EPSILON,
            (active.y - candidate.bottom()).max(0.0),
            projection_overlap(active.x, active.right(), candidate.x, candidate.right()),
            (active.center_x() - candidate.center_x()).abs(),
        ),
        Direction::Down => (
            candidate.center_y() > active.center_y() + GEOMETRY_EPSILON,
            (candidate.y - active.bottom()).max(0.0),
            projection_overlap(active.x, active.right(), candidate.x, candidate.right()),
            (active.center_x() - candidate.center_x()).abs(),
        ),
    };

    in_half_plane.then_some(FocusCandidate {
        pane_id,
        edge_distance,
        overlap,
        center_distance,
    })
}

fn projection_overlap(first_start: f64, first_end: f64, second_start: f64, second_end: f64) -> f64 {
    (first_end.min(second_end) - first_start.max(second_start)).max(0.0)
}

fn closest_pane_after_close(
    rectangles: &HashMap<String, NormalizedRect>,
    closing_pane_id: &str,
) -> Option<String> {
    let closing = rectangles.get(closing_pane_id)?;
    rectangles
        .iter()
        .filter(|(pane_id, _)| pane_id.as_str() != closing_pane_id)
        .min_by(|(left_id, left), (right_id, right)| {
            rectangle_edge_distance_squared(closing, left)
                .total_cmp(&rectangle_edge_distance_squared(closing, right))
                .then_with(|| {
                    rectangle_center_distance_squared(closing, left)
                        .total_cmp(&rectangle_center_distance_squared(closing, right))
                })
                .then_with(|| left_id.cmp(right_id))
        })
        .map(|(pane_id, _)| pane_id.clone())
}

fn rectangle_edge_distance_squared(first: &NormalizedRect, second: &NormalizedRect) -> f64 {
    let horizontal_gap = if first.right() < second.x {
        second.x - first.right()
    } else if second.right() < first.x {
        first.x - second.right()
    } else {
        0.0
    };
    let vertical_gap = if first.bottom() < second.y {
        second.y - first.bottom()
    } else if second.bottom() < first.y {
        first.y - second.bottom()
    } else {
        0.0
    };
    horizontal_gap.powi(2) + vertical_gap.powi(2)
}

fn rectangle_center_distance_squared(first: &NormalizedRect, second: &NormalizedRect) -> f64 {
    (first.center_x() - second.center_x()).powi(2) + (first.center_y() - second.center_y()).powi(2)
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

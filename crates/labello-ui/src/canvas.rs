use std::{convert::Infallible, f32::consts::PI};

use egui::{
    Color32, CornerRadius, Key, Mesh, PointerButton, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationVersion, BoundingBox, NormalizedPoint,
    PanDragModifier, PrelabelSuggestion,
};

use crate::theme;

const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 12.0;
const ZOOM_STEP: f32 = 1.25;
const MIN_BOX_SIZE: f32 = 0.001;
const HANDLE_HIT_RADIUS: f32 = 12.0;
const HANDLE_SIZE: f32 = 8.0;
const VIEWPORT_CORNER_RADIUS: u8 = 18;
const CORNER_MASK_SEGMENTS: u32 = 8;
const FOCUS_MARGIN: f32 = 1.35;
const MIN_FOCUS_SPAN: f32 = 0.04;
const DOUBLE_CLICK_DELAY: f64 = 0.3;
const DOUBLE_CLICK_DISTANCE: f32 = 6.0;
const MAX_DASH_SEGMENTS: usize = 10_000;

/// The result of moving or resizing an existing bounding-box annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundingBoxEdit {
    pub annotation_id: AnnotationId,
    pub bounding_box: BoundingBox,
}

/// The result of moving an existing skeleton keypoint.
#[derive(Clone, Debug, PartialEq)]
pub struct KeypointEdit {
    pub annotation_id: AnnotationId,
    pub keypoint_index: usize,
    pub point: NormalizedPoint,
}

/// A selected keypoint on an existing skeleton annotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeypointSelection {
    pub annotation_id: AnnotationId,
    pub keypoint_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CanvasAnnotationStyle {
    pub color: Color32,
    pub dashed_box: bool,
}

impl CanvasAnnotationStyle {
    pub(crate) const fn solid(color: Color32) -> Self {
        Self {
            color,
            dashed_box: false,
        }
    }

    pub(crate) const fn dashed(color: Color32) -> Self {
        Self {
            color,
            dashed_box: true,
        }
    }
}

/// An action produced by the canvas.
///
/// `Edit` defaults to [`Infallible`] so existing exhaustive matches over actions
/// returned by [`show_canvas`] remain source-compatible. The interactive API
/// returns `CanvasAction<BoundingBoxEdit>` and can produce `EditBoundingBox`.
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasAction<Edit = Infallible> {
    CreateBoundingBox(BoundingBox),
    PlaceKeypoint(NormalizedPoint),
    Select(AnnotationId),
    EditBoundingBox(Edit),
    SelectKeypoint(KeypointSelection),
    EditKeypoint(KeypointEdit),
}

#[derive(Clone, Copy, Debug)]
pub struct CanvasInteraction {
    pub editable: bool,
    pub allow_create: bool,
    pub allow_selection: bool,
    pub edit_keypoints: bool,
    pub selected_keypoint: Option<usize>,
}

impl CanvasInteraction {
    pub fn annotations(editable: bool) -> Self {
        Self {
            editable,
            allow_create: editable,
            allow_selection: editable,
            edit_keypoints: false,
            selected_keypoint: None,
        }
    }

    pub fn correction(selected_keypoint: Option<usize>) -> Self {
        Self {
            editable: true,
            allow_create: false,
            allow_selection: false,
            edit_keypoints: true,
            selected_keypoint,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CanvasState {
    drag: Option<DragOperation>,
    draft_box: Option<BoundingBox>,
    draft_keypoint: Option<NormalizedPoint>,
    zoom: f32,
    pan: Vec2,
    modifier_pan: bool,
    pan_drag_modifier: PanDragModifier,
    pan_mode: bool,
    pan_mode_required: bool,
    primary_pan: bool,
    last_canvas_click: Option<(f64, Pos2)>,
    review_target: ReviewViewTarget,
    pending_review_view: Option<Option<Rect>>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            drag: None,
            draft_box: None,
            draft_keypoint: None,
            zoom: MIN_ZOOM,
            pan: Vec2::ZERO,
            modifier_pan: false,
            pan_drag_modifier: PanDragModifier::default(),
            pan_mode: false,
            pan_mode_required: false,
            primary_pan: false,
            last_canvas_click: None,
            review_target: ReviewViewTarget::Disabled,
            pending_review_view: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ReviewViewTarget {
    #[default]
    Disabled,
    FullImage,
    Annotation(AnnotationId, u32),
}

impl CanvasState {
    /// Whether an annotation create, move, or resize interaction is active.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn cancel_drag(&mut self) {
        self.drag = None;
        self.draft_box = None;
        self.draft_keypoint = None;
    }

    /// Return the current zoom factor relative to the aspect-fitted image.
    pub fn current_zoom(&self) -> f32 {
        self.zoom
    }

    pub fn pan_mode(&self) -> bool {
        self.pan_mode_required || self.pan_mode
    }

    pub(crate) fn pan_mode_required(&self) -> bool {
        self.pan_mode_required
    }

    pub fn can_pan(&self) -> bool {
        self.zoom > MIN_ZOOM
    }

    pub fn can_zoom_in(&self) -> bool {
        self.zoom < MAX_ZOOM
    }

    pub fn can_zoom_out(&self) -> bool {
        self.zoom > MIN_ZOOM
    }

    pub fn set_pan_drag_modifier(&mut self, modifier: PanDragModifier) {
        self.pan_drag_modifier = modifier;
    }

    pub fn toggle_pan_mode(&mut self) {
        if self.pan_mode_required {
            return;
        }
        self.cancel_drag();
        self.primary_pan = false;
        self.pan_mode = self.zoom > MIN_ZOOM && !self.pan_mode;
    }

    pub fn exit_pan_mode(&mut self) {
        if self.pan_mode_required {
            return;
        }
        self.primary_pan = false;
        self.pan_mode = false;
    }

    pub(crate) fn require_pan_mode(&mut self, required: bool) {
        if self.pan_mode_required == required {
            return;
        }
        self.cancel_drag();
        self.modifier_pan = false;
        self.primary_pan = false;
        self.pan_mode = false;
        self.pan_mode_required = required;
    }

    pub(crate) fn stored_transform(&self) -> crate::persistence::StoredCanvasTransform {
        crate::persistence::StoredCanvasTransform {
            zoom: self.zoom,
            pan_x: self.pan.x,
            pan_y: self.pan.y,
        }
        .clamped()
    }

    pub(crate) fn restore_transform(
        &mut self,
        transform: crate::persistence::StoredCanvasTransform,
    ) {
        let transform = transform.clamped();
        self.cancel_drag();
        self.pan_mode = false;
        self.primary_pan = false;
        self.zoom = transform.zoom;
        self.pan = vec2(transform.pan_x, transform.pan_y);
    }

    /// Zoom in one step around the center of the viewport.
    pub fn zoom_in(&mut self) {
        self.cancel_drag();
        let old_zoom = self.zoom;
        self.zoom = (old_zoom * ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= self.zoom / old_zoom;
    }

    /// Zoom out one step around the center of the viewport.
    pub fn zoom_out(&mut self) {
        self.cancel_drag();
        let old_zoom = self.zoom;
        self.zoom = (old_zoom / ZOOM_STEP).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= self.zoom / old_zoom;
        if self.zoom == MIN_ZOOM {
            self.pan = Vec2::ZERO;
            self.pan_mode = false;
            self.primary_pan = false;
        }
    }

    /// Fit the image to the canvas and center it.
    pub fn fit_view(&mut self) {
        self.cancel_drag();
        self.zoom = MIN_ZOOM;
        self.pan = Vec2::ZERO;
        self.pan_mode = false;
        self.primary_pan = false;
    }

    /// Focus a review object once when it becomes active, or fit the full image
    /// when object-by-object review is complete.
    pub fn set_review_focus(&mut self, annotation: Option<&AnnotationVersion>) {
        let target = annotation.map_or(ReviewViewTarget::FullImage, |annotation| {
            ReviewViewTarget::Annotation(annotation.annotation_id.clone(), annotation.version)
        });
        if self.review_target == target {
            return;
        }
        self.review_target = target;
        self.pending_review_view = Some(annotation.and_then(annotation_focus_rect));
    }

    pub(crate) fn focus_annotation(&mut self, annotation: &AnnotationVersion) {
        self.review_target =
            ReviewViewTarget::Annotation(annotation.annotation_id.clone(), annotation.version);
        self.pending_review_view = Some(annotation_focus_rect(annotation));
    }

    /// Stop tracking review targets without changing the current view.
    pub fn clear_review_focus(&mut self) {
        self.review_target = ReviewViewTarget::Disabled;
        self.pending_review_view = None;
    }

    /// Backward-compatible alias for [`Self::current_zoom`].
    pub fn zoom(&self) -> f32 {
        self.current_zoom()
    }

    fn clamp_to_viewport(&mut self, viewport: Rect, fitted_image: Rect) {
        self.zoom = finite_or(self.zoom, MIN_ZOOM).clamp(MIN_ZOOM, MAX_ZOOM);
        if self.zoom == MIN_ZOOM {
            self.pan = Vec2::ZERO;
            self.pan_mode = false;
            self.primary_pan = false;
        }
        self.pan = clamp_pan(viewport, fitted_image, self.zoom, self.pan);
    }

    fn apply_pending_review_view(&mut self, viewport: Rect, fitted_image: Rect) {
        let Some(target) = self.pending_review_view.take() else {
            return;
        };
        if let Some(target) = target {
            fit_normalized_rect(self, viewport, fitted_image, target);
        } else {
            self.fit_view();
        }
    }
}

#[derive(Clone, Debug)]
enum DragOperation {
    Create {
        start: Pos2,
    },
    Move {
        annotation_id: AnnotationId,
        original: BoundingBox,
        start: Pos2,
    },
    Resize {
        annotation_id: AnnotationId,
        original: BoundingBox,
        handle: ResizeHandle,
    },
    Keypoint {
        annotation_id: AnnotationId,
        keypoint_index: usize,
        original: NormalizedPoint,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResizeHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

include!("canvas/render.rs");
include!("canvas/painting.rs");
include!("canvas/interaction.rs");
include!("canvas/hit_testing.rs");
include!("canvas/viewport.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, MouseWheelUnit, TouchDeviceId, TouchId, TouchPhase};
    use egui_kittest::{Harness, kittest::Queryable};

    struct InteractiveTestState {
        canvas: CanvasState,
        actions: Vec<CanvasAction<BoundingBoxEdit>>,
        editable: bool,
        annotations: Vec<AnnotationVersion>,
        selected_annotation: Option<AnnotationId>,
    }

    impl Default for InteractiveTestState {
        fn default() -> Self {
            Self {
                canvas: CanvasState::default(),
                actions: Vec::new(),
                editable: true,
                annotations: Vec::new(),
                selected_annotation: None,
            }
        }
    }

    fn canvas_harness(bounding_box_tool: bool) -> Harness<'static, InteractiveTestState> {
        Harness::builder()
            .with_size(vec2(400.0, 300.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                move |ui, test: &mut InteractiveTestState| {
                    if let Some(action) = show_canvas_interactive(
                        ui,
                        &mut test.canvas,
                        None,
                        &test.annotations,
                        [400, 200],
                        bounding_box_tool,
                        test.selected_annotation.as_ref(),
                        test.editable,
                        &[],
                        &[],
                    ) {
                        test.actions.push(action);
                    }
                },
                InteractiveTestState::default(),
            )
    }

    fn correction_canvas_harness(
        annotation: AnnotationVersion,
        bounding_box_tool: bool,
    ) -> Harness<'static, InteractiveTestState> {
        let selected_annotation = annotation.annotation_id.clone();
        Harness::builder()
            .with_size(vec2(400.0, 300.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                move |ui, test: &mut InteractiveTestState| {
                    if let Some(action) = show_canvas_configured(
                        ui,
                        &mut test.canvas,
                        None,
                        &test.annotations,
                        [400, 200],
                        bounding_box_tool,
                        test.selected_annotation.as_ref(),
                        CanvasInteraction::correction(None),
                        &[],
                        &[],
                    ) {
                        test.actions.push(action);
                    }
                },
                InteractiveTestState {
                    annotations: vec![annotation],
                    selected_annotation: Some(selected_annotation),
                    ..Default::default()
                },
            )
    }

    fn test_annotation(geometry: AnnotationGeometry) -> AnnotationVersion {
        AnnotationVersion {
            annotation_id: AnnotationId::from("ann_test"),
            version: 1,
            object_group_id: None,
            origin: labello_domain::AnnotationOrigin::native(),
            task_id: labello_domain::TaskId::from("task"),
            class_id: labello_domain::ClassId::from("class"),
            annotation_type: match &geometry {
                AnnotationGeometry::BoundingBox(_) => labello_domain::AnnotationType::BoundingBox,
                AnnotationGeometry::Skeleton(_) => labello_domain::AnnotationType::Skeleton,
            },
            revision_source: labello_domain::RevisionSource::Human {
                action: labello_domain::HumanRevisionKind::Authored,
            },
            geometry,
            author_user_id: labello_domain::UserId::from("annotator"),
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
            deleted: false,
        }
    }

    #[test]
    fn editable_canvas_uses_move_resize_and_keypoint_cursors() {
        let image_rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        let annotation = test_annotation(AnnotationGeometry::BoundingBox(BoundingBox {
            x: 0.2,
            y: 0.2,
            width: 0.4,
            height: 0.4,
        }));
        let selected = annotation.annotation_id.clone();
        let annotations = [annotation];
        let state = CanvasState::default();
        let interaction = CanvasInteraction::annotations(true);

        assert_eq!(
            canvas_hover_cursor(
                Some(pos2(20.0, 20.0)),
                image_rect,
                &state,
                &annotations,
                Some(&selected),
                interaction,
                true,
                None,
                false,
                false,
            ),
            Some(egui::CursorIcon::ResizeNwSe)
        );
        assert_eq!(
            canvas_hover_cursor(
                Some(pos2(40.0, 40.0)),
                image_rect,
                &state,
                &annotations,
                Some(&selected),
                interaction,
                true,
                None,
                false,
                false,
            ),
            Some(egui::CursorIcon::Move)
        );
        assert_eq!(
            canvas_hover_cursor(
                Some(pos2(90.0, 90.0)),
                image_rect,
                &state,
                &annotations,
                Some(&selected),
                interaction,
                true,
                None,
                false,
                false,
            ),
            Some(egui::CursorIcon::Crosshair)
        );

        let skeleton = test_annotation(AnnotationGeometry::Skeleton(
            labello_domain::SkeletonGeometry {
                keypoints: vec![labello_domain::KeypointAnnotation {
                    name: "nose".to_string(),
                    state: labello_domain::KeypointState::Visible,
                    point: Some(NormalizedPoint { x: 0.5, y: 0.5 }),
                }],
            },
        ));
        let selected = skeleton.annotation_id.clone();
        assert_eq!(
            canvas_hover_cursor(
                Some(pos2(50.0, 50.0)),
                image_rect,
                &state,
                &[skeleton],
                Some(&selected),
                CanvasInteraction::correction(Some(0)),
                false,
                None,
                false,
                false,
            ),
            Some(egui::CursorIcon::Move)
        );

        assert_eq!(
            canvas_hover_cursor(
                Some(pos2(90.0, 90.0)),
                image_rect,
                &state,
                &annotations,
                Some(&selected),
                interaction,
                true,
                None,
                false,
                true,
            ),
            Some(egui::CursorIcon::Grabbing)
        );

        for (handle, cursor) in [
            (ResizeHandle::TopLeft, egui::CursorIcon::ResizeNwSe),
            (ResizeHandle::Top, egui::CursorIcon::ResizeVertical),
            (ResizeHandle::TopRight, egui::CursorIcon::ResizeNeSw),
            (ResizeHandle::Right, egui::CursorIcon::ResizeHorizontal),
            (ResizeHandle::BottomRight, egui::CursorIcon::ResizeNwSe),
            (ResizeHandle::Bottom, egui::CursorIcon::ResizeVertical),
            (ResizeHandle::BottomLeft, egui::CursorIcon::ResizeNeSw),
            (ResizeHandle::Left, egui::CursorIcon::ResizeHorizontal),
        ] {
            assert_eq!(resize_cursor(handle), cursor);
        }
    }

    #[test]
    fn unavailable_preview_is_exposed_to_accessibility() {
        let harness = canvas_harness(true);

        assert!(
            harness
                .query_by_label("Image preview unavailable")
                .is_some()
        );
    }

    fn click_at(harness: &mut Harness<'_, InteractiveTestState>, pos: Pos2) {
        harness.event(Event::PointerMoved(pos));
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.step();
    }

    fn drag_at(
        harness: &mut Harness<'_, InteractiveTestState>,
        button: PointerButton,
        start: Pos2,
        end: Pos2,
    ) {
        drag_at_with_modifiers(harness, button, start, end, Modifiers::NONE);
    }

    fn drag_at_with_modifiers(
        harness: &mut Harness<'_, InteractiveTestState>,
        button: PointerButton,
        start: Pos2,
        end: Pos2,
        modifiers: Modifiers,
    ) {
        harness.input_mut().modifiers = modifiers;
        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button,
            pressed: true,
            modifiers,
        });
        harness.event(Event::PointerMoved(end));
        harness.event(Event::PointerButton {
            pos: end,
            button,
            pressed: false,
            modifiers,
        });
        harness.step();
        harness.input_mut().modifiers = Modifiers::NONE;
    }

    fn stepped_primary_drag(
        harness: &mut Harness<'_, InteractiveTestState>,
        start: Pos2,
        end: Pos2,
    ) {
        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        harness.event(Event::PointerMoved(end));
        harness.step();
        harness.event(Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }

    fn bbox(x: f32, y: f32, width: f32, height: f32) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width,
            height,
        }
    }

    fn assert_bbox(actual: BoundingBox, expected: BoundingBox) {
        assert!((actual.x - expected.x).abs() < 0.000_01, "x: {actual:?}");
        assert!((actual.y - expected.y).abs() < 0.000_01, "y: {actual:?}");
        assert!(
            (actual.width - expected.width).abs() < 0.000_01,
            "width: {actual:?}"
        );
        assert!(
            (actual.height - expected.height).abs() < 0.000_01,
            "height: {actual:?}"
        );
        assert!(actual.validate().is_ok(), "invalid bbox: {actual:?}");
    }

    #[test]
    fn points_create_normalized_boxes_in_every_drag_direction() {
        let expected = bbox(0.2, 0.3, 0.6, 0.6);
        for (a, b) in [
            (pos2(0.2, 0.3), pos2(0.8, 0.9)),
            (pos2(0.8, 0.3), pos2(0.2, 0.9)),
            (pos2(0.8, 0.9), pos2(0.2, 0.3)),
            (pos2(0.2, 0.9), pos2(0.8, 0.3)),
        ] {
            assert_bbox(bbox_from_normalized_points(a, b).unwrap(), expected);
        }
    }

    #[test]
    fn point_creation_clamps_to_image_and_rejects_zero_area() {
        assert_bbox(
            bbox_from_normalized_points(pos2(-2.0, 0.25), pos2(4.0, 2.0)).unwrap(),
            bbox(0.0, 0.25, 1.0, 0.75),
        );
        assert!(bbox_from_normalized_points(pos2(0.2, 0.2), pos2(0.2, 0.8)).is_none());
        assert!(bbox_from_normalized_points(pos2(-2.0, -1.0), pos2(-1.0, -3.0)).is_none());
    }

    #[test]
    fn screen_and_normalized_geometry_round_trip() {
        let image = Rect::from_min_size(pos2(25.0, 50.0), vec2(800.0, 400.0));
        let point = pos2(0.125, 0.75);
        let screen = normalized_to_screen(image, point);
        assert_eq!(screen, pos2(125.0, 350.0));
        let round_trip = screen_to_normalized(image, screen);
        assert!((round_trip.x - point.x).abs() < f32::EPSILON);
        assert!((round_trip.y - point.y).abs() < f32::EPSILON);

        let rect = bbox_to_screen_rect(image, bbox(0.25, 0.1, 0.5, 0.4));
        assert_eq!(rect.min, pos2(225.0, 90.0));
        assert_eq!(rect.max, pos2(625.0, 250.0));
    }

    #[test]
    fn skeleton_edge_endpoint_uses_active_keypoint_preview() {
        let annotation_id = AnnotationId::from("annotation-preview");
        let other_annotation_id = AnnotationId::from("annotation-other");
        let original = NormalizedPoint { x: 0.2, y: 0.3 };
        let preview = NormalizedPoint { x: 0.7, y: 0.8 };
        let skeleton = labello_domain::SkeletonGeometry {
            keypoints: vec![labello_domain::KeypointAnnotation {
                name: "nose".to_string(),
                state: labello_domain::KeypointState::Visible,
                point: Some(original),
            }],
        };

        assert_eq!(
            skeleton_keypoint_point(
                &annotation_id,
                &skeleton,
                "nose",
                Some((&annotation_id, 0, preview)),
            ),
            Some(preview)
        );
        assert_eq!(
            skeleton_keypoint_point(
                &annotation_id,
                &skeleton,
                "nose",
                Some((&other_annotation_id, 0, preview)),
            ),
            Some(original)
        );
    }

    #[test]
    fn degenerate_viewports_and_non_finite_dashes_have_finite_fallbacks() {
        let degenerate = Rect::from_min_size(pos2(10.0, 20.0), Vec2::ZERO);
        let fitted = fitted_image_rect(degenerate, [0, 0]);
        assert_eq!(fitted.size(), Vec2::ZERO);
        assert_eq!(
            screen_to_normalized(fitted, pos2(50.0, 60.0)),
            pos2(0.5, 0.5)
        );
        assert_eq!(
            clamp_pan(degenerate, fitted, f32::NAN, Vec2::splat(f32::NAN)),
            Vec2::ZERO
        );

        let context = egui::Context::default();
        let painter = context.layer_painter(egui::LayerId::background());
        paint_dashed_segment(
            &painter,
            pos2(f32::NAN, 0.0),
            pos2(f32::INFINITY, 1.0),
            Color32::WHITE,
        );
    }

    #[test]
    fn malformed_boxes_are_finite_normalized_and_nonzero() {
        for malformed in [
            bbox(-0.5, -1.0, 3.0, 4.0),
            bbox(0.9, 0.8, -2.0, -3.0),
            bbox(f32::NAN, f32::INFINITY, f32::NAN, f32::NEG_INFINITY),
            bbox(2.0, 2.0, 2.0, 2.0),
        ] {
            let clamped = clamp_bbox(malformed);
            assert!(clamped.validate().is_ok(), "{malformed:?} -> {clamped:?}");
            assert!(clamped.width >= MIN_BOX_SIZE - f32::EPSILON);
            assert!(clamped.height >= MIN_BOX_SIZE - f32::EPSILON);
        }
    }

    #[test]
    fn moving_preserves_size_and_clamps_each_image_edge() {
        let original = bbox(0.2, 0.3, 0.4, 0.2);
        assert_bbox(
            move_bbox(original, vec2(0.1, 0.15)),
            bbox(0.3, 0.45, 0.4, 0.2),
        );
        assert_bbox(
            move_bbox(original, vec2(-2.0, -2.0)),
            bbox(0.0, 0.0, 0.4, 0.2),
        );
        assert_bbox(
            move_bbox(original, vec2(2.0, 2.0)),
            bbox(0.6, 0.8, 0.4, 0.2),
        );
        assert_bbox(move_bbox(original, vec2(f32::NAN, f32::INFINITY)), original);
        assert!(!bbox_changed(original, move_bbox(original, Vec2::ZERO)));
        assert!(bbox_changed(original, move_bbox(original, vec2(0.01, 0.0))));
    }

    #[test]
    fn corner_resize_changes_two_edges_and_keeps_opposite_corner() {
        let original = bbox(0.2, 0.3, 0.4, 0.4);
        assert_bbox(
            resize_bbox(original, ResizeHandle::TopLeft, pos2(0.1, 0.15)),
            bbox(0.1, 0.15, 0.5, 0.55),
        );
        assert_bbox(
            resize_bbox(original, ResizeHandle::BottomRight, pos2(0.9, 0.95)),
            bbox(0.2, 0.3, 0.7, 0.65),
        );
    }

    #[test]
    fn edge_resize_only_changes_its_axis() {
        let original = bbox(0.2, 0.3, 0.4, 0.4);
        assert_bbox(
            resize_bbox(original, ResizeHandle::Top, pos2(0.99, 0.1)),
            bbox(0.2, 0.1, 0.4, 0.6),
        );
        assert_bbox(
            resize_bbox(original, ResizeHandle::Right, pos2(0.8, 0.99)),
            bbox(0.2, 0.3, 0.6, 0.4),
        );
        assert_bbox(
            resize_bbox(original, ResizeHandle::Bottom, pos2(0.99, 0.9)),
            bbox(0.2, 0.3, 0.4, 0.6),
        );
        assert_bbox(
            resize_bbox(original, ResizeHandle::Left, pos2(0.1, 0.99)),
            bbox(0.1, 0.3, 0.5, 0.4),
        );
    }

    #[test]
    fn resize_clamps_to_bounds_and_cannot_cross_opposite_edge() {
        let original = bbox(0.2, 0.3, 0.4, 0.4);
        assert_bbox(
            resize_bbox(original, ResizeHandle::TopLeft, pos2(-2.0, -3.0)),
            bbox(0.0, 0.0, 0.6, 0.7),
        );
        let collapsed = resize_bbox(original, ResizeHandle::TopLeft, pos2(2.0, 2.0));
        assert_bbox(
            collapsed,
            bbox(
                0.6 - MIN_BOX_SIZE,
                0.7 - MIN_BOX_SIZE,
                MIN_BOX_SIZE,
                MIN_BOX_SIZE,
            ),
        );
        assert_bbox(
            resize_bbox(original, ResizeHandle::BottomRight, pos2(2.0, 3.0)),
            bbox(0.2, 0.3, 0.8, 0.7),
        );
    }

    #[test]
    fn all_resize_handles_produce_valid_geometry_for_outside_pointers() {
        let original = bbox(0.2, 0.3, 0.4, 0.4);
        for (handle, _) in resize_handles(Rect::EVERYTHING) {
            for pointer in [pos2(-5.0, -5.0), pos2(5.0, 5.0), pos2(f32::NAN, f32::NAN)] {
                assert!(resize_bbox(original, handle, pointer).validate().is_ok());
            }
        }
    }

    #[test]
    fn handle_hit_testing_covers_corners_and_edges() {
        let rect = Rect::from_min_max(pos2(20.0, 30.0), pos2(120.0, 90.0));
        for (handle, center) in resize_handles(rect) {
            assert_eq!(resize_handle_at(center, rect), Some(handle));
        }
        assert_eq!(resize_handle_at(rect.center(), rect), None);
    }

    #[test]
    fn aspect_fit_letterboxes_landscape_and_portrait_images() {
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 200.0));
        let landscape = fitted_image_rect(viewport, [800, 200]);
        assert_eq!(landscape.size(), vec2(400.0, 100.0));
        assert_eq!(landscape.center(), viewport.center());

        let portrait = fitted_image_rect(viewport, [100, 400]);
        assert_eq!(portrait.size(), vec2(50.0, 200.0));
        assert_eq!(portrait.center(), viewport.center());

        let zero_dimensions = fitted_image_rect(viewport, [0, 0]);
        assert_eq!(zero_dimensions.size(), vec2(200.0, 200.0));
    }

    #[test]
    fn rounded_corner_mask_covers_all_viewport_corners() {
        let viewport = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
        let mask = rounded_corner_mask(viewport, Color32::BLACK);

        assert!(mask.is_valid());
        assert_eq!(mask.calc_bounds(), viewport);
        assert_eq!(mask.indices.len(), 4 * CORNER_MASK_SEGMENTS as usize * 3);
    }

    #[test]
    fn zoom_scales_the_fitted_image_in_a_fixed_viewport() {
        let viewport = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
        let fitted = fitted_image_rect(viewport, [800, 200]);
        let transformed = transformed_image_rect(fitted, 2.0, vec2(25.0, -10.0));
        assert_eq!(transformed.size(), vec2(800.0, 200.0));
        assert_eq!(transformed.center(), viewport.center() + vec2(25.0, -10.0));
    }

    #[test]
    fn pan_clamps_to_the_fitted_image_footprint() {
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 300.0));
        let fitted = fitted_image_rect(viewport, [800, 200]);
        assert_eq!(fitted.size(), vec2(400.0, 100.0));
        assert_eq!(
            clamp_pan(viewport, fitted, 2.0, vec2(999.0, -999.0)),
            vec2(200.0, -50.0)
        );
        assert_eq!(
            clamp_pan(viewport, fitted, 4.0, vec2(-999.0, 999.0)),
            vec2(-600.0, 150.0)
        );
    }

    #[test]
    fn pan_is_reclamped_when_the_viewport_resizes() {
        let old_viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0));
        let old_fit = fitted_image_rect(old_viewport, [800, 400]);
        let mut state = CanvasState {
            zoom: 2.0,
            pan: vec2(400.0, 200.0),
            ..Default::default()
        };
        state.clamp_to_viewport(old_viewport, old_fit);
        assert_eq!(state.pan, vec2(400.0, 200.0));

        let resized = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 500.0));
        let resized_fit = fitted_image_rect(resized, [800, 400]);
        state.clamp_to_viewport(resized, resized_fit);
        assert_eq!(state.pan, vec2(150.0, 75.0));
    }

    #[test]
    fn zoom_is_anchored_under_pointer_and_pan_is_clamped() {
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 200.0));
        let fitted = fitted_image_rect(viewport, [400, 200]);
        let focus = pos2(200.0, 250.0);
        let mut state = CanvasState::default();
        let before =
            screen_to_normalized(transformed_image_rect(fitted, state.zoom, state.pan), focus);
        set_zoom_around(&mut state, viewport, fitted, focus, 2.0);
        let after =
            screen_to_normalized(transformed_image_rect(fitted, state.zoom, state.pan), focus);
        assert!((before.x - after.x).abs() < 0.000_01);
        assert!((before.y - after.y).abs() < 0.000_01);
        assert_eq!(
            clamp_pan(viewport, fitted, 2.0, vec2(999.0, -999.0)),
            vec2(200.0, -100.0)
        );

        state.fit_view();
        assert_eq!(state.zoom(), 1.0);
        assert_eq!(state.pan, Vec2::ZERO);
    }

    #[test]
    fn zoom_keeps_pointer_anchor_across_letterboxed_viewport_edges() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        for (image_size, focus_at_edge) in [
            ([400, 100], pos2(viewport.center().x, viewport.top() + 50.0)),
            (
                [100, 300],
                pos2(viewport.left() + 100.0, viewport.center().y),
            ),
        ] {
            let fitted = fitted_image_rect(viewport, image_size);
            let mut state = CanvasState {
                zoom: 2.0,
                ..Default::default()
            };
            let before = screen_to_normalized(
                transformed_image_rect(fitted, state.zoom, state.pan),
                focus_at_edge,
            );

            set_zoom_around(&mut state, viewport, fitted, focus_at_edge, 4.0);

            let after = screen_to_normalized(
                transformed_image_rect(fitted, state.zoom, state.pan),
                focus_at_edge,
            );
            assert!(before.distance(after) < 0.000_01, "{image_size:?}");
        }
    }

    #[test]
    fn first_centered_zoom_discards_stale_default_pan_without_drift() {
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 200.0));
        let fitted = fitted_image_rect(viewport, [400, 200]);
        let mut state = CanvasState {
            pan: vec2(80.0, -40.0),
            ..Default::default()
        };

        set_zoom_around(&mut state, viewport, fitted, viewport.center(), 2.0);

        assert_eq!(state.zoom, 2.0);
        assert_eq!(state.pan, Vec2::ZERO);
        assert_eq!(
            transformed_image_rect(fitted, state.zoom, state.pan).center(),
            viewport.center()
        );
    }

    #[test]
    fn pinch_zoom_uses_previous_center_before_translation() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let fitted = fitted_image_rect(viewport, [400, 200]);
        let previous_center = pos2(160.0, 90.0);
        let translation = vec2(25.0, 10.0);
        let anchor = screen_to_normalized(fitted, previous_center);
        let mut state = CanvasState::default();

        set_zoom_around(&mut state, viewport, fitted, previous_center, 2.0);
        state.pan += translation;
        state.clamp_to_viewport(viewport, fitted);

        let moved_anchor = normalized_to_screen(
            transformed_image_rect(fitted, state.zoom, state.pan),
            anchor,
        );
        assert!(moved_anchor.distance(previous_center + translation) < 0.000_01);
    }

    #[test]
    fn review_object_focus_is_centered_with_margin_and_applied_once() {
        let viewport = Rect::from_min_size(pos2(20.0, 30.0), vec2(600.0, 400.0));
        let fitted = fitted_image_rect(viewport, [1200, 800]);
        let target = Rect::from_min_max(pos2(0.4, 0.35), pos2(0.6, 0.65));
        let mut state = CanvasState {
            pending_review_view: Some(Some(target)),
            ..Default::default()
        };

        state.apply_pending_review_view(viewport, fitted);
        let image = transformed_image_rect(fitted, state.zoom, state.pan);
        let focused = Rect::from_min_max(
            normalized_to_screen(image, target.min),
            normalized_to_screen(image, target.max),
        );
        assert!(focused.center().distance(viewport.center()) < 0.001);
        assert!(focused.width() <= viewport.width() / FOCUS_MARGIN + 0.000_1);
        assert!(focused.height() <= viewport.height() / FOCUS_MARGIN + 0.000_1);
        assert!(state.zoom > MIN_ZOOM);

        state.pan += vec2(5.0, 0.0);
        state.apply_pending_review_view(viewport, fitted);
        assert_eq!(state.pan.x, 5.0);
    }

    #[test]
    fn full_image_review_target_restores_fit_view() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0));
        let fitted = fitted_image_rect(viewport, [1200, 800]);
        let mut state = CanvasState {
            zoom: 4.0,
            pan: vec2(75.0, -20.0),
            pending_review_view: Some(None),
            ..Default::default()
        };

        state.apply_pending_review_view(viewport, fitted);

        assert_eq!(state.zoom, MIN_ZOOM);
        assert_eq!(state.pan, Vec2::ZERO);
    }

    #[test]
    fn canvas_state_zoom_methods_are_bounded_and_fit_clears_pan() {
        let mut state = CanvasState::default();
        state.zoom_in();
        assert_eq!(state.current_zoom(), ZOOM_STEP);
        state.pan = vec2(20.0, -10.0);
        state.zoom_in();
        assert_eq!(state.pan, vec2(25.0, -12.5));

        for _ in 0..100 {
            state.zoom_in();
        }
        assert_eq!(state.current_zoom(), MAX_ZOOM);
        for _ in 0..100 {
            state.zoom_out();
        }
        assert_eq!(state.current_zoom(), MIN_ZOOM);
        assert_eq!(state.pan, Vec2::ZERO);

        state.zoom_in();
        state.pan = Vec2::splat(12.0);
        state.fit_view();
        assert_eq!(state.current_zoom(), MIN_ZOOM);
        assert_eq!(state.pan, Vec2::ZERO);
    }

    #[test]
    fn wheel_zoom_factor_is_directional_symmetric_and_finite() {
        let inward = wheel_zoom_factor(vec2(0.0, 120.0));
        let outward = wheel_zoom_factor(vec2(0.0, -120.0));
        assert!(inward > 1.0);
        assert!(outward < 1.0);
        assert!((inward * outward - 1.0).abs() < 0.000_01);
        assert!(wheel_zoom_factor(vec2(f32::INFINITY, 0.0)).is_finite());
    }

    #[test]
    fn canvas_interaction_uses_the_full_available_viewport() {
        let harness = canvas_harness(false);
        let canvas = harness.get_by_label("Annotation canvas").rect();
        assert_eq!(canvas.size(), vec2(384.0, 284.0));
    }

    #[test]
    fn plain_wheel_over_canvas_zooms_without_a_modifier() {
        let mut harness = canvas_harness(false);
        let center = harness.get_by_label("Annotation canvas").rect().center();
        harness.event(Event::PointerMoved(center));
        harness.event(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        assert!(harness.state().canvas.current_zoom() > MIN_ZOOM);
    }

    #[test]
    fn primary_drag_creates_a_normalized_bounding_box() {
        let mut harness = canvas_harness(true);
        let canvas = harness.get_by_label("Annotation canvas").rect();
        let start = canvas.center() - vec2(80.0, 40.0);
        let end = start + vec2(120.0, 80.0);
        drag_at(&mut harness, PointerButton::Primary, start, end);

        let [CanvasAction::CreateBoundingBox(created)] = harness.state().actions.as_slice() else {
            panic!("expected one bounding-box creation action");
        };
        assert!(created.validate().is_ok());
        assert!(created.width > 0.0 && created.height > 0.0);
    }

    #[test]
    fn correction_canvas_moves_existing_box_but_never_creates_one() {
        let annotation = test_annotation(AnnotationGeometry::BoundingBox(bbox(0.2, 0.2, 0.2, 0.2)));
        let mut harness = correction_canvas_harness(annotation, true);
        let canvas = harness.get_by_label("Annotation canvas").rect();
        let image_top = canvas.center().y - canvas.width() * 0.25;
        let start = pos2(
            canvas.left() + canvas.width() * 0.3,
            image_top + canvas.width() * 0.15,
        );
        drag_at(
            &mut harness,
            PointerButton::Primary,
            start,
            start + vec2(30.0, 20.0),
        );
        assert!(matches!(
            harness.state().actions.as_slice(),
            [CanvasAction::EditBoundingBox(_)]
        ));

        harness.state_mut().actions.clear();
        let top_left = pos2(
            canvas.left() + canvas.width() * 0.2,
            image_top + canvas.width() * 0.1,
        );
        drag_at(
            &mut harness,
            PointerButton::Primary,
            top_left,
            top_left - vec2(20.0, 15.0),
        );
        let [CanvasAction::EditBoundingBox(resized)] = harness.state().actions.as_slice() else {
            panic!("expected one bounding-box resize action");
        };
        assert!(resized.bounding_box.width > 0.2);
        assert!(resized.bounding_box.height > 0.2);

        harness.state_mut().actions.clear();
        let blank = pos2(canvas.right() - 20.0, canvas.bottom() - 20.0);
        drag_at(
            &mut harness,
            PointerButton::Primary,
            blank,
            blank - vec2(50.0, 30.0),
        );
        assert!(harness.state().actions.is_empty());
    }

    #[test]
    fn correction_canvas_selects_and_drags_existing_keypoint() {
        let annotation = test_annotation(AnnotationGeometry::Skeleton(
            labello_domain::SkeletonGeometry {
                keypoints: vec![labello_domain::KeypointAnnotation {
                    name: "nose".to_string(),
                    state: labello_domain::KeypointState::Visible,
                    point: Some(NormalizedPoint { x: 0.5, y: 0.5 }),
                }],
            },
        ));
        let mut harness = correction_canvas_harness(annotation, false);
        let center = harness.get_by_label("Annotation canvas").rect().center();
        drag_at(
            &mut harness,
            PointerButton::Primary,
            center,
            center + vec2(40.0, -20.0),
        );
        let [CanvasAction::EditKeypoint(edit)] = harness.state().actions.as_slice() else {
            panic!("expected one keypoint edit action");
        };
        assert_eq!(edit.keypoint_index, 0);
        assert!(edit.point.x > 0.5);
        assert!(edit.point.y < 0.5);
    }

    #[test]
    fn drag_is_cancelled_when_canvas_becomes_read_only_or_pointer_is_cancelled() {
        let mut harness = canvas_harness(true);
        let canvas = harness.get_by_label("Annotation canvas").rect();
        let start = canvas.center();
        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        assert!(harness.state().canvas.is_dragging());

        harness.state_mut().editable = false;
        harness.step();
        assert!(!harness.state().canvas.is_dragging());
        assert!(harness.state().actions.is_empty());

        harness.state_mut().editable = true;
        harness.event(Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        assert!(harness.state().canvas.is_dragging());
        harness.key_press(Key::Escape);
        harness.step();
        assert!(!harness.state().canvas.is_dragging());
        assert!(harness.state().actions.is_empty());

        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        assert!(harness.state().canvas.is_dragging());
        harness.event(Event::PointerGone);
        harness.step();
        assert!(!harness.state().canvas.is_dragging());
    }

    #[test]
    fn wheel_zoom_cancels_an_active_annotation_drag() {
        let mut harness = canvas_harness(true);
        let center = harness.get_by_label("Annotation canvas").rect().center();
        harness.event(Event::PointerMoved(center));
        harness.event(Event::PointerButton {
            pos: center,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        assert!(harness.state().canvas.is_dragging());
        harness.event(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        assert!(!harness.state().canvas.is_dragging());
        assert!(harness.state().actions.is_empty());
    }

    #[test]
    fn middle_and_configured_primary_drags_pan_without_annotating() {
        let mut harness = canvas_harness(true);
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.zoom_in();
        let center = harness.get_by_label("Annotation canvas").rect().center();
        drag_at(
            &mut harness,
            PointerButton::Middle,
            center,
            center + vec2(40.0, 15.0),
        );
        assert!(harness.state().canvas.pan.x > 0.0);
        assert!(harness.state().actions.is_empty());

        let pan_before = harness.state().canvas.pan;
        drag_at_with_modifiers(
            &mut harness,
            PointerButton::Primary,
            center,
            center - vec2(30.0, 10.0),
            Modifiers::CTRL,
        );
        assert!(harness.state().canvas.pan.x < pan_before.x);
        assert!(harness.state().actions.is_empty());

        harness
            .state_mut()
            .canvas
            .set_pan_drag_modifier(PanDragModifier::Alt);
        let pan_before = harness.state().canvas.pan;
        drag_at_with_modifiers(
            &mut harness,
            PointerButton::Primary,
            center,
            center + vec2(20.0, 5.0),
            Modifiers::ALT,
        );
        assert!(harness.state().canvas.pan.x > pan_before.x);
        assert!(harness.state().actions.is_empty());

        harness.key_down(Key::Space);
        drag_at(
            &mut harness,
            PointerButton::Primary,
            center - vec2(20.0, 20.0),
            center + vec2(20.0, 20.0),
        );
        harness.key_up(Key::Space);
        harness.step();
        assert!(
            matches!(
                harness.state().actions.last(),
                Some(CanvasAction::CreateBoundingBox(_))
            ),
            "Space must no longer turn a primary drag into a Pan gesture"
        );
    }

    #[test]
    fn pan_mode_primary_drag_pans_and_fit_restores_annotation_mode() {
        let mut harness = canvas_harness(true);
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.toggle_pan_mode();
        harness.step();
        assert!(harness.state().canvas.pan_mode());

        stepped_primary_drag(&mut harness, pos2(200.0, 150.0), pos2(230.0, 175.0));
        assert_ne!(harness.state().canvas.pan, Vec2::ZERO);
        assert!(harness.state().actions.is_empty());

        harness.state_mut().canvas.fit_view();
        harness.step();
        assert!(!harness.state().canvas.pan_mode());
        drag_at(
            &mut harness,
            PointerButton::Primary,
            pos2(100.0, 100.0),
            pos2(180.0, 160.0),
        );
        assert!(matches!(
            harness.state().actions.last(),
            Some(CanvasAction::CreateBoundingBox(_))
        ));
    }

    #[test]
    fn required_pan_mode_cannot_be_exited_and_unlocks_to_annotation_input() {
        let mut harness = canvas_harness(true);
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.require_pan_mode(true);
        harness.step();
        assert!(harness.state().canvas.pan_mode());
        assert!(harness.state().canvas.pan_mode_required());

        stepped_primary_drag(&mut harness, pos2(200.0, 150.0), pos2(230.0, 175.0));
        assert_ne!(harness.state().canvas.pan, Vec2::ZERO);
        assert!(harness.state().actions.is_empty());

        harness.state_mut().canvas.toggle_pan_mode();
        harness.state_mut().canvas.exit_pan_mode();
        harness.state_mut().canvas.fit_view();
        harness.step();
        assert!(harness.state().canvas.pan_mode());

        harness.state_mut().canvas.require_pan_mode(false);
        harness.step();
        assert!(!harness.state().canvas.pan_mode());
        drag_at(
            &mut harness,
            PointerButton::Primary,
            pos2(100.0, 100.0),
            pos2(180.0, 160.0),
        );
        assert!(matches!(
            harness.state().actions.last(),
            Some(CanvasAction::CreateBoundingBox(_))
        ));
    }

    #[test]
    fn double_click_still_fits_while_pan_mode_is_active() {
        let mut harness = canvas_harness(true);
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.toggle_pan_mode();
        harness.step();

        click_at(&mut harness, pos2(200.0, 150.0));
        click_at(&mut harness, pos2(200.0, 150.0));

        assert_eq!(harness.state().canvas.zoom, MIN_ZOOM);
        assert!(!harness.state().canvas.pan_mode());
        assert!(harness.state().actions.is_empty());
    }

    #[test]
    fn two_finger_gesture_zooms_and_pans_without_annotating() {
        let mut harness = canvas_harness(true);
        let center = harness.get_by_label("Annotation canvas").rect().center();
        let device = TouchDeviceId(1);
        let touch = |id, phase, pos| Event::Touch {
            device_id: device,
            id: TouchId(id),
            phase,
            pos,
            force: None,
        };
        // Touch integrations report the primary pointer position alongside touch events.
        harness.event(Event::PointerMoved(center));
        harness.event(touch(1, TouchPhase::Start, center - vec2(50.0, 0.0)));
        harness.event(touch(2, TouchPhase::Start, center + vec2(50.0, 0.0)));
        harness.event(touch(1, TouchPhase::Move, center - vec2(70.0, -10.0)));
        harness.event(touch(2, TouchPhase::Move, center + vec2(90.0, 10.0)));
        harness.event(touch(1, TouchPhase::End, center - vec2(70.0, -10.0)));
        harness.event(touch(2, TouchPhase::End, center + vec2(90.0, 10.0)));
        harness.step();

        assert!(harness.state().canvas.current_zoom() > MIN_ZOOM);
        assert!(harness.state().actions.is_empty());
    }

    #[test]
    fn double_click_fits_view_and_does_not_emit_a_second_annotation_action() {
        let mut harness = canvas_harness(false);
        let center = harness.get_by_label("Annotation canvas").rect().center();
        click_at(&mut harness, center);
        assert_eq!(harness.state().actions.len(), 1);
        assert!(matches!(
            harness.state().actions[0],
            CanvasAction::PlaceKeypoint(_)
        ));
        let first_click = harness.state().canvas.last_canvas_click.unwrap();

        harness.state_mut().canvas.zoom_in();
        harness.state_mut().canvas.pan = vec2(20.0, 10.0);
        click_at(&mut harness, center);
        let second_click = harness.state().canvas.last_canvas_click.unwrap();
        assert!(second_click.0 - first_click.0 <= DOUBLE_CLICK_DELAY);
        assert!(second_click.1.distance(first_click.1) <= DOUBLE_CLICK_DISTANCE);
        assert_eq!(harness.state().actions.len(), 1);
        assert_eq!(harness.state().canvas.current_zoom(), MIN_ZOOM);
        assert_eq!(harness.state().canvas.pan, Vec2::ZERO);
    }
}

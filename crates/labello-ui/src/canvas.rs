use std::{convert::Infallible, f32::consts::PI};

use egui::{
    Color32, CornerRadius, Key, Mesh, PointerButton, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationVersion, BoundingBox, NormalizedPoint,
    PrelabelSuggestion,
};

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
    space_pan: bool,
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
            space_pan: false,
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
    Annotation(AnnotationId),
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
        }
    }

    /// Fit the image to the canvas and center it.
    pub fn fit_view(&mut self) {
        self.cancel_drag();
        self.zoom = MIN_ZOOM;
        self.pan = Vec2::ZERO;
    }

    /// Focus a review object once when it becomes active, or fit the full image
    /// when object-by-object review is complete.
    pub fn set_review_focus(&mut self, annotation: Option<&AnnotationVersion>) {
        let target = annotation.map_or(ReviewViewTarget::FullImage, |annotation| {
            ReviewViewTarget::Annotation(annotation.annotation_id.clone())
        });
        if self.review_target == target {
            return;
        }
        self.review_target = target;
        self.pending_review_view = Some(annotation.and_then(annotation_focus_rect));
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

/// Backward-compatible canvas entry point.
///
/// Use [`show_canvas_interactive`] when selection feedback, editing, or a
/// read-only canvas is required.
pub fn show_canvas(
    ui: &mut Ui,
    state: &mut CanvasState,
    texture: Option<&egui::TextureHandle>,
    annotations: &[AnnotationVersion],
    image_size: [u32; 2],
    bounding_box_tool: bool,
) -> Option<CanvasAction> {
    match show_canvas_interactive(
        ui,
        state,
        texture,
        annotations,
        image_size,
        bounding_box_tool,
        None,
        true,
        &[],
        &[],
    ) {
        Some(CanvasAction::CreateBoundingBox(bbox)) => Some(CanvasAction::CreateBoundingBox(bbox)),
        Some(CanvasAction::PlaceKeypoint(point)) => Some(CanvasAction::PlaceKeypoint(point)),
        Some(CanvasAction::Select(id)) => Some(CanvasAction::Select(id)),
        // No annotation is selected above, so an edit cannot be started.
        Some(CanvasAction::EditBoundingBox(_))
        | Some(CanvasAction::SelectKeypoint(_))
        | Some(CanvasAction::EditKeypoint(_))
        | None => None,
    }
}

/// Show a canvas with selection, box editing, and read-only support.
///
/// Integration is unchanged: place this call in the center UI after surrounding
/// panels and pass the source image dimensions in `image_size`. The canvas
/// consumes all remaining UI space, derives the fitted image rectangle itself,
/// and retains viewport state in `state`; no separate available-size or viewport
/// argument is required.
///
/// A plain wheel or pinch zooms around the pointer. Middle-button drag,
/// space+primary drag, and a two-finger translation pan. A double-click resets
/// the view. A single pointer or stylus keeps its annotation behavior: it moves
/// or resizes a selected box, creates a box with the bounding-box tool, or places
/// a skeleton keypoint.
#[allow(clippy::too_many_arguments)]
pub fn show_canvas_interactive(
    ui: &mut Ui,
    state: &mut CanvasState,
    texture: Option<&egui::TextureHandle>,
    annotations: &[AnnotationVersion],
    image_size: [u32; 2],
    bounding_box_tool: bool,
    selected_annotation: Option<&AnnotationId>,
    editable: bool,
    skeleton_edges: &[(String, String)],
    prelabels: &[PrelabelSuggestion],
) -> Option<CanvasAction<BoundingBoxEdit>> {
    show_canvas_configured(
        ui,
        state,
        texture,
        annotations,
        image_size,
        bounding_box_tool,
        selected_annotation,
        CanvasInteraction::annotations(editable),
        skeleton_edges,
        prelabels,
    )
}

/// Show a canvas with an explicit interaction policy.
#[allow(clippy::too_many_arguments)]
pub fn show_canvas_configured(
    ui: &mut Ui,
    state: &mut CanvasState,
    texture: Option<&egui::TextureHandle>,
    annotations: &[AnnotationVersion],
    image_size: [u32; 2],
    bounding_box_tool: bool,
    selected_annotation: Option<&AnnotationId>,
    interaction: CanvasInteraction,
    skeleton_edges: &[(String, String)],
    prelabels: &[PrelabelSuggestion],
) -> Option<CanvasAction<BoundingBoxEdit>> {
    let editable = interaction.editable;
    let available = ui.available_size().max(vec2(1.0, 1.0));
    let (viewport, _) = ui.allocate_exact_size(available, Sense::hover());
    let interaction_rect = viewport;
    if !editable {
        state.cancel_drag();
    }
    let response = ui.interact(
        interaction_rect,
        ui.id().with("annotation_canvas"),
        Sense::click_and_drag(),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Annotation canvas"));
    if !valid_rect(interaction_rect) {
        state.cancel_drag();
        paint_canvas(
            ui,
            viewport,
            Rect::from_center_size(interaction_rect.center(), Vec2::ZERO),
            None,
            annotations,
            selected_annotation,
            false,
            None,
            None,
            interaction.selected_keypoint,
            None,
            skeleton_edges,
            prelabels,
        );
        return None;
    }
    let fitted_image = fitted_image_rect(interaction_rect, image_size);
    state.clamp_to_viewport(interaction_rect, fitted_image);
    state.apply_pending_review_view(interaction_rect, fitted_image);

    let view_consumed = handle_view_gestures(
        ui,
        &response,
        interaction_rect,
        fitted_image,
        interaction_rect,
        state,
    );
    let image_rect = transformed_image_rect(fitted_image, state.zoom, state.pan);

    let preview = if editable {
        state.draft_box.and_then(|bbox| match &state.drag {
            Some(DragOperation::Move { annotation_id, .. })
            | Some(DragOperation::Resize { annotation_id, .. }) => Some((annotation_id, bbox)),
            _ => None,
        })
    } else {
        None
    };
    let draft = if editable && matches!(state.drag, Some(DragOperation::Create { .. })) {
        state.draft_box
    } else {
        None
    };
    let keypoint_preview = if editable {
        match &state.drag {
            Some(DragOperation::Keypoint {
                annotation_id,
                keypoint_index,
                ..
            }) => state
                .draft_keypoint
                .map(|point| (annotation_id, *keypoint_index, point)),
            _ => None,
        }
    } else {
        None
    };
    paint_canvas(
        ui,
        viewport,
        image_rect,
        texture,
        annotations,
        selected_annotation,
        editable,
        preview,
        draft,
        interaction.selected_keypoint,
        keypoint_preview,
        skeleton_edges,
        prelabels,
    );

    let action = handle_annotation_pointer(
        ui,
        response,
        interaction_rect,
        image_rect,
        state,
        annotations,
        bounding_box_tool,
        selected_annotation,
        interaction,
        view_consumed,
    );
    state.clamp_to_viewport(interaction_rect, fitted_image);
    action
}

#[allow(clippy::too_many_arguments)]
fn paint_canvas(
    ui: &Ui,
    viewport: Rect,
    image_rect: Rect,
    texture: Option<&egui::TextureHandle>,
    annotations: &[AnnotationVersion],
    selected_annotation: Option<&AnnotationId>,
    editable: bool,
    edit_preview: Option<(&AnnotationId, BoundingBox)>,
    draft_box: Option<BoundingBox>,
    selected_keypoint: Option<usize>,
    keypoint_preview: Option<(&AnnotationId, usize, NormalizedPoint)>,
    skeleton_edges: &[(String, String)],
    prelabels: &[PrelabelSuggestion],
) {
    let painter = ui.painter_at(viewport);
    painter.rect_filled(
        viewport,
        CornerRadius::same(VIEWPORT_CORNER_RADIUS),
        Color32::from_rgb(18, 23, 34),
    );
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let grid = Color32::from_rgba_unmultiplied(255, 255, 255, 18);
    for i in 1..4 {
        let x = image_rect.left() + image_rect.width() * i as f32 / 4.0;
        let y = image_rect.top() + image_rect.height() * i as f32 / 4.0;
        painter.line_segment(
            [pos2(x, image_rect.top()), pos2(x, image_rect.bottom())],
            Stroke::new(1.0, grid),
        );
        painter.line_segment(
            [pos2(image_rect.left(), y), pos2(image_rect.right(), y)],
            Stroke::new(1.0, grid),
        );
    }

    for suggestion in prelabels {
        let color = Color32::from_rgb(192, 132, 252);
        match &suggestion.geometry {
            AnnotationGeometry::BoundingBox(bbox) => {
                let rect = bbox_to_screen_rect(image_rect, *bbox);
                paint_dashed_segment(&painter, rect.left_top(), rect.right_top(), color);
                paint_dashed_segment(&painter, rect.right_top(), rect.right_bottom(), color);
                paint_dashed_segment(&painter, rect.right_bottom(), rect.left_bottom(), color);
                paint_dashed_segment(&painter, rect.left_bottom(), rect.left_top(), color);
            }
            AnnotationGeometry::Skeleton(skeleton) => {
                for keypoint in &skeleton.keypoints {
                    if let Some(point) = keypoint.point {
                        painter.circle_stroke(
                            normalized_to_screen(image_rect, pos2(point.x, point.y)),
                            6.0,
                            Stroke::new(2.0, color),
                        );
                    }
                }
            }
        }
    }

    for annotation in annotations.iter().filter(|annotation| !annotation.deleted) {
        let selected = selected_annotation == Some(&annotation.annotation_id);
        match &annotation.geometry {
            AnnotationGeometry::BoundingBox(bbox) => {
                let bbox = edit_preview
                    .filter(|(id, _)| *id == &annotation.annotation_id)
                    .map_or(*bbox, |(_, preview)| preview);
                if selected {
                    paint_selected_box(&painter, image_rect, bbox, editable);
                } else {
                    paint_existing_box(&painter, image_rect, bbox);
                }
            }
            AnnotationGeometry::Skeleton(skeleton) => {
                let color = if selected {
                    Color32::from_rgb(251, 191, 36)
                } else {
                    Color32::from_rgb(250, 204, 21)
                };
                for (from, to) in skeleton_edges {
                    let from = skeleton
                        .keypoints
                        .iter()
                        .find(|keypoint| &keypoint.name == from)
                        .and_then(|keypoint| keypoint.point);
                    let to = skeleton
                        .keypoints
                        .iter()
                        .find(|keypoint| &keypoint.name == to)
                        .and_then(|keypoint| keypoint.point);
                    if let (Some(from), Some(to)) = (from, to) {
                        painter.line_segment(
                            [
                                normalized_to_screen(image_rect, pos2(from.x, from.y)),
                                normalized_to_screen(image_rect, pos2(to.x, to.y)),
                            ],
                            Stroke::new(if selected { 3.0 } else { 2.0 }, color),
                        );
                    }
                }
                for (keypoint_index, keypoint) in skeleton.keypoints.iter().enumerate() {
                    if let Some(point) = keypoint.point {
                        let point = keypoint_preview
                            .filter(|(id, index, _)| {
                                *id == &annotation.annotation_id && *index == keypoint_index
                            })
                            .map_or(point, |(_, _, preview)| preview);
                        let center = normalized_to_screen(image_rect, pos2(point.x, point.y));
                        if selected {
                            painter.circle_stroke(center, 7.0, Stroke::new(2.0, Color32::WHITE));
                        }
                        painter.circle_filled(center, if selected { 5.0 } else { 4.0 }, color);
                        if selected && selected_keypoint == Some(keypoint_index) {
                            painter.circle_stroke(
                                center,
                                10.0,
                                Stroke::new(3.0, Color32::from_rgb(96, 165, 250)),
                            );
                        }
                    }
                }
            }
        }
    }

    if let Some(bbox) = draft_box {
        paint_draft_box(&painter, image_rect, bbox);
    }
    painter.add(rounded_corner_mask(viewport, ui.visuals().panel_fill));
    painter.rect_stroke(
        viewport,
        CornerRadius::same(VIEWPORT_CORNER_RADIUS),
        Stroke::new(1.0, Color32::from_rgb(70, 82, 105)),
        StrokeKind::Inside,
    );
}

fn rounded_corner_mask(viewport: Rect, color: Color32) -> Mesh {
    let radius = f32::from(VIEWPORT_CORNER_RADIUS)
        .min(viewport.width() * 0.5)
        .min(viewport.height() * 0.5);
    let mut mesh = Mesh::default();
    if radius <= 0.0 {
        return mesh;
    }

    let corners = [
        (
            viewport.left_top(),
            viewport.left_top() + vec2(radius, radius),
            PI,
            PI * 1.5,
        ),
        (
            viewport.right_top(),
            viewport.right_top() + vec2(-radius, radius),
            PI * 1.5,
            PI * 2.0,
        ),
        (
            viewport.right_bottom(),
            viewport.right_bottom() - vec2(radius, radius),
            0.0,
            PI * 0.5,
        ),
        (
            viewport.left_bottom(),
            viewport.left_bottom() + vec2(radius, -radius),
            PI * 0.5,
            PI,
        ),
    ];
    for (outer, center, start, end) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(outer, color);
        for step in 0..=CORNER_MASK_SEGMENTS {
            let angle = start + (end - start) * step as f32 / CORNER_MASK_SEGMENTS as f32;
            mesh.colored_vertex(center + vec2(angle.cos(), angle.sin()) * radius, color);
        }
        for step in 0..CORNER_MASK_SEGMENTS {
            mesh.add_triangle(base, base + step + 1, base + step + 2);
        }
    }
    mesh
}

fn paint_existing_box(painter: &egui::Painter, image_rect: Rect, bbox: BoundingBox) {
    let rect = bbox_to_screen_rect(image_rect, bbox);
    let color = Color32::from_rgb(94, 234, 212);
    painter.rect_filled(
        rect,
        CornerRadius::same(4),
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 14),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.5, color),
        StrokeKind::Inside,
    );
}

fn paint_selected_box(
    painter: &egui::Painter,
    image_rect: Rect,
    bbox: BoundingBox,
    editable: bool,
) {
    let rect = bbox_to_screen_rect(image_rect, bbox);
    let color = Color32::from_rgb(251, 191, 36);
    painter.rect_filled(
        rect,
        CornerRadius::same(5),
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(3.0, color),
        StrokeKind::Inside,
    );

    if editable {
        for (_, center) in resize_handles(rect) {
            let handle = Rect::from_center_size(center, Vec2::splat(HANDLE_SIZE));
            painter.rect_filled(handle, CornerRadius::same(2), Color32::WHITE);
            painter.rect_stroke(
                handle,
                CornerRadius::same(2),
                Stroke::new(1.5, color),
                StrokeKind::Inside,
            );
        }
    }
}

fn paint_draft_box(painter: &egui::Painter, image_rect: Rect, bbox: BoundingBox) {
    let rect = bbox_to_screen_rect(image_rect, bbox);
    let color = Color32::from_rgb(96, 165, 250);
    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 30),
    );
    paint_dashed_segment(painter, rect.left_top(), rect.right_top(), color);
    paint_dashed_segment(painter, rect.right_top(), rect.right_bottom(), color);
    paint_dashed_segment(painter, rect.right_bottom(), rect.left_bottom(), color);
    paint_dashed_segment(painter, rect.left_bottom(), rect.left_top(), color);
}

fn paint_dashed_segment(painter: &egui::Painter, start: Pos2, end: Pos2, color: Color32) {
    if !start.x.is_finite() || !start.y.is_finite() || !end.x.is_finite() || !end.y.is_finite() {
        return;
    }
    let vector = end - start;
    let length = vector.length();
    if !length.is_finite() || length <= f32::EPSILON {
        return;
    }
    let direction = vector / length;
    let mut offset = 0.0;
    for _ in 0..MAX_DASH_SEGMENTS {
        if offset >= length {
            break;
        }
        let dash_end = (offset + 6.0).min(length);
        if !dash_end.is_finite() || dash_end <= offset {
            break;
        }
        painter.line_segment(
            [start + direction * offset, start + direction * dash_end],
            Stroke::new(2.0, color),
        );
        let next = offset + 10.0;
        if !next.is_finite() || next <= offset {
            break;
        }
        offset = next;
    }
}

fn handle_view_gestures(
    ui: &Ui,
    response: &Response,
    viewport: Rect,
    fitted_image: Rect,
    interaction_rect: Rect,
    state: &mut CanvasState,
) -> bool {
    let pointer = response.hover_pos();
    let multi_touch = ui.input(|input| input.multi_touch());
    if let Some(touch) = multi_touch.filter(|touch| interaction_rect.contains(touch.center_pos)) {
        // Scale around the previous touch center, then apply this frame's translation.
        // Using the new center for both operations applies translation twice as visible drift.
        let previous_center = touch.center_pos - touch.translation_delta;
        set_zoom_around(
            state,
            viewport,
            fitted_image,
            previous_center,
            state.zoom * touch.zoom_delta,
        );
        state.pan += touch.translation_delta;
        state.clamp_to_viewport(viewport, fitted_image);
        state.cancel_drag();
        state.space_pan = false;
        return true;
    }

    let (space_down, primary_down, primary_pressed, primary_released, pointer_delta) =
        ui.input(|input| {
            (
                input.key_down(Key::Space),
                input.pointer.primary_down(),
                input.pointer.primary_pressed(),
                input.pointer.primary_released(),
                input.pointer.delta(),
            )
        });
    if space_down && primary_pressed && response.is_pointer_button_down_on() {
        state.space_pan = true;
    }
    if state.space_pan {
        if primary_down {
            state.pan += pointer_delta;
            state.clamp_to_viewport(viewport, fitted_image);
        }
        state.cancel_drag();
        if primary_released {
            state.space_pan = false;
        }
        return true;
    }

    if response.dragged_by(PointerButton::Middle) {
        state.pan += response.drag_delta();
        state.clamp_to_viewport(viewport, fitted_image);
        state.cancel_drag();
        return true;
    }

    if let Some(pointer) = pointer {
        let (zoom_delta, wheel_delta) =
            ui.input(|input| (input.zoom_delta(), input.smooth_scroll_delta()));
        if (zoom_delta - 1.0).abs() > f32::EPSILON {
            set_zoom_around(
                state,
                viewport,
                fitted_image,
                pointer,
                state.zoom * zoom_delta,
            );
            state.cancel_drag();
            return true;
        }
        if wheel_delta != Vec2::ZERO {
            set_zoom_around(
                state,
                viewport,
                fitted_image,
                pointer,
                state.zoom * wheel_zoom_factor(wheel_delta),
            );
            state.cancel_drag();
            return true;
        }
    }

    let repeated_click = if response.clicked() {
        let now = ui.input(|input| input.time);
        let position = response.interact_pointer_pos();
        let repeated = position.is_some_and(|position| {
            state
                .last_canvas_click
                .is_some_and(|(last_time, last_position)| {
                    now - last_time <= DOUBLE_CLICK_DELAY
                        && position.distance(last_position) <= DOUBLE_CLICK_DISTANCE
                })
        });
        state.last_canvas_click = position.map(|position| (now, position));
        repeated
    } else {
        false
    };
    if response.double_clicked() || repeated_click {
        state.fit_view();
        state.cancel_drag();
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn handle_annotation_pointer(
    ui: &Ui,
    response: Response,
    interaction_rect: Rect,
    image_rect: Rect,
    state: &mut CanvasState,
    annotations: &[AnnotationVersion],
    bounding_box_tool: bool,
    selected_annotation: Option<&AnnotationId>,
    interaction: CanvasInteraction,
    view_consumed: bool,
) -> Option<CanvasAction<BoundingBoxEdit>> {
    if view_consumed || ui.input(|input| input.multi_touch().is_some()) {
        state.cancel_drag();
        return None;
    }

    let pointer = response.interact_pointer_pos();
    let (primary_pressed, primary_released, primary_down, pointer_available, cancelled) =
        ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_released(),
                input.pointer.primary_down(),
                input.pointer.latest_pos().is_some(),
                input.key_pressed(Key::Escape),
            )
        });
    if !interaction.editable || cancelled {
        state.cancel_drag();
        return None;
    }
    if state.is_dragging() && ((!primary_down && !primary_released) || !pointer_available) {
        state.cancel_drag();
        return None;
    }
    if interaction.editable
        && primary_pressed
        && state.drag.is_none()
        && let Some(pointer) = pointer
        && interaction_rect.contains(pointer)
    {
        let normalized_pointer = screen_to_normalized(image_rect, pointer);
        let selected_box = selected_annotation.and_then(|id| annotation_bbox(id, annotations));
        if let (Some(id), Some(bbox)) = (selected_annotation, selected_box) {
            let box_rect = bbox_to_screen_rect(image_rect, bbox);
            if let Some(handle) = resize_handle_at(pointer, box_rect) {
                state.drag = Some(DragOperation::Resize {
                    annotation_id: id.clone(),
                    original: bbox,
                    handle,
                });
                state.draft_box = Some(bbox);
            } else if box_rect.contains(pointer) {
                state.drag = Some(DragOperation::Move {
                    annotation_id: id.clone(),
                    original: bbox,
                    start: normalized_pointer,
                });
                state.draft_box = Some(bbox);
            }
        }

        if state.drag.is_none()
            && interaction.edit_keypoints
            && let Some(annotation_id) = selected_annotation
            && let Some((keypoint_index, point)) =
                keypoint_at(pointer, image_rect, annotation_id, annotations)
        {
            state.drag = Some(DragOperation::Keypoint {
                annotation_id: annotation_id.clone(),
                keypoint_index,
                original: point,
            });
            state.draft_keypoint = Some(point);
        }

        if state.drag.is_none()
            && interaction.allow_create
            && bounding_box_tool
            && image_rect.contains(pointer)
            && annotation_at(pointer, image_rect, annotations).is_none()
        {
            state.drag = Some(DragOperation::Create {
                start: normalized_pointer,
            });
            state.draft_box = None;
        }
    }

    if interaction.editable
        && state.drag.is_some()
        && let Some(pointer) = pointer
    {
        update_drag_preview(state, image_rect, pointer);
    }

    if primary_released {
        let drag = state.drag.take();
        let bbox = state.draft_box.take();
        let keypoint = state.draft_keypoint.take();
        match (drag, bbox, keypoint) {
            (Some(DragOperation::Create { .. }), Some(bbox), _)
                if bbox.width > 0.005 && bbox.height > 0.005 =>
            {
                return Some(CanvasAction::CreateBoundingBox(bbox));
            }
            (
                Some(DragOperation::Move {
                    annotation_id,
                    original,
                    ..
                })
                | Some(DragOperation::Resize {
                    annotation_id,
                    original,
                    ..
                }),
                Some(bounding_box),
                _,
            ) if bbox_changed(original, bounding_box) => {
                return Some(CanvasAction::EditBoundingBox(BoundingBoxEdit {
                    annotation_id,
                    bounding_box,
                }));
            }
            (
                Some(DragOperation::Keypoint {
                    annotation_id,
                    keypoint_index,
                    original,
                }),
                _,
                Some(point),
            ) => {
                if point != original {
                    return Some(CanvasAction::EditKeypoint(KeypointEdit {
                        annotation_id,
                        keypoint_index,
                        point,
                    }));
                }
                return Some(CanvasAction::SelectKeypoint(KeypointSelection {
                    annotation_id,
                    keypoint_index,
                }));
            }
            _ => {}
        }
    }

    if response.clicked()
        && let Some(pointer) = pointer
        && interaction_rect.contains(pointer)
    {
        if interaction.allow_selection
            && let Some(annotation) = annotation_at(pointer, image_rect, annotations)
        {
            return Some(CanvasAction::Select(annotation.annotation_id.clone()));
        }
        if interaction.allow_create && !bounding_box_tool && image_rect.contains(pointer) {
            let point = screen_to_normalized(image_rect, pointer);
            return Some(CanvasAction::PlaceKeypoint(NormalizedPoint {
                x: point.x.clamp(0.0, 1.0),
                y: point.y.clamp(0.0, 1.0),
            }));
        }
    }
    None
}

fn update_drag_preview(state: &mut CanvasState, image_rect: Rect, pointer: Pos2) {
    let Some(drag) = &state.drag else {
        return;
    };
    let current = screen_to_normalized(image_rect, pointer);
    state.draft_box = match drag {
        DragOperation::Create { start } => bbox_from_normalized_points(*start, current),
        DragOperation::Move {
            original, start, ..
        } => Some(move_bbox(*original, current - *start)),
        DragOperation::Resize {
            original, handle, ..
        } => Some(resize_bbox(*original, *handle, current)),
        DragOperation::Keypoint { .. } => {
            state.draft_keypoint = Some(NormalizedPoint {
                x: current.x.clamp(0.0, 1.0),
                y: current.y.clamp(0.0, 1.0),
            });
            None
        }
    };
}

fn keypoint_at(
    pos: Pos2,
    image_rect: Rect,
    annotation_id: &AnnotationId,
    annotations: &[AnnotationVersion],
) -> Option<(usize, NormalizedPoint)> {
    let annotation = annotations
        .iter()
        .rev()
        .find(|annotation| !annotation.deleted && &annotation.annotation_id == annotation_id)?;
    let AnnotationGeometry::Skeleton(skeleton) = &annotation.geometry else {
        return None;
    };
    skeleton
        .keypoints
        .iter()
        .enumerate()
        .filter_map(|(index, keypoint)| keypoint.point.map(|point| (index, point)))
        .find(|(_, point)| {
            normalized_to_screen(image_rect, pos2(point.x, point.y)).distance(pos) <= 12.0
        })
}

fn annotation_bbox(id: &AnnotationId, annotations: &[AnnotationVersion]) -> Option<BoundingBox> {
    annotations.iter().rev().find_map(|annotation| {
        if !annotation.deleted && &annotation.annotation_id == id {
            match annotation.geometry {
                AnnotationGeometry::BoundingBox(bbox) => Some(clamp_bbox(bbox)),
                AnnotationGeometry::Skeleton(_) => None,
            }
        } else {
            None
        }
    })
}

fn annotation_at(
    pos: Pos2,
    image_rect: Rect,
    annotations: &[AnnotationVersion],
) -> Option<&AnnotationVersion> {
    annotations
        .iter()
        .rev()
        .filter(|annotation| !annotation.deleted)
        .find(|annotation| match annotation.geometry {
            AnnotationGeometry::BoundingBox(bbox) => {
                bbox_to_screen_rect(image_rect, bbox).contains(pos)
            }
            AnnotationGeometry::Skeleton(ref skeleton) => {
                skeleton.keypoints.iter().any(|keypoint| {
                    keypoint.point.is_some_and(|point| {
                        normalized_to_screen(image_rect, pos2(point.x, point.y)).distance(pos)
                            <= 12.0
                    })
                })
            }
        })
}

fn fitted_image_rect(viewport: Rect, image_size: [u32; 2]) -> Rect {
    if !valid_rect(viewport) {
        return Rect::from_center_size(viewport.center(), Vec2::ZERO);
    }
    let source = vec2(image_size[0].max(1) as f32, image_size[1].max(1) as f32);
    let scale = (viewport.width() / source.x).min(viewport.height() / source.y);
    Rect::from_center_size(viewport.center(), source * scale)
}

fn transformed_image_rect(fitted_image: Rect, zoom: f32, pan: Vec2) -> Rect {
    if !valid_rect(fitted_image) {
        return Rect::from_center_size(fitted_image.center(), Vec2::ZERO);
    }
    Rect::from_center_size(
        fitted_image.center() + vec2(finite_or(pan.x, 0.0), finite_or(pan.y, 0.0)),
        fitted_image.size() * finite_or(zoom, MIN_ZOOM).clamp(MIN_ZOOM, MAX_ZOOM),
    )
}

fn normalized_to_screen(image_rect: Rect, point: Pos2) -> Pos2 {
    pos2(
        image_rect.left() + point.x * image_rect.width(),
        image_rect.top() + point.y * image_rect.height(),
    )
}

fn screen_to_normalized(image_rect: Rect, point: Pos2) -> Pos2 {
    pos2(
        if image_rect.width().is_finite() && image_rect.width() > f32::EPSILON {
            (point.x - image_rect.left()) / image_rect.width()
        } else {
            0.5
        },
        if image_rect.height().is_finite() && image_rect.height() > f32::EPSILON {
            (point.y - image_rect.top()) / image_rect.height()
        } else {
            0.5
        },
    )
}

fn bbox_to_screen_rect(image_rect: Rect, bbox: BoundingBox) -> Rect {
    let bbox = clamp_bbox(bbox);
    Rect::from_min_max(
        normalized_to_screen(image_rect, pos2(bbox.x, bbox.y)),
        normalized_to_screen(image_rect, pos2(bbox.x + bbox.width, bbox.y + bbox.height)),
    )
}

fn annotation_focus_rect(annotation: &AnnotationVersion) -> Option<Rect> {
    match &annotation.geometry {
        AnnotationGeometry::BoundingBox(bbox) => {
            let bbox = clamp_bbox(*bbox);
            Some(Rect::from_min_max(
                pos2(bbox.x, bbox.y),
                pos2(bbox.x + bbox.width, bbox.y + bbox.height),
            ))
        }
        AnnotationGeometry::Skeleton(skeleton) => {
            let mut points = skeleton
                .keypoints
                .iter()
                .filter_map(|keypoint| keypoint.point)
                .map(|point| pos2(point.x, point.y));
            let first = points.next()?;
            Some(
                points.fold(Rect::from_min_max(first, first), |rect, point| {
                    rect.union(Rect::from_min_max(point, point))
                }),
            )
        }
    }
}

fn fit_normalized_rect(state: &mut CanvasState, viewport: Rect, fitted_image: Rect, target: Rect) {
    if !valid_rect(viewport) || !valid_rect(fitted_image) {
        state.cancel_drag();
        return;
    }
    let target_size = target
        .size()
        .max(Vec2::splat(MIN_FOCUS_SPAN))
        .min(Vec2::splat(1.0));
    let zoom = (viewport.width() / (fitted_image.width() * target_size.x * FOCUS_MARGIN))
        .min(viewport.height() / (fitted_image.height() * target_size.y * FOCUS_MARGIN))
        .clamp(MIN_ZOOM, MAX_ZOOM);
    let target_center = pos2(
        finite_or(target.center().x, 0.5).clamp(0.0, 1.0),
        finite_or(target.center().y, 0.5).clamp(0.0, 1.0),
    );
    state.zoom = zoom;
    state.pan = viewport.center()
        - normalized_to_screen(
            Rect::from_center_size(fitted_image.center(), fitted_image.size() * zoom),
            target_center,
        );
    state.clamp_to_viewport(viewport, fitted_image);
}

fn set_zoom_around(
    state: &mut CanvasState,
    viewport: Rect,
    fitted_image: Rect,
    focus: Pos2,
    requested_zoom: f32,
) {
    if !valid_rect(viewport) || !valid_rect(fitted_image) {
        state.cancel_drag();
        return;
    }
    state.clamp_to_viewport(viewport, fitted_image);
    let old_zoom = state.zoom;
    let old_center = fitted_image.center() + state.pan;
    let zoom = finite_or(requested_zoom, state.zoom).clamp(MIN_ZOOM, MAX_ZOOM);
    if zoom == old_zoom {
        return;
    }
    state.zoom = zoom;
    state.pan = focus - fitted_image.center() - (focus - old_center) * (zoom / old_zoom);
    state.pan = clamp_pan(viewport, fitted_image, state.zoom, state.pan);
}

fn clamp_pan(viewport: Rect, fitted_image: Rect, zoom: f32, pan: Vec2) -> Vec2 {
    if !valid_rect(viewport) || !valid_rect(fitted_image) {
        return Vec2::ZERO;
    }
    let overflow = fitted_image.size() * ((zoom.max(MIN_ZOOM) - MIN_ZOOM) * 0.5);
    vec2(
        finite_or(pan.x, 0.0).clamp(-overflow.x, overflow.x),
        finite_or(pan.y, 0.0).clamp(-overflow.y, overflow.y),
    )
}

fn valid_rect(rect: Rect) -> bool {
    rect.min.x.is_finite()
        && rect.min.y.is_finite()
        && rect.max.x.is_finite()
        && rect.max.y.is_finite()
        && rect.width() > f32::EPSILON
        && rect.height() > f32::EPSILON
}

fn wheel_zoom_factor(delta: Vec2) -> f32 {
    let points = if delta.y.abs() >= delta.x.abs() {
        delta.y
    } else {
        delta.x
    };
    (points.clamp(-500.0, 500.0) * 0.005).exp()
}

fn bbox_from_normalized_points(a: Pos2, b: Pos2) -> Option<BoundingBox> {
    let min = pos2(a.x.min(b.x).clamp(0.0, 1.0), a.y.min(b.y).clamp(0.0, 1.0));
    let max = pos2(a.x.max(b.x).clamp(0.0, 1.0), a.y.max(b.y).clamp(0.0, 1.0));
    let bbox = BoundingBox {
        x: min.x,
        y: min.y,
        width: max.x - min.x,
        height: max.y - min.y,
    };
    (bbox.width > 0.0 && bbox.height > 0.0).then_some(bbox)
}

fn clamp_bbox(bbox: BoundingBox) -> BoundingBox {
    let x1 = finite_or(bbox.x, 0.0).clamp(0.0, 1.0 - MIN_BOX_SIZE);
    let y1 = finite_or(bbox.y, 0.0).clamp(0.0, 1.0 - MIN_BOX_SIZE);
    let x2 = finite_or(bbox.x + bbox.width, x1 + MIN_BOX_SIZE).clamp(x1 + MIN_BOX_SIZE, 1.0);
    let y2 = finite_or(bbox.y + bbox.height, y1 + MIN_BOX_SIZE).clamp(y1 + MIN_BOX_SIZE, 1.0);
    BoundingBox {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn move_bbox(bbox: BoundingBox, delta: Vec2) -> BoundingBox {
    let bbox = clamp_bbox(bbox);
    BoundingBox {
        x: (bbox.x + finite_or(delta.x, 0.0)).clamp(0.0, 1.0 - bbox.width),
        y: (bbox.y + finite_or(delta.y, 0.0)).clamp(0.0, 1.0 - bbox.height),
        ..bbox
    }
}

fn bbox_changed(a: BoundingBox, b: BoundingBox) -> bool {
    (a.x - b.x).abs() > f32::EPSILON
        || (a.y - b.y).abs() > f32::EPSILON
        || (a.width - b.width).abs() > f32::EPSILON
        || (a.height - b.height).abs() > f32::EPSILON
}

fn resize_bbox(bbox: BoundingBox, handle: ResizeHandle, pointer: Pos2) -> BoundingBox {
    let bbox = clamp_bbox(bbox);
    let mut left = bbox.x;
    let mut top = bbox.y;
    let mut right = bbox.x + bbox.width;
    let mut bottom = bbox.y + bbox.height;
    let pointer = pos2(finite_or(pointer.x, left), finite_or(pointer.y, top));

    if matches!(
        handle,
        ResizeHandle::TopLeft | ResizeHandle::Left | ResizeHandle::BottomLeft
    ) {
        left = pointer.x.clamp(0.0, right - MIN_BOX_SIZE);
    }
    if matches!(
        handle,
        ResizeHandle::TopRight | ResizeHandle::Right | ResizeHandle::BottomRight
    ) {
        right = pointer.x.clamp(left + MIN_BOX_SIZE, 1.0);
    }
    if matches!(
        handle,
        ResizeHandle::TopLeft | ResizeHandle::Top | ResizeHandle::TopRight
    ) {
        top = pointer.y.clamp(0.0, bottom - MIN_BOX_SIZE);
    }
    if matches!(
        handle,
        ResizeHandle::BottomLeft | ResizeHandle::Bottom | ResizeHandle::BottomRight
    ) {
        bottom = pointer.y.clamp(top + MIN_BOX_SIZE, 1.0);
    }

    BoundingBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn resize_handles(rect: Rect) -> [(ResizeHandle, Pos2); 8] {
    [
        (ResizeHandle::TopLeft, rect.left_top()),
        (ResizeHandle::Top, pos2(rect.center().x, rect.top())),
        (ResizeHandle::TopRight, rect.right_top()),
        (ResizeHandle::Right, pos2(rect.right(), rect.center().y)),
        (ResizeHandle::BottomRight, rect.right_bottom()),
        (ResizeHandle::Bottom, pos2(rect.center().x, rect.bottom())),
        (ResizeHandle::BottomLeft, rect.left_bottom()),
        (ResizeHandle::Left, pos2(rect.left(), rect.center().y)),
    ]
}

fn resize_handle_at(pointer: Pos2, rect: Rect) -> Option<ResizeHandle> {
    resize_handles(rect)
        .into_iter()
        .find_map(|(handle, center)| {
            (center.distance(pointer) <= HANDLE_HIT_RADIUS).then_some(handle)
        })
}

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
            task_id: labello_domain::TaskId::from("task"),
            class_id: labello_domain::ClassId::from("class"),
            annotation_type: match &geometry {
                AnnotationGeometry::BoundingBox(_) => labello_domain::AnnotationType::BoundingBox,
                AnnotationGeometry::Skeleton(_) => labello_domain::AnnotationType::Skeleton,
            },
            source: labello_domain::AnnotationSource::Human,
            geometry,
            author_user_id: labello_domain::UserId::from("annotator"),
            created_at: labello_domain::now(),
            updated_at: labello_domain::now(),
            deleted: false,
        }
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
        harness.event(Event::PointerMoved(start));
        harness.event(Event::PointerButton {
            pos: start,
            button,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.event(Event::PointerMoved(end));
        harness.event(Event::PointerButton {
            pos: end,
            button,
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
    fn middle_and_space_primary_drags_pan_without_annotating() {
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
        harness.key_down(Key::Space);
        drag_at(
            &mut harness,
            PointerButton::Primary,
            center,
            center - vec2(30.0, 10.0),
        );
        harness.key_up(Key::Space);
        harness.step();
        assert!(harness.state().canvas.pan.x < pan_before.x);
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

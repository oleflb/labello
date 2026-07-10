use std::convert::Infallible;

use egui::{
    Button, Color32, CornerRadius, PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use labello_domain::{
    AnnotationGeometry, AnnotationId, AnnotationVersion, BoundingBox, NormalizedPoint,
    PrelabelSuggestion,
};

const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 12.0;
const MIN_BOX_SIZE: f32 = 0.001;
const HANDLE_HIT_RADIUS: f32 = 12.0;
const HANDLE_SIZE: f32 = 8.0;

/// The result of moving or resizing an existing bounding-box annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundingBoxEdit {
    pub annotation_id: AnnotationId,
    pub bounding_box: BoundingBox,
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
}

#[derive(Clone, Debug)]
pub struct CanvasState {
    drag: Option<DragOperation>,
    draft_box: Option<BoundingBox>,
    zoom: f32,
    pan: Vec2,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            drag: None,
            draft_box: None,
            zoom: MIN_ZOOM,
            pan: Vec2::ZERO,
        }
    }
}

impl CanvasState {
    /// Return the current zoom factor relative to a fitted image.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Fit the image to the canvas and center it.
    pub fn fit_view(&mut self) {
        self.zoom = MIN_ZOOM;
        self.pan = Vec2::ZERO;
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
        Some(CanvasAction::EditBoundingBox(_)) | None => None,
    }
}

/// Show a canvas with selection, box editing, and read-only support.
///
/// Mouse users can pan with the middle button and zoom over the pointer with
/// ctrl-scroll. Touch users can pan and pinch with two fingers. The `Fit`
/// button and a double-click on empty canvas reset the viewport. A single
/// pointer or stylus moves a selected box, resizes it from any of its eight
/// handles, or creates a box when the bounding-box tool is active.
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
    // Never allocate beyond the viewport. Compact tablet layouts place controls
    // in drawers, leaving the canvas whatever space remains.
    let available = ui.available_size().max(vec2(1.0, 1.0));
    let image_ratio = image_size[0].max(1) as f32 / image_size[1].max(1) as f32;
    let mut canvas_size = available;
    if canvas_size.x / canvas_size.y > image_ratio {
        canvas_size.x = canvas_size.y * image_ratio;
    } else {
        canvas_size.y = canvas_size.x / image_ratio;
    }

    let (viewport, response) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Annotation canvas"));

    let fit_rect = Rect::from_min_size(viewport.min + vec2(10.0, 10.0), vec2(42.0, 28.0));
    handle_view_gestures(ui, &response, viewport, fit_rect, state);
    let image_rect = transformed_image_rect(viewport, state.zoom, state.pan);

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
        skeleton_edges,
        prelabels,
    );

    // Paint controls last so zoomed image content cannot obscure them.
    let fit_clicked = ui
        .put(fit_rect, Button::new("Fit").corner_radius(6))
        .on_hover_text("Fit image to canvas (double-click also resets the view)")
        .clicked();
    if fit_clicked {
        state.fit_view();
        return None;
    }
    handle_annotation_pointer(
        ui,
        response,
        viewport,
        fit_rect,
        image_rect,
        state,
        annotations,
        bounding_box_tool,
        selected_annotation,
        editable,
    )
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
    skeleton_edges: &[(String, String)],
    prelabels: &[PrelabelSuggestion],
) {
    let painter = ui.painter_at(viewport);
    painter.rect_filled(
        viewport,
        CornerRadius::same(18),
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
                for keypoint in &skeleton.keypoints {
                    if let Some(point) = keypoint.point {
                        let center = normalized_to_screen(image_rect, pos2(point.x, point.y));
                        if selected {
                            painter.circle_stroke(center, 7.0, Stroke::new(2.0, Color32::WHITE));
                        }
                        painter.circle_filled(center, if selected { 5.0 } else { 4.0 }, color);
                    }
                }
            }
        }
    }

    if let Some(bbox) = draft_box {
        paint_draft_box(&painter, image_rect, bbox);
    }
    painter.rect_stroke(
        viewport,
        CornerRadius::same(18),
        Stroke::new(1.0, Color32::from_rgb(70, 82, 105)),
        StrokeKind::Inside,
    );
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
    let vector = end - start;
    let length = vector.length();
    if length <= f32::EPSILON {
        return;
    }
    let direction = vector / length;
    let mut offset = 0.0;
    while offset < length {
        let dash_end = (offset + 6.0).min(length);
        painter.line_segment(
            [start + direction * offset, start + direction * dash_end],
            Stroke::new(2.0, color),
        );
        offset += 10.0;
    }
}

fn handle_view_gestures(
    ui: &Ui,
    response: &Response,
    viewport: Rect,
    fit_rect: Rect,
    state: &mut CanvasState,
) {
    let pointer = response.hover_pos();
    let multi_touch = ui.input(|input| input.multi_touch());
    if let Some(touch) = multi_touch.filter(|touch| viewport.contains(touch.center_pos)) {
        set_zoom_around(
            state,
            viewport,
            touch.center_pos,
            state.zoom * touch.zoom_delta,
        );
        state.pan += touch.translation_delta;
        state.pan = clamp_pan(viewport, state.zoom, state.pan);
        // A second finger always means viewport manipulation, never annotation editing.
        state.drag = None;
        state.draft_box = None;
        return;
    }

    if response.dragged_by(PointerButton::Middle) {
        state.pan += response.drag_delta();
        state.pan = clamp_pan(viewport, state.zoom, state.pan);
    }

    if let Some(pointer) = pointer.filter(|pointer| !fit_rect.contains(*pointer)) {
        let zoom_delta = ui.input(|input| input.zoom_delta());
        if (zoom_delta - 1.0).abs() > f32::EPSILON {
            set_zoom_around(state, viewport, pointer, state.zoom * zoom_delta);
        } else {
            let pan_delta = ui.input(|input| input.smooth_scroll_delta());
            if pan_delta != Vec2::ZERO && state.zoom > MIN_ZOOM {
                state.pan += pan_delta;
                state.pan = clamp_pan(viewport, state.zoom, state.pan);
            }
        }
    }

    if response.double_clicked() && pointer.is_some_and(|pointer| !fit_rect.contains(pointer)) {
        state.fit_view();
        state.drag = None;
        state.draft_box = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_annotation_pointer(
    ui: &Ui,
    response: Response,
    viewport: Rect,
    fit_rect: Rect,
    image_rect: Rect,
    state: &mut CanvasState,
    annotations: &[AnnotationVersion],
    bounding_box_tool: bool,
    selected_annotation: Option<&AnnotationId>,
    editable: bool,
) -> Option<CanvasAction<BoundingBoxEdit>> {
    if ui.input(|input| input.multi_touch().is_some()) || response.dragged_by(PointerButton::Middle)
    {
        return None;
    }

    let pointer = response.interact_pointer_pos();
    let primary_pressed = ui.input(|input| input.pointer.primary_pressed());
    if editable
        && primary_pressed
        && state.drag.is_none()
        && let Some(pointer) = pointer
        && viewport.contains(pointer)
        && !fit_rect.contains(pointer)
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
            && bounding_box_tool
            && annotation_at(pointer, image_rect, annotations).is_none()
        {
            state.drag = Some(DragOperation::Create {
                start: normalized_pointer,
            });
            state.draft_box = None;
        }
    }

    if editable
        && state.drag.is_some()
        && let Some(pointer) = pointer
    {
        update_drag_preview(state, image_rect, pointer);
    }

    let primary_released = ui.input(|input| input.pointer.primary_released());
    if editable && primary_released {
        let drag = state.drag.take();
        let bbox = state.draft_box.take();
        match (drag, bbox) {
            (Some(DragOperation::Create { .. }), Some(bbox))
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
            ) if bbox_changed(original, bounding_box) => {
                return Some(CanvasAction::EditBoundingBox(BoundingBoxEdit {
                    annotation_id,
                    bounding_box,
                }));
            }
            _ => {}
        }
    }

    if response.clicked()
        && let Some(pointer) = pointer
        && !fit_rect.contains(pointer)
    {
        if let Some(annotation) = annotation_at(pointer, image_rect, annotations) {
            return Some(CanvasAction::Select(annotation.annotation_id.clone()));
        }
        if editable && !bounding_box_tool && image_rect.contains(pointer) {
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
    };
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

fn transformed_image_rect(viewport: Rect, zoom: f32, pan: Vec2) -> Rect {
    Rect::from_center_size(viewport.center() + pan, viewport.size() * zoom)
}

fn normalized_to_screen(image_rect: Rect, point: Pos2) -> Pos2 {
    pos2(
        image_rect.left() + point.x * image_rect.width(),
        image_rect.top() + point.y * image_rect.height(),
    )
}

fn screen_to_normalized(image_rect: Rect, point: Pos2) -> Pos2 {
    pos2(
        (point.x - image_rect.left()) / image_rect.width(),
        (point.y - image_rect.top()) / image_rect.height(),
    )
}

fn bbox_to_screen_rect(image_rect: Rect, bbox: BoundingBox) -> Rect {
    let bbox = clamp_bbox(bbox);
    Rect::from_min_max(
        normalized_to_screen(image_rect, pos2(bbox.x, bbox.y)),
        normalized_to_screen(image_rect, pos2(bbox.x + bbox.width, bbox.y + bbox.height)),
    )
}

fn set_zoom_around(state: &mut CanvasState, viewport: Rect, focus: Pos2, requested_zoom: f32) {
    let old_rect = transformed_image_rect(viewport, state.zoom, state.pan);
    let anchor = screen_to_normalized(old_rect, focus);
    let zoom = requested_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    let centered = Rect::from_center_size(viewport.center(), viewport.size() * zoom);
    let desired_min = focus - (viewport.size() * zoom * anchor.to_vec2());
    state.zoom = zoom;
    state.pan = desired_min - centered.min;
    state.pan = clamp_pan(viewport, state.zoom, state.pan);
}

fn clamp_pan(viewport: Rect, zoom: f32, pan: Vec2) -> Vec2 {
    let limit = viewport.size() * ((zoom.max(MIN_ZOOM) - 1.0) * 0.5);
    vec2(
        pan.x.clamp(-limit.x, limit.x),
        pan.y.clamp(-limit.y, limit.y),
    )
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
    fn zoom_is_anchored_under_pointer_and_pan_is_clamped() {
        let viewport = Rect::from_min_size(pos2(100.0, 200.0), vec2(400.0, 200.0));
        let focus = pos2(200.0, 250.0);
        let mut state = CanvasState::default();
        let before = screen_to_normalized(
            transformed_image_rect(viewport, state.zoom, state.pan),
            focus,
        );
        set_zoom_around(&mut state, viewport, focus, 2.0);
        let after = screen_to_normalized(
            transformed_image_rect(viewport, state.zoom, state.pan),
            focus,
        );
        assert!((before.x - after.x).abs() < 0.000_01);
        assert!((before.y - after.y).abs() < 0.000_01);
        assert_eq!(
            clamp_pan(viewport, 2.0, vec2(999.0, -999.0)),
            vec2(200.0, -100.0)
        );

        state.fit_view();
        assert_eq!(state.zoom(), 1.0);
        assert_eq!(state.pan, Vec2::ZERO);
    }
}

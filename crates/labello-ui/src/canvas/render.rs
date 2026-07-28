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
#[allow(
    clippy::too_many_arguments,
    reason = "the public canvas adapter keeps each independent rendering input explicit"
)]
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
#[allow(
    clippy::too_many_arguments,
    reason = "the configured canvas adapter keeps each independent rendering input explicit"
)]
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
    show_canvas_styled(
        ui,
        state,
        texture,
        annotations,
        image_size,
        bounding_box_tool,
        selected_annotation,
        interaction,
        skeleton_edges,
        prelabels,
        theme::ANNOTATION,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the styled canvas adapter keeps rendering policy explicit at its caller"
)]
pub(crate) fn show_canvas_styled(
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
    annotation_color: Color32,
) -> Option<CanvasAction<BoundingBoxEdit>> {
    show_canvas_colored(
        ui,
        state,
        texture,
        annotations,
        image_size,
        bounding_box_tool,
        selected_annotation,
        interaction,
        skeleton_edges,
        prelabels,
        annotation_color,
        &std::collections::BTreeMap::new(),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the colored canvas adapter keeps per-object styling explicit at its caller"
)]
pub(crate) fn show_canvas_colored(
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
    annotation_color: Color32,
    annotation_colors: &std::collections::BTreeMap<AnnotationId, Color32>,
) -> Option<CanvasAction<BoundingBoxEdit>> {
    let editable = interaction.editable;
    let available = ui.available_size().max(vec2(1.0, 1.0));
    let (viewport, _) = ui.allocate_exact_size(available, Sense::hover());
    let interaction_rect = viewport;
    if !editable {
        state.cancel_drag();
    }
    let mut response = ui.interact(
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
            annotation_color,
            annotation_colors,
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
    let (primary_down, middle_down) = ui.input(|input| {
        (
            input.pointer.primary_down(),
            input.pointer.button_down(PointerButton::Middle),
        )
    });
    if let Some(cursor) = canvas_hover_cursor(
        response.hover_pos(),
        image_rect,
        state,
        annotations,
        selected_annotation,
        interaction,
        bounding_box_tool,
        primary_down,
        middle_down,
    ) {
        response = response.on_hover_cursor(cursor);
    }

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
        annotation_color,
        annotation_colors,
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

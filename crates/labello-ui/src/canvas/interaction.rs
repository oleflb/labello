#[allow(
    clippy::too_many_arguments,
    reason = "cursor selection combines independent interaction facts without owning them"
)]
fn canvas_hover_cursor(
    pointer: Option<Pos2>,
    image_rect: Rect,
    state: &CanvasState,
    annotations: &[AnnotationVersion],
    selected_annotation: Option<&AnnotationId>,
    interaction: CanvasInteraction,
    bounding_box_tool: bool,
    selectable_annotations: Option<&std::collections::BTreeSet<AnnotationId>>,
    primary_down: bool,
    middle_down: bool,
) -> Option<egui::CursorIcon> {
    if middle_down {
        return Some(egui::CursorIcon::Grabbing);
    }
    if state.pan_mode() || state.space_pan || state.primary_pan {
        return Some(if primary_down {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }
    if !interaction.editable {
        return None;
    }
    if let Some(drag) = state.drag.as_ref() {
        return Some(match drag {
            DragOperation::Resize { handle, .. } => resize_cursor(*handle),
            DragOperation::Move { .. } | DragOperation::Keypoint { .. } => {
                egui::CursorIcon::Grabbing
            }
            DragOperation::Create { .. } => egui::CursorIcon::Crosshair,
        });
    }

    let pointer = pointer?;
    if let Some(annotation_id) = selected_annotation {
        if let Some(bbox) = annotation_bbox(annotation_id, annotations) {
            let rect = bbox_to_screen_rect(image_rect, bbox);
            if let Some(handle) = resize_handle_at(pointer, rect) {
                return Some(resize_cursor(handle));
            }
            if rect.contains(pointer) {
                return Some(egui::CursorIcon::Move);
            }
        }
        if interaction.edit_keypoints
            && keypoint_at(pointer, image_rect, annotation_id, annotations).is_some()
        {
            return Some(egui::CursorIcon::Move);
        }
    }
    if interaction.allow_selection
        && annotation_at_selectable(
            pointer,
            image_rect,
            annotations,
            selectable_annotations,
        )
        .is_some()
    {
        return Some(egui::CursorIcon::PointingHand);
    }
    (interaction.allow_create && image_rect.contains(pointer)).then_some(if bounding_box_tool {
        egui::CursorIcon::Crosshair
    } else {
        egui::CursorIcon::Cell
    })
}

fn resize_cursor(handle: ResizeHandle) -> egui::CursorIcon {
    match handle {
        ResizeHandle::TopLeft | ResizeHandle::BottomRight => egui::CursorIcon::ResizeNwSe,
        ResizeHandle::TopRight | ResizeHandle::BottomLeft => egui::CursorIcon::ResizeNeSw,
        ResizeHandle::Top | ResizeHandle::Bottom => egui::CursorIcon::ResizeVertical,
        ResizeHandle::Left | ResizeHandle::Right => egui::CursorIcon::ResizeHorizontal,
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

    if response.double_clicked() {
        state.fit_view();
        state.cancel_drag();
        return true;
    }

    let (space_down, primary_down, primary_pressed, pointer_delta) = ui.input(|input| {
        (
            input.key_down(Key::Space),
            input.pointer.primary_down(),
            input.pointer.primary_pressed(),
            input.pointer.delta(),
        )
    });
    if primary_pressed && response.is_pointer_button_down_on() {
        state.space_pan = space_down;
        state.primary_pan = state.pan_mode();
    }
    if state.space_pan || state.primary_pan {
        if primary_down {
            state.pan += pointer_delta;
            state.clamp_to_viewport(viewport, fitted_image);
        }
        state.cancel_drag();
        if !primary_down {
            state.space_pan = false;
            state.primary_pan = false;
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

#[allow(
    clippy::too_many_arguments,
    reason = "pointer handling receives the explicit canvas state needed for gesture priority"
)]
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
    selectable_annotations: Option<&std::collections::BTreeSet<AnnotationId>>,
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
            && let Some(annotation) = annotation_at_selectable(
                pointer,
                image_rect,
                annotations,
                selectable_annotations,
            )
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

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
    annotation_color: Color32,
    annotation_colors: &std::collections::BTreeMap<AnnotationId, Color32>,
) {
    let painter = ui.painter_at(viewport);
    painter.rect_filled(
        viewport,
        CornerRadius::same(VIEWPORT_CORNER_RADIUS),
        theme::INPUT_BG,
    );
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        for i in 1..4 {
            let x = image_rect.left() + image_rect.width() * i as f32 / 4.0;
            let y = image_rect.top() + image_rect.height() * i as f32 / 4.0;
            painter.line_segment(
                [pos2(x, image_rect.top()), pos2(x, image_rect.bottom())],
                Stroke::new(1.0, theme::CANVAS_GRID),
            );
            painter.line_segment(
                [pos2(image_rect.left(), y), pos2(image_rect.right(), y)],
                Stroke::new(1.0, theme::CANVAS_GRID),
            );
        }
    } else {
        painter.text(
            viewport.center(),
            egui::Align2::CENTER_CENTER,
            "Image preview unavailable",
            egui::FontId::new(theme::BODY_SIZE, egui::FontFamily::Proportional),
            theme::TEXT_MUTED,
        );
        ui.interact(
            viewport,
            ui.id().with("image_preview_unavailable"),
            Sense::hover(),
        )
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, "Image preview unavailable"));
    }

    for suggestion in prelabels {
        let color = theme::PRELABEL;
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
        let annotation_color = annotation_colors
            .get(&annotation.annotation_id)
            .copied()
            .unwrap_or(annotation_color);
        match &annotation.geometry {
            AnnotationGeometry::BoundingBox(bbox) => {
                let bbox = edit_preview
                    .filter(|(id, _)| *id == &annotation.annotation_id)
                    .map_or(*bbox, |(_, preview)| preview);
                if selected {
                    paint_selected_box(&painter, image_rect, bbox, editable, annotation_color);
                } else {
                    paint_existing_box(&painter, image_rect, bbox, annotation_color);
                }
            }
            AnnotationGeometry::Skeleton(skeleton) => {
                let color = annotation_color;
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
                                Stroke::new(3.0, theme::FOCUS_RING),
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
        Stroke::new(1.0, theme::BORDER_STRONG),
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

fn paint_existing_box(
    painter: &egui::Painter,
    image_rect: Rect,
    bbox: BoundingBox,
    color: Color32,
) {
    let rect = bbox_to_screen_rect(image_rect, bbox);
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
    color: Color32,
) {
    let rect = bbox_to_screen_rect(image_rect, bbox);
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
                Stroke::new(1.5, theme::SELECTION),
                StrokeKind::Inside,
            );
        }
    }
}

fn paint_draft_box(painter: &egui::Painter, image_rect: Rect, bbox: BoundingBox) {
    let rect = bbox_to_screen_rect(image_rect, bbox);
    let color = theme::DRAFT;
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

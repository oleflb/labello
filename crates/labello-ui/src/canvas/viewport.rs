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
